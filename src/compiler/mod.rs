use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};
use std::path::{MAIN_SEPARATOR, Path};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cranelift::codegen;
use cranelift::codegen::isa::TargetIsa;
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, ModuleReloc};
use cranelift_object::{ObjectBuilder, ObjectModule};
use target_lexicon::BinaryFormat;

use crate::ast::{Access, Annotation, EnumVariant, Expr, Param, Span, Spanned, TypeExpr, TypeParam};
use crate::diagnostics::{Diagnostic, SourceMap};
use crate::loader::{Program, Scope, is_hook_trait, is_literal};
use crate::runtime;

mod cache;
mod comp;
mod expand;
mod lower;
mod resolve;
mod role;
mod traits;
mod typ;

use expand::expand;
use lower::Translator;
use lower::value::{define_data, define_once, define_ptr_data};
pub(crate) use resolve::*;
pub(crate) use traits::*;
pub(crate) use typ::*;

struct FnItem<'a> {
	key: String,
	scope: &'a Scope,
	params: Vec<Param>,
	params_tuple: bool,
	ret: Option<Spanned<TypeExpr>>,
	body: &'a [Spanned<Expr>],
}

type EnumItem<'a> = (&'a str, Option<&'a Spanned<TypeExpr>>, &'a [EnumVariant]);

// A claim's fill types.
#[derive(Default, Clone, Copy)]
struct Fills<'a, 'b> {
	decls: &'b [TraitFn<'a>],
	generic: bool,
}

#[derive(Clone)]
pub(crate) struct FnSig {
	pub id: FuncId,
	pub params: Vec<FnParam>,
	pub access: Vec<Access>,
	pub ret: Typ,
	pub foreign: bool,
	pub unsafe_call: bool,
	pub pure: bool,
}

impl FnSig {
	// Params as a fn value sees them, the access mods folded back in.
	pub(crate) fn value_params(&self) -> Vec<FnParam> {
		let fold = |(p, &a): (&FnParam, &Access)| FnParam {
			typ: access_wrap(a, p.typ.clone()),
			..p.clone()
		};
		self.params.iter().zip(&self.access).map(fold).collect()
	}
}

// Enforce default param rules.
fn check_param_defaults(params: &[Param]) -> Result<(), Diagnostic> {
	if let Some(p) = params.iter().find(|p| p.access == Access::Mut && p.default.is_some()) {
		let msg = format!("`{}` is `mut` so it can't have a default value", p.name);
		return Err(Diagnostic::new(msg, p.span.into_range())
			.with_label("`mut` lends the caller's binding, there is none when the arg is omitted"));
	}
	let tail = params.iter().skip_while(|p| p.default.is_none());
	if let Some(p) = tail.skip(1).find(|p| p.default.is_none()) {
		let msg = format!("`{}` needs a default value", p.name);
		return Err(Diagnostic::new(msg, p.span.into_range()).with_label("defaults must be trailing"));
	}
	Ok(())
}

// Check that varargs don't violate our rules.
// TODO: I hope to lax these in the future but don't want to spin my wheels on greedy type checking and stuff right now
fn check_varargs(name: &str, params: &[Param]) -> Result<(), Diagnostic> {
	let mut varargs = params.iter().filter(|p| matches!(p.typ, TypeExpr::Variadic(_)));
	let Some(p) = varargs.nth(1) else { return Ok(()) };
	let msg = format!("`{}` has more than one vararg", display_name(name));
	Err(Diagnostic::new(msg, p.span.into_range()).with_label("second vararg"))
}

// Whether fn specifies a Result.
pub(crate) fn fallible(typ: &Typ) -> bool {
	matches!(typ, Typ::Result(ok, _) if ok.is_unit())
}

// Check that every param and return are C friendly.
pub(crate) fn check_c_sig(name: &str, params: &[FnParam], ret: &Typ, span: Span) -> Result<(), Diagnostic> {
	match (params.iter().map(|p| &p.typ))
		.chain((!ret.is_unit()).then_some(ret))
		.find(|t| !t.is_c_repr())
	{
		Some(t) => Err(
			Diagnostic::new(format!("`{name}` can't cross the C ABI"), span.into_range())
				.with_label(format!("`{t}` has no C representation")),
		),
		None => Ok(()),
	}
}

// Handle annotated types.
pub(crate) fn check_ann_typ(names: &[String], typ: &Typ, span: Span) -> Result<(), Diagnostic> {
	let c = |n: &String| n == role::C;
	match (names, typ) {
		([n], Typ::Fn(params, ret)) if c(n) => check_c_sig("@c fn", params, ret, span),
		([n], Typ::Closure(..)) if c(n) => Err(Diagnostic::new("`@c` fns can't capture", span.into_range())
			.with_label("C has nowhere to keep an environment")),
		_ => Err(
			Diagnostic::new(format!("`{}{typ}` isn't a type", marks(names)), span.into_range())
				.with_label("only `@c` on a fn, so far"),
		),
	}
}

// A generic free function, monomorphized per callsite.
#[derive(Clone)]
pub(crate) struct GenericFnDef {
	pub params: Vec<Param>,
	pub params_tuple: bool,
	pub ret: Option<Spanned<TypeExpr>>,
	pub body: Vec<Spanned<Expr>>,
	pub type_params: Vec<TypeParam>,
	pub captures: Vec<(String, Typ, bool)>,
	pub self_name: Option<String>,
	pub module: String,
}

// A monomorphized instance whose sig is declared but body not yet compiled.
pub(crate) type Pending = (String, GenericFnDef, HashMap<String, Typ>);

// A resolved fn.
#[derive(Default)]
struct FnDef<'a> {
	params: &'a [(String, Typ, Access)],
	params_tuple: bool,
	ret: Option<(Typ, Span)>,
	body: &'a [Spanned<Expr>],
	self_type: Option<&'a str>,
	is_main: bool,
	is_test: bool,
	captures: &'a [(String, Typ, bool)],
	self_fn: Option<(&'a str, &'a FnSig)>,
	foreign: bool,
	pure: bool,
}

// A generic struct definition.
#[derive(Clone)]
pub(crate) struct GenericStructDef {
	pub type_params: Vec<TypeParam>,
	pub fields: Vec<Param>,
}

// A generic enum definition.
#[derive(Clone)]
pub(crate) struct GenericEnumDef {
	pub type_params: Vec<TypeParam>,
	pub variants: Vec<EnumVariant>,
}

// Generic type definitions.
#[derive(Default)]
pub(crate) struct Generics {
	pub structs: HashMap<String, GenericStructDef>,
	pub enums: HashMap<String, GenericEnumDef>,
	// enum instances keyed by display name (`Opt[int]`)
	pub instances: RefCell<HashMap<String, Vec<VariantInfo>>>,
	// struct instances' concrete type args keyed by display name (`Box[int]`)
	pub instance_args: RefCell<HashMap<String, Vec<Typ>>>,
}

// Does a type ref mention the named type?
fn mentions(te: &TypeExpr, name: &str) -> bool {
	match te {
		TypeExpr::Name(n) => n == name,
		TypeExpr::Array(e) | TypeExpr::FixedArray(e, _) | TypeExpr::Option(e) | TypeExpr::Variadic(e) => {
			mentions(e, name)
		}
		TypeExpr::Result(e, err) => mentions(e, name) || err.as_deref().is_some_and(|e| mentions(e, name)),
		TypeExpr::Sum(es) | TypeExpr::Generic(_, es) => es.iter().any(|e| mentions(e, name)),
		TypeExpr::Tuple(fs) => fs.iter().any(|(_, t)| mentions(t, name)),
		TypeExpr::Fn(ps, r) => ps.iter().any(|(_, _, p)| mentions(p, name)) || mentions(r, name),
		TypeExpr::Annotated(_, t) => mentions(t, name),
		TypeExpr::TupleStruct(_, fs) => fs.iter().any(|(_, t)| mentions(t, name)),
		TypeExpr::AnonStruct(fs) => fs.iter().any(|f| mentions(&f.typ, name)),
		TypeExpr::Map(k, v) => mentions(k, name) || mentions(v, name),
		TypeExpr::Ref(e) => mentions(e, name),
		TypeExpr::AtomSum(_) | TypeExpr::Unquote(_) => false,
	}
}

// Qualify each type param's trait bound through its defining scope.
fn qualify_bounds(scope: &Scope, params: &mut [TypeParam]) {
	for bound in params.iter_mut().filter_map(|p| p.bound.as_mut()) {
		*bound = scope.qualify_trait(bound);
	}
}

// Qualify a value annotation's name through its defining scope.
fn qualify_anns(scope: &Scope, anns: &[Annotation]) -> Vec<Annotation> {
	let mut anns = anns.to_vec();
	for a in &mut anns {
		match &mut a.0 {
			Expr::StructLit { name, .. } | Expr::Ident(name) | Expr::Call { name, .. } if name != "unsafe" => {
				*name = scope.qualify_name(name)
			}
			_ => {}
		}
	}
	anns
}

// The literal fields of a `@name` annotation.
fn ann<'a>(a: &'a Annotation, name: &str) -> Option<&'a [(Option<String>, Spanned<Expr>)]> {
	match &a.0 {
		Expr::StructLit { name: n, fields, .. } if n == name => Some(fields),
		Expr::Ident(n) if n == name => Some(&[]),
		_ => None,
	}
}

// Annotation names, qualified through the scope that wrote them.
pub(crate) fn ann_names(scope: &Scope, anns: &[Annotation]) -> Vec<String> {
	let name = |a: &Annotation| match &a.0 {
		Expr::StructLit { name, .. } | Expr::Ident(name) | Expr::Call { name, .. } => Some(name.clone()),
		_ => None,
	};
	qualify_anns(scope, anns).iter().filter_map(name).collect()
}

pub(crate) fn is_c_struct(anns: &HashMap<String, Vec<Annotation>>, name: &str) -> bool {
	let mut anns = anns.get(name).into_iter().flatten();
	anns.any(|a| ann(a, role::C).is_some())
}

// Check that every struct marked `@c` has a C layout.
fn check_c_structs(
	anns: &HashMap<String, Vec<Annotation>>,
	structs: &HashMap<String, Vec<FieldDef>>,
) -> Result<(), Diagnostic> {
	let is_c = |n: &str| is_c_struct(anns, n);
	for (name, fields) in structs.iter().filter(|(n, _)| is_c(n)) {
		let Some(bad) = fields.iter().find(|f| f.typ.c_size_align(&is_c).is_none()) else {
			continue;
		};
		let span = anns[name].iter().find(|a| ann(a, role::C).is_some()).unwrap().1;
		let msg = format!("`{}.{}` has no C representation", display_name(name), bad.name);
		return Err(Diagnostic::new(msg, span.into_range())
			.with_label(format!("`{}` is not a type with a known C layout", bad.typ)));
	}
	Ok(())
}

// Check that annotations name a struct value and pass literal args.
fn check_annotations<'p>(
	anns: &HashMap<String, Vec<Annotation>>,
	structs: &HashMap<String, Vec<FieldDef>>,
	generics: &Generics,
	consts: &HashMap<String, Spanned<Expr>>,
	scope_of: impl Fn(&str) -> &'p Scope,
) -> Result<(), Diagnostic> {
	let mut generic_field_anns = Vec::new();
	for (name, def) in &generics.structs {
		let scope = scope_of(name);
		generic_field_anns.extend(def.fields.iter().flat_map(|p| qualify_anns(scope, &p.annotations)));
	}
	let field_anns = structs.values().flatten().flat_map(|f| f.annotations.iter());
	for a in anns.values().flatten().chain(field_anns).chain(&generic_field_anns) {
		check_annotation(a, structs, generics, consts)?;
	}
	Ok(())
}

fn check_annotation(
	a: &Annotation,
	structs: &HashMap<String, Vec<FieldDef>>,
	generics: &Generics,
	consts: &HashMap<String, Spanned<Expr>>,
) -> Result<(), Diagnostic> {
	let err = |msg: String, label: String| Err(Diagnostic::new(msg, a.1.into_range()).with_label(label));
	match &a.0 {
		Expr::StructLit { name, fields, .. } => check_struct_lit(name, fields, a.1, structs, generics),
		Expr::Ident(name) if name == "unsafe" => Ok(()),
		Expr::Ident(name) => match consts.get(name) {
			Some((Expr::StructLit { name, fields, .. }, _)) => check_struct_lit(name, fields, a.1, structs, generics),
			Some((Expr::Tuple(fields), _)) if fields.is_empty() => Ok(()),
			Some(_) => err(
				format!("`{name}` is not an annotation value"),
				"a bare annotation names a unit or struct const".into(),
			),
			// a bare struct type denotes its zero value
			None if structs.contains_key(name) => Ok(()),
			None if generics.structs.contains_key(name) => {
				let msg = "a generic struct can't be an annotation";
				Err(Diagnostic::new(msg, a.1.into_range()).with_label("pick a concrete struct"))
			}
			None => err(
				format!("`{name}` is not a constant"),
				"a bare annotation names a unit or struct const".into(),
			),
		},
		Expr::Call { .. } => err(
			"annotation fns aren't supported here yet".into(),
			"only main-file items take fn annotations".into(),
		),
		_ => Ok(()),
	}
}

fn check_struct_lit(
	name: &str,
	fields: &[(Option<String>, Spanned<Expr>)],
	span: Span,
	structs: &HashMap<String, Vec<FieldDef>>,
	generics: &Generics,
) -> Result<(), Diagnostic> {
	if name.is_empty() {
		return fields.iter().try_for_each(|(_, v)| check_lit(&v.0, None, v.1));
	}
	let Some(field_defs) = structs.get(name) else {
		if generics.structs.contains_key(name) {
			let msg = "a generic struct can't be an annotation";
			return Err(Diagnostic::new(msg, span.into_range()).with_label("pick a concrete struct"));
		}
		let msg = format!("`{name}` is not a struct");
		return Err(Diagnostic::new(msg, span.into_range()).with_label("an annotation is a struct value"));
	};
	let mut prefix = 0;
	for (i, (key, value)) in fields.iter().enumerate() {
		let err = |msg: String, label| Err(Diagnostic::new(msg, value.1.into_range()).with_label(label));
		let idx = match key {
			None if i != prefix => {
				return err(
					"positional fields go before named fields".into(),
					"positional field after a named one",
				);
			}
			None if i >= field_defs.len() => {
				let n = fields.iter().filter(|(k, _)| k.is_none()).count();
				let msg = format!("`{name}` has {} fields but {n} values were provided", field_defs.len());
				return err(msg, "wrong number of fields");
			}
			None => {
				prefix += 1;
				i
			}
			Some(key) => match field_defs.iter().position(|f| &f.name == key) {
				None => return err(format!("`{name}` has no field `{key}`"), "no such field"),
				Some(idx) if idx < prefix => return err(format!("`{key}` was already set positionally"), "set twice"),
				Some(idx) => idx,
			},
		};
		check_lit(&value.0, Some(&field_defs[idx].typ), value.1)?;
	}
	Ok(())
}

fn check_lit(e: &Expr, typ: Option<&Typ>, span: Span) -> Result<(), Diagnostic> {
	if !is_literal(e) {
		let msg = "annotation arguments must be literal values";
		return Err(Diagnostic::new(msg, span.into_range()).with_label("not a literal"));
	}
	match typ {
		Some(typ) if !lit_matches(e, typ) => {
			Err(Diagnostic::new(format!("expected {typ}"), span.into_range()).with_label("type mismatch"))
		}
		_ => Ok(()),
	}
}

// Check whether a literal expression agrees with a field's type.
fn lit_matches(e: &Expr, typ: &Typ) -> bool {
	match e {
		Expr::Negative(inner) => lit_matches(&inner.0, typ),
		Expr::Bool(_) => matches!(typ, Typ::Bool),
		Expr::Int(_) => matches!(
			typ,
			Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize | Typ::Float(_)
		),
		Expr::Float(_) => matches!(typ, Typ::Float(_)),
		Expr::String(_) => matches!(typ, Typ::Str),
		Expr::Atom(_) => typ.is_enumish() || matches!(typ, Typ::Atom),
		_ => false,
	}
}

// No placeholder struct may appear outside a `&T`.
fn ref_guarded(typ: &Typ, placeholders: &HashSet<String>) -> bool {
	match typ {
		Typ::Ref(_) => true,
		Typ::Struct(n, fs) => !placeholders.contains(n) && fs.iter().all(|f| ref_guarded(&f.typ, placeholders)),
		Typ::Option(i) | Typ::Array(i) | Typ::FixedArray(i, _) => ref_guarded(i, placeholders),
		Typ::Result(ok, err) => ref_guarded(ok, placeholders) && ref_guarded(err, placeholders),
		Typ::Map(k, v) => ref_guarded(k, placeholders) && ref_guarded(v, placeholders),
		Typ::Tuple(fs) | Typ::TupleStruct(_, fs) => fs.iter().all(|(_, t)| ref_guarded(t, placeholders)),
		Typ::Sum(_, vs) => vs.iter().all(|v| v.payload.iter().all(|t| ref_guarded(t, placeholders))),
		_ => true,
	}
}

// Rewrite `Self` type refs to the owning type.
fn replace_self(te: &TypeExpr, self_ty: &TypeExpr) -> TypeExpr {
	match te {
		TypeExpr::Name(n) if n == "Self" => self_ty.clone(),
		TypeExpr::Array(e) => TypeExpr::Array(Box::new(replace_self(e, self_ty))),
		TypeExpr::FixedArray(e, n) => TypeExpr::FixedArray(Box::new(replace_self(e, self_ty)), n.clone()),
		TypeExpr::Option(e) => TypeExpr::Option(Box::new(replace_self(e, self_ty))),
		TypeExpr::Variadic(e) => TypeExpr::Variadic(Box::new(replace_self(e, self_ty))),
		TypeExpr::Result(e, err) => TypeExpr::Result(Box::new(replace_self(e, self_ty)), err.clone()),
		TypeExpr::Tuple(fs) => TypeExpr::Tuple(fs.iter().map(|(n, t)| (n.clone(), replace_self(t, self_ty))).collect()),
		TypeExpr::Annotated(a, t) => TypeExpr::Annotated(a.clone(), Box::new(replace_self(t, self_ty))),
		TypeExpr::Fn(ps, r) => TypeExpr::Fn(
			ps.iter().map(|(n, a, p)| (n.clone(), *a, replace_self(p, self_ty))).collect(),
			Box::new(replace_self(r, self_ty)),
		),
		TypeExpr::Map(k, v) => TypeExpr::Map(Box::new(replace_self(k, self_ty)), Box::new(replace_self(v, self_ty))),
		TypeExpr::Generic(name, args) => {
			TypeExpr::Generic(name.clone(), args.iter().map(|a| replace_self(a, self_ty)).collect())
		}
		other => other.clone(),
	}
}

#[derive(Clone)]
pub(crate) struct Local {
	pub var: Variable,
	pub typ: Typ,
	pub mutable: bool,
	pub boxed: bool,
	pub stat: bool,
}

impl Local {
	pub fn plain(var: Variable, typ: Typ, mutable: bool) -> Self {
		Local {
			var,
			typ,
			mutable,
			boxed: false,
			stat: false,
		}
	}
}

// `continue` jumps to `top`, `break` jumps to `exit`
pub(crate) struct LoopFrame {
	pub top: Block,
	pub exit: Option<Block>,
	pub depth: usize,
}

pub struct Compiler<M: Module = JITModule> {
	builder_ctx: FunctionBuilderContext,
	ctx: codegen::Context,
	module: M,
	string_idx: usize,
	atoms: HashSet<String>,
	generics: HashMap<String, GenericFnDef>,
	mono: HashMap<String, FnSig>,
	pending: Vec<Pending>,
	wanted: Vec<FuncId>,
	printers: Vec<(String, Typ, bool, runtime::Sink)>,
	trait_impls: HashSet<(String, String)>,
	generic_claims: HashSet<(String, String)>,
	core_traits: HashSet<String>,
	descs: HashMap<String, DataId>,
	defined: HashSet<FuncId>,
	publics: HashSet<String>,
	privates: HashMap<String, HashSet<String>>,
	reexports: HashMap<String, String>,
	consts: HashMap<String, Spanned<Expr>>,
	statics: HashMap<String, (String, Typ)>,
	static_inits: Vec<(String, Spanned<Expr>)>,
	annotations: HashMap<String, Vec<Annotation>>,
	module_scopes: HashMap<String, Scope>,
	map: SourceMap,
	hoisted: HashMap<String, FnSig>,
	lib: bool,
	aot: bool,
	pub(crate) include_tests: bool,
	pub(crate) tests: Vec<(String, String, bool)>,
	pub(crate) roots: Vec<String>,
	link_libs: Vec<String>,
	exports: HashMap<String, String>,
	cache: Option<cache::Store>,
	pub timings: Vec<(&'static str, Duration)>,
}

// Get a static's type from its literal.
fn static_typ(e: &Expr, types: &TypeCtx, span: Span) -> Result<Typ, Diagnostic> {
	match e {
		Expr::Negative(v) => static_typ(&v.0, types, span),
		Expr::Int(_) => Ok(Typ::Int(64)),
		Expr::Float(_) => Ok(Typ::Float(64)),
		Expr::Bool(_) => Ok(Typ::Bool),
		Expr::String(_) => Ok(Typ::Str),
		Expr::StructLit { name, .. } if !name.is_empty() => types.resolve(&TypeExpr::Name(name.clone()), span),
		_ => Err(
			Diagnostic::new("cannot tell what type this static is", span.into_range())
				.with_label("annotate it, or initialize it with a literal"),
		),
	}
}

fn comptime_only(e: &Expr) -> bool {
	let ast = |t: &TypeExpr| matches!(t, TypeExpr::Name(n) if n == "Ast");
	match e {
		Expr::Pub(inner) | Expr::Annotated(_, inner) => comptime_only(&inner.0),
		Expr::Fn { params, ret, .. } => params.iter().any(|p| ast(&p.typ)) || ret.as_ref().is_some_and(|r| ast(&r.0)),
		Expr::Bind {
			typ: Some((TypeExpr::Fn(ps, r), _)),
			..
		} => ps.iter().any(|(_, _, t)| ast(t)) || ast(r),
		_ => false,
	}
}

/// Whether `name` resolves in the running process (libc/libm etc).
fn process_symbol_exists(name: &str) -> bool {
	#[cfg(unix)]
	{
		let Ok(cname) = std::ffi::CString::new(name) else {
			return false;
		};
		!unsafe { libc::dlsym(libc::RTLD_DEFAULT, cname.as_ptr()) }.is_null()
	}
	#[cfg(not(unix))]
	{
		let _ = name;
		true
	}
}

/// dlopen library
/// `name` can be shared lib name or a file path.
fn load_library(name: &str) -> bool {
	#[cfg(unix)]
	{
		let file = match Path::new(name).is_absolute() {
			true => name.to_string(),
			false => format!("{}{name}{}", DLL_PREFIX, DLL_SUFFIX),
		};
		let Ok(cname) = std::ffi::CString::new(file) else {
			return false;
		};
		!unsafe { libc::dlopen(cname.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) }.is_null()
	}
	#[cfg(not(unix))]
	{
		let _ = name;
		true
	}
}

// Enable position-independent code for AOT/JIT compilation.
fn isa(pic: bool) -> Arc<dyn TargetIsa> {
	let mut flag_builder = settings::builder();
	flag_builder.set("use_colocated_libcalls", "false").unwrap();
	flag_builder.set("is_pic", if pic { "true" } else { "false" }).unwrap();
	cranelift_native::builder()
		.unwrap_or_else(|e| panic!("unsupported host: {e}"))
		.finish(settings::Flags::new(flag_builder))
		.unwrap()
}

impl Default for Compiler<JITModule> {
	fn default() -> Self {
		let mut builder = JITBuilder::with_isa(isa(false), cranelift_module::default_libcall_names());
		builder.symbol(expand::RT_QUOTE, expand::rt_quote as *const u8);
		builder.symbol(expand::RT_AST_LIT, expand::rt_ast_lit as *const u8);
		builder.symbol(expand::RT_AST_METHOD, expand::rt_ast_method as *const u8);
		builder.symbol(expand::RT_QUOTE_MATCH, expand::rt_quote_match as *const u8);
		builder.symbol(comp::RT_COMP_YIELD, comp::rt_comp_yield as *const u8);
		builder.symbol(comp::RT_COMP_STRUCT, comp::rt_comp_struct as *const u8);

		Self::new(JITModule::new(builder))
	}
}

impl Compiler<ObjectModule> {
	pub fn object(name: &str, lib: bool) -> Self {
		let builder = ObjectBuilder::new(isa(true), name, cranelift_module::default_libcall_names()).unwrap();
		let mut compiler = Self::new(ObjectModule::new(builder));
		compiler.lib = lib;
		compiler.aot = true;
		compiler
	}

	pub fn compile_object(mut self, program: &Program, timings: bool) -> Result<(Vec<u8>, Vec<String>), Diagnostic> {
		let (t, n) = (Instant::now(), self.timings.len());
		let entry = self.build(program)?;
		self.time_codegen(t, n);
		self.emit_entry(entry);
		if timings {
			self.report_timings();
		}
		Ok((self.module.finish().emit().expect("emit object"), self.link_libs))
	}

	// The object's entrypoint.
	fn emit_entry(&mut self, entry: FuncId) {
		let sig = self.module.make_signature();
		let mut calls = vec![entry];
		if !self.lib {
			calls.push(self.module.declare_function("oi_epilogue", Linkage::Import, &sig).unwrap());
			self.ctx.func.signature.returns.push(AbiParam::new(types::I32));
		}
		let mut b = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_ctx);
		let block = b.create_block();
		b.switch_to_block(block);
		b.seal_block(block);
		for id in calls {
			let f = self.module.declare_func_in_func(id, b.func);
			b.ins().call(f, &[]);
		}
		let ret = if self.lib {
			vec![]
		} else {
			vec![b.ins().iconst(types::I32, 0)]
		};
		b.ins().return_(&ret);
		b.finalize();
		let (name, linkage) = match self.lib {
			true => ("oi_init", Linkage::Local),
			false => ("main", Linkage::Export),
		};
		let id = self.module.declare_function(name, linkage, &self.ctx.func.signature).unwrap();
		self.module.define_function(id, &mut self.ctx).unwrap();
		self.module.clear_context(&mut self.ctx);
		if self.lib {
			// seed statics because libs don't have a `main` fn entrypoint
			let (seg, sec, flags) = match self.module.isa().triple().binary_format {
				BinaryFormat::Macho => ("__DATA", "__mod_init_func", 0x9),
				BinaryFormat::Coff => ("", ".CRT$XCU", 0),
				_ => ("", ".init_array", 0),
			};
			let mut desc = DataDescription::new();
			desc.set_align(8);
			desc.set_segment_section(seg, sec, flags);
			desc.define(vec![0; 8].into_boxed_slice());
			let f = self.module.declare_func_in_data(id, &mut desc);
			desc.write_function_addr(0, f);
			let arr = self.module.declare_data("oi_init_array", Linkage::Local, true, false).unwrap();
			self.module.define_data(arr, &desc).unwrap();
		}
	}
}

impl<M: Module> Compiler<M> {
	fn new(module: M) -> Self {
		Self {
			builder_ctx: FunctionBuilderContext::new(),
			ctx: module.make_context(),
			module,
			string_idx: 0,
			atoms: HashSet::new(),
			generics: HashMap::new(),
			mono: HashMap::new(),
			pending: Vec::new(),
			wanted: Vec::new(),
			printers: Vec::new(),
			trait_impls: HashSet::new(),
			generic_claims: HashSet::new(),
			core_traits: HashSet::new(),
			descs: HashMap::new(),
			defined: HashSet::new(),
			publics: HashSet::new(),
			privates: HashMap::new(),
			reexports: HashMap::new(),
			consts: HashMap::new(),
			statics: HashMap::new(),
			static_inits: vec![],
			annotations: HashMap::new(),
			module_scopes: HashMap::new(),
			map: SourceMap::default(),
			hoisted: HashMap::new(),
			lib: false,
			aot: false,
			include_tests: false,
			tests: Vec::new(),
			roots: Vec::new(),
			link_libs: Vec::new(),
			exports: HashMap::new(),
			cache: None,
			timings: Vec::new(),
		}
	}

	// Time codegen phase.
	fn time_codegen(&mut self, t: Instant, n: usize) {
		let inner: Duration = self.timings[n..].iter().map(|(_, d)| *d).sum();
		self.timings.push(("codegen", t.elapsed().saturating_sub(inner)));
	}

	/// Print each phase's time to stderr.
	pub fn report_timings(&self) {
		for (phase, dur) in &self.timings {
			eprintln!("{phase}  {dur:.1?}");
		}
	}

	// Register a type's fills as `Type.method` fns.
	fn register_fills<'a>(
		&mut self,
		typ: &str,
		type_params: &[TypeParam],
		fills: &'a [Spanned<Expr>],
		scope: &'a Scope,
		others: &mut Vec<FnItem<'a>>,
		claim: Fills,
	) -> Result<(), Diagnostic> {
		for m in fills {
			let (anns, m) = match &m.0 {
				Expr::Annotated(anns, inner) => (&anns[..], &**inner),
				_ => (&[][..], m),
			};
			let (public, m) = match &m.0 {
				Expr::Pub(inner) => (true, &**inner),
				_ => (false, m),
			};
			let Expr::Fn {
				name,
				type_params: mtp,
				params,
				params_tuple,
				ret,
				body,
			} = &m.0
			else {
				continue;
			};
			// key claims by type+name to handle generics
			let key = match claim.generic {
				true => format!("{typ}.{name}#{}", others.len()),
				false => format!("{typ}.{name}"),
			};
			self.annotations
				.entry(key.clone())
				.or_default()
				.extend(qualify_anns(scope, anns));
			// visibility
			if !public && claim.decls.is_empty() && typ.contains("::") {
				self.privates.entry(typ.to_string()).or_default().insert(name.clone());
			}
			if others.iter().any(|f| f.key == key) || self.generics.contains_key(&key) {
				let msg = format!("duplicate fill `{key}`");
				return Err(Diagnostic::new(msg, m.1.into_range()).with_label("one fill per name"));
			}
			if type_params.is_empty() && mtp.is_empty() {
				let (params, params_tuple, ret) = match claim.decls.iter().find(|(n, ..)| *n == name) {
					Some(decl) => fill_from_decl(params, *params_tuple, ret, *decl, m.1)?,
					None if params.is_empty() && !params_tuple => {
						let msg = format!("no trait method `{name}` supplies a signature");
						return Err(Diagnostic::new(msg, m.1.into_range()).with_label("write the `fn` header out"));
					}
					None => (params.clone(), *params_tuple, ret.clone()),
				};
				others.push(FnItem {
					key,
					scope,
					params,
					params_tuple,
					ret,
					body,
				});
				continue;
			}
			let name = |p: &TypeParam| Box::new(TypeExpr::Name(p.name.clone()));
			let self_ty = match (typ, type_params) {
				("array", [t]) => TypeExpr::Array(name(t)),
				("map", [k, v]) => TypeExpr::Map(name(k), name(v)),
				_ if type_params.is_empty() => TypeExpr::Name(typ.to_string()),
				_ => {
					let args = type_params.iter().map(|p| TypeExpr::Name(p.name.clone())).collect();
					TypeExpr::Generic(typ.to_string(), args)
				}
			};
			let params = params
				.iter()
				.map(|p| Param {
					typ: replace_self(&p.typ, &self_ty),
					..p.clone()
				})
				.collect();
			let ret = ret.as_ref().map(|(te, span)| (replace_self(te, &self_ty), *span));
			let mut all_params = type_params.to_vec();
			all_params.extend(mtp.clone());
			qualify_bounds(scope, &mut all_params);
			self.generics.insert(
				key,
				GenericFnDef {
					params,
					params_tuple: *params_tuple,
					ret,
					body: body.clone(),
					type_params: all_params,
					captures: vec![],
					self_name: None,
					module: scope.module.clone(),
				},
			);
		}
		Ok(())
	}

	fn note_privates(&mut self, name: &str, fields: &[Param]) {
		if name.contains("::") {
			let hidden = fields.iter().filter(|f| !f.public).map(|f| f.name.clone());
			self.privates.entry(name.to_string()).or_default().extend(hidden);
		}
	}

	// An 8-byte vtable cell holding a trait const's value.
	fn const_cell(&mut self, sym: &str, lit: &Expr, want: &Typ) -> DataId {
		let m = &mut self.module;
		let bytes = match lit {
			Expr::String(s) => {
				let b = define_data(m, &format!("{sym}_bytes"), [s.as_bytes(), &[0]].concat());
				let hdr = define_ptr_data(m, &format!("{sym}_hdr"), b, &(s.len() as i64).to_le_bytes());
				return define_ptr_data(m, sym, hdr, &[]);
			}
			Expr::Float(v) if matches!(want, Typ::Float(32)) => (f32::to_bits(*v as f32) as i64).to_le_bytes(),
			Expr::Float(v) => v.to_bits().to_le_bytes(),
			Expr::Bool(b) => (*b as i64).to_le_bytes(),
			Expr::Int(n) => n.to_le_bytes(),
			_ => unreachable!("trait consts are literals"),
		};
		define_data(m, sym, bytes.to_vec())
	}

	fn build(&mut self, program: &Program) -> Result<FuncId, Diagnostic> {
		let mut struct_items: Vec<(&str, &[Param])> = vec![];
		let mut generics = Generics::default();
		let mut enum_items: Vec<EnumItem> = vec![];
		let mut alias_items: Vec<(&str, TypeExpr)> = vec![];
		let mut soft_aliases: Vec<(String, TypeExpr)> = vec![];
		let mut main_body: Option<&[Spanned<Expr>]> = None;
		let mut main_ret: Option<&Spanned<TypeExpr>> = None;
		let mut others: Vec<FnItem> = vec![];
		let mut loose_refs: Vec<&Spanned<Expr>> = vec![];
		let mut trait_bodies: Vec<TraitBody> = vec![];
		let mut foreign_items: Vec<(String, TypeExpr, Span, &Scope)> = vec![];
		let mut static_items = vec![];

		// stage 0 cleanup
		self.generics.clear();
		self.trait_impls.clear();
		self.generic_claims.clear();
		self.static_inits.clear();
		self.tests.clear();
		self.wanted.clear();
		self.pending.clear();
		self.module.clear_context(&mut self.ctx);
		self.builder_ctx = FunctionBuilderContext::new();

		self.cache = cache::Store::open(&program.roots[0]);
		self.publics = program.publics.clone();
		self.reexports = program.reexports.clone();
		self.consts = program.consts.clone();
		self.map = program.map.clone();
		let scopes: HashMap<&str, &Scope> = program.modules.iter().map(|m| (m.name.as_str(), &m.scope)).collect();
		self.module_scopes = scopes.iter().map(|(&k, &v)| (k.to_string(), v.clone())).collect();
		let scope_of = |key: &str| scopes[key.split_once("::").map_or("main", |(m, _)| m)];
		self.annotations = program
			.annotations
			.iter()
			.map(|(k, anns)| (k.clone(), qualify_anns(scope_of(k), anns)))
			.collect();

		// expand user macros to AST
		let t = Instant::now();
		let (mut expanded, mut stage0) = expand(program)?;
		self.timings.push(("expand", t.elapsed()));
		for m in &program.modules {
			for item in expanded.get_mut(&m.name).expect("every module was seeded") {
				loop {
					let (anns, inner) = match &item.0 {
						Expr::Annotated(anns, inner) => (&anns[..], inner),
						Expr::Pub(inner) => (&[][..], inner),
						_ => break,
					};
					if let Some(name) = inner.0.def_name()
						&& !anns.is_empty()
					{
						let anns = qualify_anns(&m.scope, anns);
						self.annotations.entry(name.into()).or_default().extend(anns);
					}
					*item = (**inner).clone();
				}
			}
		}
		// fold `comp` expressions to literals
		let t = Instant::now();
		comp::eval(
			&mut expanded,
			&mut self.annotations,
			&mut self.consts,
			program,
			&mut stage0,
		)?;
		self.timings.push(("comp", t.elapsed()));
		if self.aot {
			expanded.values_mut().for_each(|items| items.retain(|(e, _)| !comptime_only(e)));
		}
		let items = || {
			program
				.modules
				.iter()
				.flat_map(|m| expanded[&m.name].iter().map(move |i| (&m.scope, i)))
		};

		let has_main = items().any(|(_, i)| matches!(&i.0, Expr::Fn { name, .. } if name == "main"));

		let mut traits: HashMap<&str, TraitItem> = HashMap::new();
		for (scope, (e, span)) in items() {
			let Expr::TraitDef {
				name,
				type_params,
				supers,
				fields,
				methods,
			} = e
			else {
				continue;
			};
			let supers = supers.iter().map(|s| scope.qualify_trait(s)).collect();
			let item = (supers, type_params.as_slice(), fields.as_slice(), methods.as_slice());
			if traits.insert(name.as_str(), item).is_some() {
				let msg = format!("duplicate trait `{name}`");
				return Err(Diagnostic::new(msg, span.into_range()).with_label("already defined"));
			}
			if scope.module == "core" {
				self.core_traits.insert(name.clone());
			}
		}
		for (scope, item) in items() {
			match &item.0 {
				Expr::StructDef {
					name,
					type_params,
					fields,
					fills,
				} if !type_params.is_empty() => {
					generics.structs.insert(
						name.clone(),
						GenericStructDef {
							type_params: type_params.clone(),
							fields: fields.clone(),
						},
					);
					self.note_privates(name, fields);
					self.register_fills(name, type_params, fills, scope, &mut others, Fills::default())?;
				}
				Expr::StructDef {
					name, fields, fills, ..
				} => {
					struct_items.push((name.as_str(), fields.as_slice()));
					self.note_privates(name, fields);
					self.register_fills(name, &[], fills, scope, &mut others, Fills::default())?;
				}
				Expr::EnumDef {
					name,
					type_params,
					variants,
					fills,
					..
				} if !type_params.is_empty() => {
					generics.enums.insert(
						name.clone(),
						GenericEnumDef {
							type_params: type_params.clone(),
							variants: variants.clone(),
						},
					);
					self.register_fills(name, type_params, fills, scope, &mut others, Fills::default())?;
				}
				Expr::EnumDef {
					name,
					backing,
					variants,
					fills,
					..
				} => {
					enum_items.push((name.as_str(), backing.as_ref(), variants.as_slice()));
					self.register_fills(name, &[], fills, scope, &mut others, Fills::default())?;
				}
				Expr::TypeAlias { name, typ } => {
					if matches!(typ, TypeExpr::TupleStruct(..)) && TypeCtx::builtin_type(name) {
						let msg = format!("`{name}` is a builtin type");
						return Err(Diagnostic::new(msg, item.1.into_range()).with_label("pick another struct name"));
					}
					let mut typ = typ.clone();
					typ.walk_mut(&mut |t| {
						if let TypeExpr::Name(n) = t
							&& !TypeCtx::builtin_type(n)
							&& let Some(q) = scope.env.get(n.as_str())
						{
							n.clone_from(q);
						}
					});
					alias_items.push((name.as_str(), typ));
				}
				Expr::TraitDef { .. } => {}
				Expr::Claim {
					typ,
					type_params,
					traits: ts,
					via,
					fills,
				} if fills.is_empty()
					&& type_params.is_empty()
					&& via.is_none()
					&& !typ.contains("::")
					&& matches!(ts.as_slice(), [(t, a)] if a.is_empty() && !traits.contains_key(scope.qualify_trait(t).as_str())) =>
				{
					loose_refs.push(item)
				}
				Expr::Claim {
					typ,
					type_params,
					traits: claimed,
					via,
					fills,
				} => {
					let claimed: Vec<(String, &[Spanned<TypeExpr>])> = (claimed.iter())
						.map(|(tn, args)| (scope.qualify_trait(tn), args.as_slice()))
						.collect();
					if claimed.is_empty() && TypeCtx::builtin_type(typ) && scope.module != "core" {
						let msg = format!("`{typ}` is a builtin type and can only be amended in core");
						return Err(Diagnostic::new(msg, item.1.into_range()).with_label("not your type"));
					}
					let generic = claimed.iter().any(|(_, args)| !args.is_empty());
					for (tn, args) in &claimed {
						if !type_params.is_empty() && !is_hook_trait(tn) {
							let msg = "generic trait claims aren't supported yet".to_string();
							return Err(
								Diagnostic::new(msg, item.1.into_range()).with_label("remove the type parameters")
							);
						}
						trait_bodies.push(TraitBody {
							span: item.1,
							typ,
							trait_name: tn.clone(),
							args,
							via: via.as_deref(),
							methods: fills,
							scope,
						});
						match type_params.is_empty() {
							true => self.trait_impls.insert((typ.clone(), tn.clone())),
							false => self.generic_claims.insert((typ.clone(), tn.clone())),
						};
					}
					let decls: Vec<TraitFn> = claimed
						.iter()
						.filter_map(|(tn, _)| traits.get(tn.as_str()))
						.flat_map(|(.., ms)| trait_fns(ms))
						.collect();
					let claim = Fills { decls: &decls, generic };
					self.register_fills(typ, type_params, fills, scope, &mut others, claim)?;
				}
				Expr::Fn { name, body, ret, .. } if name == "main" => {
					main_body = Some(body);
					main_ret = ret.as_ref();
				}
				Expr::Fn {
					name,
					type_params,
					params,
					params_tuple,
					ret,
					body,
				} if !type_params.is_empty() => {
					let mut type_params = type_params.clone();
					qualify_bounds(scope, &mut type_params);
					self.generics.insert(
						name.clone(),
						GenericFnDef {
							params: params.clone(),
							params_tuple: *params_tuple,
							ret: ret.clone(),
							body: body.clone(),
							type_params,
							captures: vec![],
							self_name: None,
							module: scope.module.clone(),
						},
					);
				}
				Expr::Fn {
					name,
					params,
					params_tuple,
					ret,
					body,
					..
				} => {
					// `@test`
					let test_ann = (!name.contains("::"))
						.then(|| self.annotations.get(name))
						.flatten()
						.and_then(|anns| anns.iter().find_map(|a| ann(a, role::TEST)));
					if let Some(fields) = test_ann {
						if !self.include_tests {
							continue;
						}
						if !params.is_empty() || ret.is_some() {
							let msg = format!("test `{name}` must be `fn()`");
							return Err(Diagnostic::new(msg, item.1.into_range())
								.with_label("tests take no params and return nothing"));
						}
						let lit = |key: &str, i: usize| {
							fields.iter().enumerate().find_map(|(j, (k, v))| {
								(k.as_deref() == Some(key) || k.is_none() && j == i).then_some(&v.0)
							})
						};
						let display = match lit("name", 0) {
							Some(Expr::String(s)) => s.clone(),
							_ => name.clone(),
						};
						let skip = matches!(lit("skip", 1), Some(Expr::Bool(true)));
						self.tests.push((name.clone(), display, skip));
					}
					if others.iter().any(|f| f.key == *name) {
						let msg = format!("duplicate fn `{}`", display_name(name));
						return Err(Diagnostic::new(msg, item.1.into_range()).with_label("already defined"));
					}
					others.push(FnItem {
						key: name.clone(),
						scope,
						params: params.clone(),
						params_tuple: *params_tuple,
						ret: ret.clone(),
						body,
					})
				}
				Expr::Doc(_) => {}
				Expr::Bind {
					name,
					typ: Some((t, _)),
					value: Some(v),
					..
				} if matches!(v.0, Expr::Foreign) => {
					foreign_items.push((name.clone(), t.clone(), item.1, scope));
				}
				// statics
				Expr::Bind {
					mutable: true,
					name,
					typ,
					value: Some(v),
				} if has_main || !scope.module.is_empty() => {
					static_items.push((name.clone(), typ.clone(), (**v).clone(), scope, item.1));
				}
				Expr::Bind {
					mutable: false,
					name,
					typ: None,
					value: Some(v),
				} => {
					if let Some(te) = TypeExpr::from_expr(&v.0) {
						soft_aliases.push((name.clone(), te));
						if !scope.module.is_empty() {
							continue;
						}
					}
					loose_refs.push(item);
				}
				_ => loose_refs.push(item),
			}
		}

		let mut aliases: HashMap<String, TypeExpr> =
			alias_items.iter().map(|(name, te)| (name.to_string(), te.clone())).collect();
		aliases.extend(soft_aliases);

		for (name, te) in &mut aliases {
			if let TypeExpr::TupleStruct(n, _) = te {
				n.clone_from(name);
			}
		}

		// name-only registry
		let enum_names: HashMap<String, Vec<VariantInfo>> =
			enum_items.iter().map(|(name, ..)| (name.to_string(), Vec::new())).collect();

		let (const_map, const_anns) = (self.consts.clone(), self.annotations.clone());
		let consts = Consts {
			map: &const_map,
			anns: &const_anns,
		};

		// structs can ref each other
		let no_type_params: HashMap<String, Typ> = HashMap::new();
		let mut structs: HashMap<String, Vec<FieldDef>> = HashMap::new();
		let mut pending = struct_items;
		let mut placeholders: HashSet<String> = HashSet::new();
		let field = |types: &TypeCtx, p: &Param| {
			let typ = types.resolve(&p.typ, p.span)?;
			if matches!(typ, Typ::Ref(_)) && p.default.is_none() {
				let msg = "a reference field must be optional (`?&T`) or have a default";
				return Err(Diagnostic::new(msg, p.span.into_range()).with_label("no zero value for `&T`"));
			}
			Ok(FieldDef {
				name: p.name.clone(),
				typ,
				default: p.default.clone(),
				embedded: embedded(p),
				annotations: qualify_anns(types.scope, &p.annotations),
			})
		};
		while !pending.is_empty() {
			let (mut done, mut err) = (vec![], None);
			pending.retain(|(name, fields)| {
				let types = TypeCtx::new(&structs, &enum_names, &aliases, &no_type_params, &generics, &traits)
					.with_consts(consts)
					.with_scope(scope_of(name));
				let resolve = || {
					let mut fs: Vec<FieldDef> = fields.iter().map(|p| field(&types, p)).collect::<Result<_, _>>()?;
					if is_c_struct(&self.annotations, name) {
						for f in fs.iter_mut().filter(|f| matches!(f.typ, Typ::Fn(..))) {
							f.typ = Typ::Annotated(vec![role::C.into()], Box::new(f.typ.clone()));
						}
					}
					for (p, f) in fields.iter().zip(&fs) {
						if !ref_guarded(&f.typ, &placeholders) {
							return Err(Diagnostic::new(
								format!("`{name}` recurses for ever ever"),
								p.span.into_range(),
							)
							.with_label("would require infinitely nested fields"));
						}
					}
					Ok(fs)
				};
				match resolve() {
					Ok(fs) => {
						done.push((name.to_string(), fs));
						false
					}
					Err(e) => {
						err = Some(e);
						true
					}
				}
			});
			if done.is_empty() {
				if !placeholders.is_empty() {
					return Err(err.unwrap());
				}
				placeholders.extend(pending.iter().map(|(n, _)| n.to_string()));
				structs.extend(placeholders.iter().map(|n| (n.clone(), vec![])));
			}
			structs.extend(done);
		}
		check_annotations(&self.annotations, &structs, &generics, &self.consts, scope_of)?;
		check_c_structs(&self.annotations, &structs)?;

		let field_types =
			TypeCtx::new(&structs, &enum_names, &aliases, &no_type_params, &generics, &traits).with_consts(consts);

		// implicit traits
		for (tn, anns) in &self.annotations {
			if !traits.contains_key(tn.as_str()) || !anns.iter().any(|a| ann(a, role::IMPLICIT).is_some()) {
				continue;
			}

			for typ in structs.keys() {
				let pair = (typ.clone(), tn.clone());
				if self.trait_impls.contains(&pair) {
					continue;
				}

				let mark = others.len();
				let body = TraitBody {
					span: Span::default(),
					typ,
					trait_name: tn.clone(),
					args: &[],
					via: None,
					methods: &[],
					scope: scope_of(typ),
				};
				match check_impls(
					vec![body],
					&traits,
					&self.core_traits,
					&self.trait_impls,
					field_types,
					&mut others,
					&mut self.consts,
				) {
					Ok(()) => {
						self.trait_impls.insert(pair);
					}
					Err(_) => others.truncate(mark),
				}
			}
		}

		for b in trait_bodies.iter().filter(|b| b.trait_name == "Copy") {
			let drops = |set: &HashSet<(String, String)>| set.contains(&(b.typ.to_string(), "Drop".into()));
			if drops(&self.trait_impls) || drops(&self.generic_claims) {
				continue;
			}
			let msg = format!("`{}` claims `Copy` without `Drop`, so nothing runs the hook", b.typ);
			return Err(Diagnostic::new(msg, b.span.into_range()).with_label("claim `Drop` too"));
		}

		check_impls(
			trait_bodies,
			&traits,
			&self.core_traits,
			&self.trait_impls,
			field_types,
			&mut others,
			&mut self.consts,
		)?;

		let enums: HashMap<String, Vec<VariantInfo>> = enum_items
			.iter()
			.map(|(name, backing, variants)| {
				let types = TypeCtx::new(&structs, &enum_names, &aliases, &no_type_params, &generics, &traits)
					.with_consts(consts)
					.with_scope(scope_of(name));
				let mut vs = build_variants(variants, types)?;
				if let Some(bt) = backing {
					apply_backing(bt, &mut vs, variants, types)?;
				}
				Ok((name.to_string(), vs))
			})
			.collect::<Result<_, _>>()?;

		// hoist fns
		let mut funcs: HashMap<String, FnSig> = HashMap::new();
		for item in &others {
			let mut aliases = aliases.clone();
			if let Some(t) = item.key.rsplit_once('.').map(|(t, _)| t) {
				aliases.insert("Self".into(), TypeExpr::Name(t.into()));
			}
			let types = TypeCtx::new(&structs, &enums, &aliases, &no_type_params, &generics, &traits)
				.with_consts(consts)
				.with_scope(item.scope);
			let resolved = types.resolve_params(&item.params)?;
			let params: Vec<FnParam> = (item.params.iter().zip(&resolved))
				.map(|(p, (_, t, _))| FnParam::of(p, t.clone()))
				.collect();
			let access: Vec<Access> = item.params.iter().map(|p| p.access).collect();
			let ret = match &item.ret {
				Some((ret_te, ret_span)) => types.resolve(ret_te, *ret_span)?,
				None => Typ::unit(),
			};
			check_param_defaults(&item.params)?;
			check_varargs(&item.key, &item.params)?;
			// `@export` / `@c`
			let anns = self.annotations.get(&item.key);
			let ann_span = |target| anns.into_iter().flatten().find_map(|a| Some((ann(a, target)?, a.1)));
			let mut is_c_fn = false;
			let is_unsafe = ann_span("unsafe").is_some();
			let pure = ann_span(role::PURE).map(|(_, s)| s);
			if let Some(span) = pure
				&& access.contains(&Access::Mut)
			{
				let msg = "a `@pure` fn can't take `mut` params";
				return Err(Diagnostic::new(msg, span.into_range()).with_label("mutation is a side effect"));
			}
			if let Some((fields, span)) = ann_span(role::EXPORT) {
				check_c_sig(&item.key, &params, &ret, span)?;
				let sym = match fields.first() {
					Some((_, (Expr::String(s), _))) if !s.is_empty() => s.clone(),
					_ => display_name(&item.key).replace('.', "_"),
				};
				self.exports.insert(item.key.clone(), sym);
			} else if let Some((_, span)) = ann_span(role::C) {
				check_c_sig(&item.key, &params, &ret, span)?;
				is_c_fn = true;
			}
			let (sym, linkage) = self.symbol(&item.key);
			let mut sig = self.declare_fn(&sym, linkage, params, access, ret);
			sig.foreign = self.exports.contains_key(&item.key) || is_c_fn;
			sig.unsafe_call = is_unsafe;
			sig.pure = pure.is_some();
			funcs.insert(item.key.clone(), sig);
		}

		for (name, fn_type, span, scope) in &foreign_items {
			let (span, scope) = (*span, *scope);
			let TypeExpr::Fn(param_types, ret) = fn_type else {
				unreachable!("loader validated foreign fn type")
			};
			let types = TypeCtx::new(&structs, &enums, &aliases, &no_type_params, &generics, &traits)
				.with_consts(consts)
				.with_scope(scope);
			let params: Vec<FnParam> = param_types
				.iter()
				.map(|(n, _, t)| {
					Ok(FnParam {
						name: n.clone(),
						variadic: matches!(t, TypeExpr::Variadic(_)),
						..FnParam::new(types.param(t, span)?)
					})
				})
				.collect::<Result<_, Diagnostic>>()?;
			let access: Vec<Access> = param_types.iter().map(|(_, a, _)| *a).collect();
			let ret = types.resolve(ret, span)?;
			let mut bare = display_name(name).to_string();
			// `@link`
			for a in self.annotations.get(name).into_iter().flatten() {
				let Some(fields) = ann(a, role::LINK) else { continue };
				let err = |msg: String, label: &str| Diagnostic::new(msg, a.1.into_range()).with_label(label);
				let mut lib = None;
				for (label, (v, _)) in fields {
					match (label.as_deref(), v) {
						(Some("name"), Expr::String(s)) => bare = s.clone(),
						(None, Expr::String(s)) => lib = Some(s),
						_ => {}
					}
				}
				let Some(lib) = lib else { continue };
				// search module dir, then each root
				let explicit = lib.contains(MAIN_SEPARATOR) || lib.contains(DLL_SUFFIX);
				let file = if explicit {
					lib.clone()
				} else {
					format!("{DLL_PREFIX}{lib}{DLL_SUFFIX}")
				};
				let dirs = program.roots.iter().flat_map(|r| [r.join(&scope.module), r.clone()]);
				let found = dirs.map(|d| d.join(&file)).find(|p| p.is_file());
				let lib = match found.or_else(|| explicit.then(|| lib.into())) {
					Some(path) => std::fs::canonicalize(path)
						.map_err(|e| err(format!("cannot open library `{lib}`: {e}"), "no such file"))?
						.to_string_lossy()
						.into_owned(),
					None => lib.clone(),
				};
				if !self.link_libs.contains(&lib) {
					if !load_library(&lib) {
						return Err(err(format!("unknown library `{lib}`"), "dlopen failed"));
					}
					self.link_libs.push(lib);
				}
			}
			let bare = bare.as_str();
			if !process_symbol_exists(bare) {
				let msg = format!("unknown foreign symbol `{bare}`");
				return Err(Diagnostic::new(msg, span.into_range()).with_label("no such symbol"));
			}
			for p in &params {
				if let Typ::Fn(ps, r) = &p.typ {
					check_c_sig(bare, ps, r, span)?;
				}
			}
			let mut sig = self.declare_fn(bare, Linkage::Import, params, access, ret);
			sig.foreign = true;
			funcs.insert(name.clone(), sig);
		}

		// lower bodies
		let mut unlowered: HashMap<FuncId, usize> = others
			.iter()
			.enumerate()
			.map(|(i, item)| (funcs[&item.key].id, i))
			.filter(|(id, _)| !self.defined.contains(id))
			.collect();
		if self.aot {
			self.wanted.extend(unlowered.keys().copied());
		} else {
			self.wanted.extend(
				self.tests
					.iter()
					.map(|(key, ..)| key)
					.chain(self.exports.keys())
					.chain(self.roots.iter())
					.filter_map(|key| funcs.get(key).map(|sig| sig.id)),
			);
		}

		// a `str` wrapper per struct
		let mut render = HashMap::new();
		for (name, _) in self.trait_impls.clone() {
			if render.contains_key(&name) {
				continue;
			}
			let types =
				TypeCtx::new(&structs, &enums, &aliases, &no_type_params, &generics, &traits).with_consts(consts);
			let styp = types.named(&name, (0..0).into())?;
			let param = [("self".into(), styp.clone(), Access::Read)];
			let def = FnDef {
				params: &param,
				..FnDef::default()
			};
			let (mut trans, block) = self.translator(&def, &funcs, types);
			let val = trans.b.block_params(block)[0];
			let s = trans.derived_str(val, &styp);
			trans.emit_return(s, Typ::Str, (0..0).into())?;
			trans.b.finalize();
			render.insert(name.clone(), self.finish_fn(&oi_symbol(&format!("{name}#str"))));
		}

		// define vtables now that every concrete method has a FuncId
		for (typ, tn) in self.trait_impls.clone() {
			if is_hook_trait(&tn) {
				continue;
			}
			let (_, tparams, tfields, tmethods) = traits[tn.as_str()];
			if !tparams.is_empty() {
				continue;
			}
			let methods: Vec<&str> = trait_fns(tmethods).map(|(n, ..)| n).collect();
			let m = methods.len();
			let f = tfields.len();
			let mut bytes = vec![0u8; (m + f + 1) * 8];
			let ftypes =
				TypeCtx::new(&structs, &enums, &aliases, &no_type_params, &generics, &traits).with_consts(consts);
			let mut const_slots = Vec::new();
			for (i, tf) in tfields.iter().enumerate() {
				match structs.get(typ.as_str()).and_then(|fs| field_slot(fs, &tf.name)) {
					Some((enc, _)) => bytes[(m + i) * 8..(m + i + 1) * 8].copy_from_slice(&enc.to_le_bytes()),
					None => {
						let want = ftypes.resolve(&tf.typ, tf.span)?;
						let lit = self.consts[&format!("{typ}::{}", tf.name)].0.clone();
						let sym = oi_symbol(&format!("const_{typ}_{tn}_{}", tf.name));
						const_slots.push(((m + i) * 8, self.const_cell(&sym, &lit, &want)));
					}
				}
			}
			let mut desc = DataDescription::new();
			desc.define(bytes.into_boxed_slice());
			for (off, cell) in const_slots {
				let gv = self.module.declare_data_in_data(cell, &mut desc);
				desc.write_data_addr(off as u32, gv, 2);
			}
			for (i, name) in methods.iter().enumerate() {
				let id = funcs[&format!("{typ}.{name}")].id;
				self.wanted.push(id);
				let fref = self.module.declare_func_in_data(id, &mut desc);
				desc.write_function_addr((i * 8) as u32, fref);
			}
			let fref = self.module.declare_func_in_data(render[typ.as_str()], &mut desc);
			desc.write_function_addr(((m + f) * 8) as u32, fref);
			let sym = oi_symbol(&format!("vtable_{typ}_{tn}"));
			let id = self
				.module
				.declare_data(&sym, Linkage::Local, false, false)
				.expect("declare vtable");
			define_once(&mut self.module, id, &desc);
		}

		// one zeroed cell per static
		for (name, annot, init, scope, span) in static_items {
			let types = TypeCtx::new(&structs, &enums, &aliases, &no_type_params, &generics, &traits)
				.with_consts(consts)
				.with_scope(scope);
			let typ = match &annot {
				Some((t, s)) => types.resolve(t, *s)?,
				None => static_typ(&init.0, &types, span)?,
			};
			let sym = oi_symbol(&format!("static_{name}"));
			self.module
				.declare_data(&sym, Linkage::Local, true, false)
				.expect("declare static");
			define_data(&mut self.module, &sym, vec![0; 8]);
			self.statics.insert(name.clone(), (sym, typ));
			self.static_inits.push((name, init));
		}

		// gather loose top-level statements
		let loose: Vec<Spanned<Expr>>;
		let entry: &[Spanned<Expr>] = match main_body {
			Some(body) => {
				if let Some(first) = loose_refs.first() {
					return Err(Diagnostic::new(
						"top-level statements are not allowed alongside `fn main`",
						first.1.into_range(),
					)
					.with_label("move this inside a function")
					.with_note("`fn main` is the entrypoint, so loose statements have nowhere to run"));
				}
				body
			}
			None => {
				loose = loose_refs.into_iter().cloned().collect();
				&loose
			}
		};

		let types = TypeCtx::new(&structs, &enums, &aliases, &no_type_params, &generics, &traits)
			.with_consts(consts)
			.with_scope(scopes["main"]);
		let ret = match main_ret {
			Some((te, span)) => Some((types.resolve(te, *span)?, *span)),
			None => None,
		};
		if let Some((typ, span)) = &ret
			&& !fallible(typ)
		{
			let msg = format!("`main` cannot return `{typ}`");
			return Err(Diagnostic::new(msg, span.into_range()).with_label("`main` returns nothing or `!`"));
		}
		let typ = self.translate(
			FnDef {
				params_tuple: true,
				body: entry,
				is_main: true,
				ret,
				..FnDef::default()
			},
			&funcs,
			types,
		)?;
		let entry_id = self.finish_fn("oi_main");
		let id = self.compile_entry(entry_id, typ, &funcs, types);

		loop {
			while let Some(id) = self.wanted.pop() {
				let Some(i) = unlowered.remove(&id) else { continue };
				let item = &others[i];
				let self_type = item.key.rsplit_once('.').map(|(t, _)| t);
				let mut aliases = aliases.clone();
				if let Some(t) = self_type {
					aliases.insert("Self".into(), TypeExpr::Name(t.into()));
				}
				let types = TypeCtx::new(&structs, &enums, &aliases, &no_type_params, &generics, &traits)
					.with_consts(consts)
					.with_scope(item.scope);
				let (params, ret) = types.resolve_params_ret(&item.params, &item.ret)?;
				let ret = ret.or_else(|| Some((funcs[&item.key].ret.clone(), (0..0).into())));
				self.translate(
					FnDef {
						params: &params,
						params_tuple: item.params_tuple,
						ret,
						body: item.body,
						self_type,
						foreign: funcs[&item.key].foreign,
						pure: funcs[&item.key].pure,
						is_test: self.tests.iter().any(|(n, ..)| *n == item.key),
						..FnDef::default()
					},
					&funcs,
					types,
				)?;
				self.finish_fn(&self.symbol(&item.key).0);
			}
			let Some((sym, def, subst)) = self.pending.pop().or_else(|| self.compile_printers(&funcs, types)) else {
				break;
			};
			let home = scopes[if def.module.is_empty() { "main" } else { &def.module }];
			let types = TypeCtx::new(&structs, &enums, &aliases, &subst, &generics, &traits)
				.with_consts(consts)
				.with_scope(home);
			let (params, ret) = types.resolve_params_ret(&def.params, &def.ret)?;
			let ret = ret.or_else(|| Some((self.mono[&sym].ret.clone(), (0..0).into())));
			let self_sig = self.mono[&sym].clone();
			self.translate(
				FnDef {
					params: &params,
					params_tuple: def.params_tuple,
					ret,
					body: &def.body,
					captures: &def.captures,
					self_fn: def.self_name.as_deref().map(|n| (n, &self_sig)),
					..FnDef::default()
				},
				&funcs,
				types,
			)?;
			self.finish_fn(&sym);
		}

		self.hoisted = funcs;

		Ok(id)
	}

	fn compile_entry(&mut self, entry: FuncId, typ: Typ, funcs: &HashMap<String, FnSig>, types: TypeCtx) -> FuncId {
		let (mut trans, _) = self.translator(&FnDef::default(), funcs, types);

		let callee = trans.module.declare_func_in_func(entry, trans.b.func);
		let call = trans.b.ins().call(callee, &[]);
		if let Some(val) = trans.b.inst_results(call).first().copied() {
			trans.emit_fail(val, &typ);
		}
		trans.b.ins().return_(&[]);
		trans.b.finalize();

		self.finish_fn("__oi_main")
	}

	// Queued printer bodies, and whatever they queued in turn.
	fn compile_printers(&mut self, funcs: &HashMap<String, FnSig>, types: TypeCtx) -> Option<Pending> {
		while let Some((sym, typ, quote, sink)) = self.printers.pop() {
			let params = [(String::new(), typ.clone(), Access::Read)];
			let def = FnDef {
				params: &params,
				..FnDef::default()
			};
			let (mut trans, block) = self.translator(&def, funcs, types);
			let val = trans.b.block_params(block)[0];
			trans.emit_variant(&typ, val, quote, sink);
			trans.b.ins().return_(&[]);
			trans.b.finalize();
			self.finish_fn(&sym);
		}
		self.pending.pop()
	}

	// A fn's object symbol.
	fn symbol(&self, key: &str) -> (String, Linkage) {
		match self.exports.get(key) {
			Some(sym) => (sym.clone(), Linkage::Export),
			None => (oi_symbol(key), Linkage::Local),
		}
	}

	// Declare a hoisted fn's signature ahead of its body.
	fn declare_fn(
		&mut self,
		symbol: &str,
		linkage: Linkage,
		params: Vec<FnParam>,
		access: Vec<Access>,
		ret: Typ,
	) -> FnSig {
		let int = self.module.target_config().pointer_type();
		let mut sig = self.module.make_signature();
		sig.params.extend(params.iter().map(|p| AbiParam::new(cl_type(&p.typ, int))));
		if !ret.is_unit() {
			sig.returns.push(AbiParam::new(cl_type(&ret, int)));
		}
		let id = self.module.declare_function(symbol, linkage, &sig).expect("declare function");
		FnSig {
			id,
			params,
			access,
			ret,
			foreign: false,
			unsafe_call: linkage == Linkage::Import,
			pure: false,
		}
	}

	fn finish_fn(&mut self, name: &str) -> FuncId {
		let id = self
			.module
			.declare_function(name, Linkage::Local, &self.ctx.func.signature)
			.expect("declare function");
		if self.defined.insert(id) {
			match &mut self.cache {
				None => self.module.define_function(id, &mut self.ctx).expect("define function"),
				Some(store) => {
					self.ctx
						.compile_with_cache(self.module.isa(), store, &mut Default::default())
						.expect("compile function");
					let code = self.ctx.compiled_code().expect("just compiled");
					let relocs: Vec<_> = code
						.buffer
						.relocs()
						.iter()
						.map(|r| ModuleReloc::from_mach_reloc(r, &self.ctx.func, id))
						.collect();
					self.module
						.define_function_bytes(id, code.buffer.alignment as u64, code.code_buffer(), &relocs)
						.expect("define function");
				}
			}
		}
		self.module.clear_context(&mut self.ctx);
		id
	}

	fn translator<'a>(
		&'a mut self,
		def: &FnDef,
		funcs: &'a HashMap<String, FnSig>,
		types: TypeCtx<'a>,
	) -> (Translator<'a, M>, Block) {
		let int = self.module.target_config().pointer_type();
		let mut b = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_ctx);
		for (_, typ, _) in def.params {
			b.func.signature.params.push(AbiParam::new(cl_type(typ, int)));
		}
		if !def.captures.is_empty() {
			b.func.signature.params.push(AbiParam::new(int));
		}
		let block = b.create_block();
		b.append_block_params_for_function_params(block);
		b.switch_to_block(block);
		b.seal_block(block);

		let trans = Translator {
			int,
			b,
			vars: HashMap::new(),
			params: vec![],
			dollar: None,
			module: &mut self.module,
			funcs,
			structs: types.structs,
			enums: types.enums,
			aliases: types.aliases,
			type_params: types.type_params,
			generics: types.generics,
			traits: types.traits,
			generic_fns: &self.generics,
			trait_impls: &self.trait_impls,
			generic_claims: &self.generic_claims,
			core_traits: &self.core_traits,
			scope: types.scope,
			module_scopes: &self.module_scopes,
			map: &self.map,
			publics: &self.publics,
			privates: &self.privates,
			reexports: &self.reexports,
			consts: &self.consts,
			statics: &self.statics,
			annotations: &self.annotations,
			mono: &mut self.mono,
			pending: &mut self.pending,
			wanted: &mut self.wanted,
			printers: &mut self.printers,
			descs: &mut self.descs,
			string_idx: &mut self.string_idx,
			atoms: &mut self.atoms,
			ret: def.ret.clone(),
			loops: vec![],
			unsafely: 0,
			scopes: vec![vec![]],
			temps: HashMap::new(),
			self_type: def.self_type.map(str::to_owned),
			is_main: def.is_main,
			pure: def.pure,
			self_name: None,
		};

		(trans, block)
	}

	fn translate(&mut self, def: FnDef, funcs: &HashMap<String, FnSig>, types: TypeCtx) -> Result<Typ, Diagnostic> {
		let decl_span = def.ret.as_ref().map(|(_, s)| *s);
		let inits = match def.is_main || def.is_test {
			true => self.static_inits.clone(),
			false => vec![],
		};
		let (mut trans, block) = self.translator(&def, funcs, types);

		trans.seed_statics(&inits)?;
		let param_vals: Vec<Value> = trans.b.block_params(block).to_vec();
		for ((name, typ, access), &val) in def.params.iter().zip(param_vals.iter()) {
			let val = if def.foreign && matches!(typ, Typ::Fn(..)) {
				trans.fn_cell(val)
			} else {
				val
			};
			let cl = trans.b.func.dfg.value_type(val);
			let var = trans.b.declare_var(cl);
			trans.b.def_var(var, val);
			if *access == Access::Move {
				trans.own_local(var, typ);
			}
			let mutable = *access == Access::Mut;
			let local = Local {
				var,
				typ: typ.clone(),
				mutable,
				boxed: mutable && name != "self",
				stat: false,
			};
			trans.vars.insert(name.clone(), local.clone());
			trans.params.push(local);
		}
		trans.bind_dollar(def.params_tuple);

		if !def.captures.is_empty() {
			let env = param_vals[def.params.len()];
			for (i, (name, typ, boxed)) in def.captures.iter().enumerate() {
				let cl = if *boxed { trans.int } else { cl_type(typ, trans.int) };
				let val = trans.b.ins().load(cl, MemFlags::new(), env, ((i + 1) * 8) as i32);
				let var = trans.b.declare_var(cl);
				trans.b.def_var(var, val);
				let local = Local {
					var,
					typ: typ.clone(),
					mutable: *boxed,
					boxed: *boxed,
					stat: false,
				};
				trans.vars.insert(name.clone(), local);
			}
		}

		// fn literal is bound in its own body and so it can refer to itself
		if let Some((name, sig)) = def.self_fn {
			let val = match def.captures.is_empty() {
				true => trans.fn_object(sig.id),
				false => param_vals[def.params.len()],
			};
			let typ = Typ::Fn(sig.value_params(), Box::new(sig.ret.clone()));
			trans.bind_local(name, val, typ, false);
		}

		let tail_target = trans.ret.as_ref().map(|(t, _)| t.clone());
		if let Some((val, typ)) = trans.block_tail(def.body, tail_target.as_ref())? {
			let span = def.body.last().map(|s| s.1).or(decl_span).unwrap_or((0..0).into());
			if let Some(e) = def.body.last() {
				trans.move_resource(e, &typ)?;
			}
			trans.emit_return(val, typ, span)?;
		}
		trans.b.finalize();

		Ok(trans.ret.map(|(t, _)| t).unwrap_or(Typ::unit()))
	}
}

impl Compiler {
	pub fn compile(&mut self, program: &Program) -> Result<*const u8, Diagnostic> {
		let (t, n) = (Instant::now(), self.timings.len());
		let id = self.build(program)?;
		self.module.finalize_definitions().expect("finalize definitions");
		self.time_codegen(t, n);
		Ok(self.module.get_finalized_function(id))
	}

	pub(crate) fn finalized_test(&self, name: &str) -> fn() {
		let ptr = self.module.get_finalized_function(self.hoisted[name].id);
		// SAFETY: `@test` fns are checked to be `fn()` at compile
		unsafe { std::mem::transmute::<*const u8, fn()>(ptr) }
	}
}
