// User-defined comptime macros.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ast::{Capture, Child, EnumVariant, Expr, MatchArm, Param, Span, Spanned, TypeExpr};
use crate::diagnostics::{Diagnostic, arity_err, fail};
use crate::loader::{Module, Program, Scope};
use crate::runtime;

use super::Compiler;

use Child::{List, One};

const BUILTINS: [&str; 6] = ["dbg", "assert", "panic", "todo", "unreachable", "src"];
const MAX_DEPTH: usize = 64;
const MAX_PARAMS: usize = 4;

pub(crate) const RT_QUOTE: &str = "oi_rt_quote";
pub(crate) const RT_AST_LIT: &str = "oi_rt_ast_lit";
pub(crate) const RT_AST_METHOD: &str = "oi_rt_ast_method";
pub(crate) const RT_QUOTE_MATCH: &str = "oi_rt_quote_match";

// `program` with its modules swapped out, for a stage-0 compile.
pub(super) fn with_modules(program: &Program, modules: Vec<Module>) -> Program {
	Program {
		map: program.map.clone(),
		modules,
		publics: program.publics.clone(),
		reexports: program.reexports.clone(),
		consts: program.consts.clone(),
		annotations: program.annotations.clone(),
		roots: program.roots.clone(),
		core_origin: program.core_origin.clone(),
	}
}

#[derive(Default)]
struct Expander {
	// stage-0 fns, in definition order
	defs: Vec<Spanned<Expr>>,
	// name -> (arity, stage-0 fn pointer)
	macros: HashMap<String, (usize, *const u8)>,
	// qualified names visible outside their own module
	publics: HashSet<String>,
	// keeps the stage-0 JIT and its code alive for every call this pass makes
	stage0: Option<Compiler>,
	hoisted: Vec<Spanned<Expr>>,
}

// Every name a pattern binds.
fn pat_names(pat: &mut Spanned<Expr>) -> Vec<&mut String> {
	let elems: Vec<&mut Spanned<Expr>> = match &mut pat.0 {
		Expr::Ident(n) => return vec![n],
		Expr::Tuple(fields) | Expr::StructLit { fields, .. } => fields.iter_mut().map(|(_, e)| e).collect(),
		Expr::Array(elems) => elems.iter_mut().collect(),
		_ => return vec![],
	};
	elems
		.into_iter()
		.filter_map(|(e, _)| if let Expr::Ident(n) = e { Some(n) } else { None })
		.collect()
}

// Visit every name-binding site in expr.
// Skips deliberate `%name` captures.
fn for_binders(e: &mut Expr, f: &mut impl FnMut(&mut String)) {
	match e {
		Expr::Bind { name, .. } if !name.starts_with('%') => f(name),
		Expr::PatBind {
			pat, mutable: Some(_), ..
		}
		| Expr::For { pat, .. } => pat_names(pat).into_iter().for_each(f),
		Expr::AnonFn { params, .. } => params
			.iter_mut()
			.filter(|p| !p.name.is_empty() && !p.name.starts_with('%'))
			.for_each(|p| f(&mut p.name)),
		Expr::Match { arms, .. } => arms.iter_mut().flat_map(|a| &mut a.binding).for_each(f),
		_ => {}
	}
}

// The name a `%name` can take.
fn def_name(e: &mut Expr) -> Option<&mut String> {
	match e {
		Expr::Field { field, .. } | Expr::IndexAssign { field: Some(field), .. } => Some(field),
		Expr::Bind { name, .. }
		| Expr::Fn { name, .. }
		| Expr::StructDef { name, .. }
		| Expr::EnumDef { name, .. }
		| Expr::TypeAlias { name, .. }
		| Expr::StructLit { name, .. }
		| Expr::Claim { typ: name, .. } => Some(name),
		_ => None,
	}
}

// Resolve a bare macro name through scope.
fn resolve_bare(name: &str, scope: &Scope) -> String {
	let key = format!("{name}!");
	scope.env.get(&key).cloned().unwrap_or(key)
}

// Get a fn's defining module, encoded in its qualified name.
fn owner(e: &Spanned<Expr>) -> &str {
	match &e.0 {
		Expr::Fn { name, .. } => name.rsplit_once("::").map_or("main", |(m, _)| m),
		_ => "main",
	}
}

// Rewrite direct macro calls from stage-0 bodies into calls of their compiled fns.
// Unquote expressions run at comptime.
// Other calls inside quotes stay as calls and expand later, when the produced Ast is spliced.
fn direct_calls(e: &mut Expr, macros: &HashMap<String, (usize, *const u8)>, scope: &Scope, in_quote: bool) {
	if !in_quote && let Expr::MacroCall { name, args } = e {
		let resolved = resolve_bare(name, scope);
		if macros.contains_key(&resolved) {
			let args = std::mem::take(args);
			*e = Expr::Call {
				name: resolved,
				type_args: vec![],
				args,
			};
		}
	}
	let child_in_quote = match e {
		Expr::Quote(_) => true,
		Expr::UnquoteExpr(_) | Expr::UnquoteSplat(_) => false,
		_ => in_quote,
	};
	e.for_children(|c| match c {
		List(list) => list
			.iter_mut()
			.for_each(|(e, _)| direct_calls(e, macros, scope, child_in_quote)),
		One((e, _)) => direct_calls(e, macros, scope, child_in_quote),
	});
}

impl Expander {
	fn define(
		&mut self,
		name: String,
		mut params: Vec<Param>,
		ret: Option<Spanned<TypeExpr>>,
		body: Vec<Spanned<Expr>>,
		span: Span,
	) -> Result<(), Diagnostic> {
		let ast = |te: &TypeExpr| matches!(te, TypeExpr::Name(n) if n == "Ast");
		let param = |te: &TypeExpr| ast(te) || matches!(te, TypeExpr::Name(n) if n == "Tokens");
		let bare = name.rsplit("::").next().unwrap_or(&name);
		if BUILTINS.contains(&bare.trim_end_matches('!')) {
			return fail(format!("`{name}` is a builtin macro"), span, "reserved name");
		}
		if self.macros.contains_key(&name) {
			return fail(format!("`{name}` is already defined"), span, "duplicate macro");
		}
		if let Some(p) = params.iter().find(|p| !param(&p.typ)) {
			return fail("macro params must be `Ast` or `Tokens`", p.span, "not Ast");
		}
		params.iter_mut().for_each(|p| p.typ = TypeExpr::Name("Ast".into()));
		if params.len() > MAX_PARAMS {
			return fail("macros take at most 4 arguments for now", span, "too many parameters");
		}
		if let Some((te, rspan)) = &ret
			&& !ast(te)
		{
			return fail("macros return `Ast`", *rspan, "not Ast");
		}
		self.macros.insert(name.clone(), (params.len(), std::ptr::null()));
		let f = Expr::Fn {
			name,
			type_params: vec![],
			params_tuple: params.len() != 1,
			params,
			ret: Some((TypeExpr::Name("Ast".into()), (0..0).into())),
			body,
		};
		self.defs.push((f, span));
		Ok(())
	}

	// Compile every macro body as a real fn, in one synthetic program, and grab their pointers.
	fn compile_stage0(
		&mut self,
		program: &Program,
		rest: &mut HashMap<String, Vec<Spanned<Expr>>>,
	) -> Result<(), Diagnostic> {
		let scopes: HashMap<&str, &Scope> = program.modules.iter().map(|m| (m.name.as_str(), &m.scope)).collect();
		for e in &mut self.defs {
			let scope = scopes[owner(e)];
			direct_calls(&mut e.0, &self.macros, scope, false);
		}
		let defs = std::mem::take(&mut self.defs);
		let modules: Vec<Module> = program
			.modules
			.iter()
			.map(|m| {
				let lent = |e: &Expr| match e {
					_ if m.name != "main" => true,
					Expr::Fn { name, .. } => name != "main",
					Expr::Bind { .. } => false,
					_ => super::comp::is_def(e),
				};
				let mut items: Vec<_> = (rest.get_mut(&m.name).expect("every module was seeded").iter_mut())
					.filter_map(|it| (lent(&it.0) && !self.calls_macro(&mut it.0, &m.scope)).then(|| it.clone()))
					.collect();
				items.extend(defs.iter().filter(|d| owner(d) == m.name).cloned());
				Module {
					name: m.name.clone(),
					items,
					scope: m.scope.clone(),
				}
			})
			.collect();
		let mut synthetic = with_modules(program, modules);
		synthetic.annotations.retain(|k, _| k.contains("::"));
		let compiler = self.stage0.get_or_insert_with(Compiler::default);
		compiler.stage0 = true;
		compiler.roots = self.macros.keys().cloned().collect();
		compiler.compile(&synthetic)?;
		for (name, (_, ptr)) in &mut self.macros {
			*ptr = compiler.module.get_finalized_function(compiler.hoisted[name].id);
		}
		Ok(())
	}

	// Checks whether item calls a user macro.
	fn calls_macro(&self, e: &mut Expr, scope: &Scope) -> bool {
		let mut found = false;
		e.walk(&mut |x| {
			if let Expr::MacroCall { name, .. } = x {
				let key = self.resolve(name, scope, (0..0).into()).unwrap_or_default();
				found |= self.macros.contains_key(&key);
			}
			x.types()
				.into_iter()
				.for_each(|t| t.walk_mut(&mut |t| found |= self.type_call(t, scope).is_some()));
		});
		found
	}

	// A `name!(T)` call.
	fn type_call(&self, t: &TypeExpr, scope: &Scope) -> Option<Spanned<Expr>> {
		if let TypeExpr::Annotated(anns, inner) = t
			&& let [(Expr::Ident(name), span)] = &anns[..]
			&& self.macros.contains_key(&resolve_bare(name, scope))
		{
			let (name, args) = (name.clone(), vec![(type_ast(inner), *span)]);
			return Some((Expr::MacroCall { name, args }, *span));
		}
		None
	}

	// Swap each `@name T` type for the one `name!(T)` defines, hoisting its definitions.
	fn expand_types(&mut self, e: &mut Expr, scope: &Scope, depth: usize) -> Result<(), Diagnostic> {
		let mut res = Ok(());
		for t in e.types() {
			t.walk_mut(&mut |t| {
				if res.is_ok()
					&& let Some(call) = self.type_call(t, scope)
				{
					res = self.call(&call, scope, depth).map(|defs| {
						let mut defs = defs.unwrap_or_default();
						defs.iter_mut()
							.filter_map(|d| def_name(&mut d.0))
							.for_each(|n| *n = scope.qualify_name(n));
						let name = defs.first().and_then(|d| d.0.def_name()).unwrap_or_default().to_string();
						if !self.hoisted.iter().any(|h| h.0.def_name() == Some(&name)) {
							self.hoisted.extend(defs);
						}
						*t = TypeExpr::Name(name);
					});
				}
			});
		}
		res
	}

	// Resolve a macro call against scope, enforcing privacy.
	fn resolve(&self, name: &str, scope: &Scope, span: Span) -> Result<String, Diagnostic> {
		let Some((module, rest)) = name.split_once('.') else {
			return Ok(resolve_bare(name, scope));
		};
		let Some(vis) = scope.visible.get(module) else {
			return fail(format!("cannot find module `{module}`"), span, "no such module");
		};
		let key = format!("{}::{rest}!", vis.module);
		if !self.publics.contains(&key) {
			return fail(format!("`{rest}` is private to module `{module}`"), span, "not public");
		}
		Ok(key)
	}

	// Instantiate expr if it calls a user macro.
	fn call(
		&mut self,
		e: &Spanned<Expr>,
		scope: &Scope,
		depth: usize,
	) -> Result<Option<Vec<Spanned<Expr>>>, Diagnostic> {
		let Expr::MacroCall { name, args } = &e.0 else {
			return Ok(None);
		};
		let key = self.resolve(name, scope, e.1)?;
		let Some(&(arity, ptr)) = self.macros.get(&key) else {
			return Ok(None);
		};
		if args.len() != arity {
			return arity_err(&format!("`{name}!`"), arity, args.len(), "argument", e.1);
		}
		if depth >= MAX_DEPTH {
			return fail("macro expansion is too deep", e.1, "recursion limit");
		}
		type Ptr = *mut Spanned<Expr>;
		let boxed: Vec<Ptr> = args
			.iter()
			.map(|a| match &a.0 {
				Expr::Quote(q) => one(q.clone(), a.1),
				_ => a.clone(),
			})
			.map(|s| Box::into_raw(Box::new(s)))
			.collect();
		let arg = |i: usize| boxed.get(i).copied().unwrap_or(std::ptr::null_mut());
		// SAFETY: stage-0 fns take at most MAX_PARAMS pointer args, all in registers on the ABIs cranelift targets, so a fixed-shape call just leaves the extras unread.
		let f = unsafe { std::mem::transmute::<*const u8, fn(Ptr, Ptr, Ptr, Ptr) -> Ptr>(ptr) };
		DEFS.with_borrow_mut(|d| d.0 = scope.clone());
		let (e0, span) = unsafe { *Box::from_raw(f(arg(0), arg(1), arg(2), arg(3))) };
		if let Some(msg) = ERROR.take() {
			return fail(msg, e.1, format!("while running `{name}!`"));
		}
		let mut body = match e0 {
			Expr::Block(stmts) => stmts,
			other => vec![(other, span)],
		};
		self.expand(List(&mut body), scope, depth + 1)?;
		Ok(Some(body))
	}

	// Walk the tree, expanding macro calls.
	fn expand(&mut self, c: Child, scope: &Scope, depth: usize) -> Result<(), Diagnostic> {
		match c {
			List(body) => {
				let mut i = 0;
				while i < body.len() {
					match self.call(&body[i], scope, depth)? {
						Some(stmts) => {
							let n = stmts.len();
							body.splice(i..i + 1, stmts);
							i += n;
						}
						None => {
							self.expand(One(&mut body[i]), scope, depth)?;
							i += 1;
						}
					}
				}
				Ok(())
			}
			One(e) => {
				if let Some(stmts) = self.call(e, scope, depth)? {
					e.0 = one(stmts, e.1).0;
					return Ok(());
				}
				self.expand_types(&mut e.0, scope, depth)?;
				match &e.0 {
					Expr::Fn { name, .. } if name.contains('!') => Ok(()),
					Expr::Quote(_) => Ok(()),
					Expr::Unquote(_) | Expr::UnquoteExpr(_) | Expr::UnquoteSplat(_) | Expr::UnquoteBind(..) => {
						fail("unquotes only make sense inside a macro template", e.1, "stray unquote")
					}
					Expr::MacroDef { .. } => {
						fail("macros can only be defined at the top level", e.1, "nested macro def")
					}
					_ => e.0.try_children(|c| self.expand(c, scope, depth)),
				}
			}
		}
	}
}

// Each module's items with macros expanded, and the stage-0 JIT that expanded them.
type Expansion = (HashMap<String, Vec<Spanned<Expr>>>, Option<Compiler>);

// Expand all macro calls across a program's modules.
pub fn expand(program: &Program) -> Result<Expansion, Diagnostic> {
	let mut ex = Expander {
		publics: program.publics.clone(),
		..Default::default()
	};
	let mut rest: HashMap<String, Vec<Spanned<Expr>>> = HashMap::new();
	for m in &program.modules {
		let mut items = Vec::with_capacity(m.items.len());
		for item in m.items.iter().cloned() {
			match item.0 {
				Expr::MacroDef {
					name,
					params,
					ret,
					body,
				} => ex.define(name, params, ret, body, item.1)?,
				_ => items.push(item),
			}
		}
		rest.insert(m.name.clone(), items);
	}
	let called = program.modules.iter().any(|m| {
		rest.get_mut(&m.name)
			.unwrap()
			.iter_mut()
			.any(|it| ex.calls_macro(&mut it.0, &m.scope))
	});
	if called {
		ex.compile_stage0(program, &mut rest)?;
	}
	let defs = program.items().map(|(_, i)| Expr::peel_meta(i).2);
	DEFS.set((
		Scope::default(),
		defs.filter_map(|d| Some((d.0.def_name()?.to_string(), d.clone()))).collect(),
	));
	for m in &program.modules {
		let items = rest.get_mut(&m.name).expect("every module was seeded above");
		ex.expand(List(items), &m.scope, 0)?;
		items.append(&mut ex.hoisted);
	}
	Ok((rest, ex.stage0))
}

// A quote template.
struct Template {
	stmts: Vec<Spanned<Expr>>,
	names: Vec<String>,
	bound: HashSet<String>,
}

static HYGIENE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
	static ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
	static DEFS: RefCell<(Scope, HashMap<String, Spanned<Expr>>)> = RefCell::default();
}

// Record a macro-run failure.
fn flag(msg: &str) {
	ERROR.with_borrow_mut(|e| {
		e.get_or_insert_with(|| msg.to_string());
	});
}

// One unquote site in a template.
pub(crate) enum Slot {
	Name(String),
	Expr(Spanned<Expr>),
	Splat(Spanned<Expr>),
}

// An instantiation argument.
// One Ast, or a splat's `[]Ast` elements.
enum Arg<'a> {
	Ast(&'a Spanned<Expr>),
	Seq(Vec<Spanned<Expr>>),
}

fn push_name(slots: &mut Vec<Slot>, n: &str) {
	if !slots.iter().any(|s| matches!(s, Slot::Name(m) if m == n)) {
		slots.push(Slot::Name(n.to_string()));
	}
}

// Walk a template body collecting unquote slots and binders, flagging nested quotes.
// Unquote expressions are pulled out and replaced with an `Unquote`.
fn scan(e: &mut Expr, slots: &mut Vec<Slot>, bound: &mut HashSet<String>, nested: &mut bool) {
	match e {
		Expr::Quote(_) => *nested = true,
		Expr::Unquote(n) => push_name(slots, n),
		Expr::UnquoteExpr(inner) | Expr::UnquoteSplat(inner) => {
			let taken = std::mem::replace(inner.as_mut(), (Expr::Unquote(String::new()), (0..0).into()));
			let (key, slot) = match e {
				Expr::UnquoteSplat(_) => (format!("...{}", slots.len()), Slot::Splat(taken)),
				_ => (slots.len().to_string(), Slot::Expr(taken)),
			};
			slots.push(slot);
			*e = Expr::Unquote(key);
		}
		Expr::UnquoteBind(binder, bind) => {
			let placeholder = (Expr::Unquote(String::new()), (0..0).into());
			if let Some(name) = def_name(&mut bind.0) {
				*name = format!("%{}", slots.len());
			}
			slots.push(Slot::Expr(std::mem::replace(binder.as_mut(), placeholder)));
			*e = std::mem::replace(&mut bind.0, Expr::Unquote(String::new()));
		}
		_ => {
			if let Some(n) = def_name(e).and_then(|n| n.strip_prefix('%')) {
				push_name(slots, n);
			}
			if let Expr::Fn { params, .. } | Expr::AnonFn { params, .. } = e {
				params
					.iter()
					.filter_map(|p| p.name.strip_prefix('%'))
					.for_each(|n| push_name(slots, n));
			}
		}
	}
	for_binders(e, &mut |n| {
		bound.insert(n.clone());
	});
	e.for_children(|c| match c {
		List(list) => list.iter_mut().for_each(|(e, _)| scan(e, slots, bound, nested)),
		One((e, _)) => scan(e, slots, bound, nested),
	});
}

// Validate a quote and leak it as a template.
pub(crate) fn register(stmts: &[Spanned<Expr>], span: Span) -> Result<(usize, Vec<Slot>), Diagnostic> {
	let mut stmts = stmts.to_vec();
	let (mut slots, mut bound, mut nested) = (Vec::new(), HashSet::new(), false);
	for (e, _) in &mut stmts {
		scan(e, &mut slots, &mut bound, &mut nested);
	}
	if nested {
		return fail("nested quotes aren't supported yet", span, "nested quote");
	}
	let names = slots
		.iter()
		.enumerate()
		.map(|(i, s)| match s {
			Slot::Name(n) => n.clone(),
			Slot::Expr(_) => i.to_string(),
			Slot::Splat(_) => format!("...{i}"),
		})
		.collect();
	let tpl = Template { names, stmts, bound };
	Ok((Box::into_raw(Box::new(tpl)) as usize, slots))
}

// One pass over a template copy.
// Adds a suffix to the template's own binders for good hygiene (see what I did there?).
// Splices `%name` arguments in verbatim.
fn fill(e: &mut Spanned<Expr>, bound: &HashSet<String>, args: &HashMap<&str, Arg>, suffix: usize) {
	if let Expr::Unquote(name) = &e.0 {
		match &args[name.as_str()] {
			Arg::Ast(v) => *e = (*v).clone(),
			Arg::Seq(_) => {
				flag("%{..} spread needs a sequence position");
				e.0 = Expr::Tuple(vec![]);
			}
		}
		return;
	}
	let rename = &mut |n: &mut String| {
		if bound.contains(n.as_str()) {
			*n = format!("{n}#{suffix}");
		}
	};
	for_binders(&mut e.0, rename);
	match &mut e.0 {
		Expr::Ident(n)
		| Expr::Assign { name: n, .. }
		| Expr::Call { name: n, .. }
		| Expr::FieldAssign { name: n, .. }
		| Expr::DerefAssign { name: n, .. }
		| Expr::IndexAssign { name: n, .. }
		| Expr::Append { name: n, .. }
		| Expr::MapDelete { name: n, .. } => rename(n),
		Expr::PatBind { pat, mutable: None, .. } => pat_names(pat).into_iter().for_each(rename),
		Expr::AnonFn {
			captures: Some(list), ..
		} => {
			for c in list {
				let (Capture::ReadOnly(n) | Capture::Mut(n) | Capture::Move(n)) = c;
				rename(n);
			}
		}
		_ => {}
	}
	if let Some(name) = def_name(&mut e.0)
		&& let Some(param) = name.strip_prefix('%')
	{
		match &args[param] {
			Arg::Ast((Expr::Ident(n), _)) => *name = n.clone(),
			_ => flag("a `%name` binder needs a plain name argument"),
		}
	}
	match &mut e.0 {
		Expr::Fn {
			params,
			params_tuple,
			ret,
			..
		}
		| Expr::AnonFn {
			params,
			params_tuple,
			ret,
			..
		} => {
			fill_sig(params, ret.as_mut(), args);
			*params_tuple |= params.len() != 1;
		}
		Expr::StructDef { fields, .. } => fill_sig(fields, None, args),
		_ => e.0.types().into_iter().for_each(|t| fill_type(t, args)),
	}
	match &mut e.0 {
		Expr::Call { args: list, .. }
		| Expr::MacroCall { args: list, .. }
		| Expr::EnumShorthand { args: list, .. }
		| Expr::Array(list)
		| Expr::DotArray(_, list)
		| Expr::DotTuple(list) => splice(list, bound, args, suffix),
		Expr::MethodCall { recv, args: list, .. } => {
			fill(recv, bound, args, suffix);
			splice(list, bound, args, suffix);
		}
		Expr::StructLit { fields, .. } if fields.iter().all(|(n, _)| n.is_none()) => {
			let mut list = fields.drain(..).map(|(_, v)| v).collect();
			splice(&mut list, bound, args, suffix);
			*fields = list.into_iter().map(|v| (None, v)).collect();
		}
		_ => e.0.for_children(|c| match c {
			List(list) => splice(list, bound, args, suffix),
			One(one) => fill(one, bound, args, suffix),
		}),
	}
	if let Expr::Match { arms, .. } = &mut e.0 {
		*arms = std::mem::take(arms)
			.into_iter()
			.flat_map(|a| match a.patterns.is_empty() {
				false => vec![a],
				true => a.body.into_iter().filter_map(|(e, _)| to_arm(e)).collect(),
			})
			.collect();
	}
	if let Expr::EnumDef { variants, fills, .. } = &mut e.0 {
		let spliced = fills.extract_if(.., |f| matches!(f.0, Expr::Ident(_) | Expr::Call { .. }));
		variants.extend(spliced.map(to_variant));
	}
}

// A `name: Type` param Ast.
fn to_param(hole: &Param, a: &Spanned<Expr>) -> Param {
	let mut p = hole.clone();
	let (notes, _, (e, _)) = Expr::peel_meta(a);
	if let Expr::Bind {
		name,
		typ: Some(t),
		value,
		..
	} = e
	{
		p.name = name.split('#').next().unwrap_or(name).into();
		(p.typ, p.default) = (t.0.clone(), value.as_deref().cloned());
		p.annotations.extend_from_slice(notes);
	} else {
		flag("a param hole needs a `name: Type` Ast");
	}
	p
}

// The slot a type hole names.
fn hole_key(t: &TypeExpr) -> Option<&str> {
	match t {
		TypeExpr::Unquote(e) if let Expr::Unquote(k) = &e.0 => Some(k),
		_ => None,
	}
}

// Fill every type hole with the type named by its argument.
fn fill_type(t: &mut TypeExpr, args: &HashMap<&str, Arg>) {
	t.walk_mut(&mut |t| {
		let Some(k) = hole_key(t) else { return };
		match &args[k] {
			Arg::Ast(a) if let Some(named) = TypeExpr::from_expr(&a.0) => *t = named,
			_ => flag("a type hole needs an Ast naming a type"),
		}
	});
}

// Fill a signature.
fn fill_sig(params: &mut Vec<Param>, ret: Option<&mut Spanned<TypeExpr>>, args: &HashMap<&str, Arg>) {
	let mut out = Vec::with_capacity(params.len());
	for mut p in params.drain(..) {
		match hole_key(&p.typ).filter(|_| p.name.is_empty()) {
			Some(k) => match &args[k] {
				Arg::Ast(a) => out.push(to_param(&p, a)),
				Arg::Seq(v) => out.extend(v.iter().map(|a| to_param(&p, a))),
			},
			None => {
				if let Some(k) = p.name.strip_prefix('%') {
					match &args[k] {
						Arg::Ast((Expr::Ident(n), _)) => p.name = n.clone(),
						_ => flag("a param name hole needs a plain name argument"),
					}
				}
				fill_type(&mut p.typ, args);
				out.push(p);
			}
		}
	}
	*params = out;
	ret.into_iter().for_each(|(t, _)| fill_type(t, args));
}

// A `Name` or `Name(Type)` Ast spliced into an enum body.
fn to_variant((e, span): Spanned<Expr>) -> EnumVariant {
	let (name, args) = match e {
		Expr::Call { name, args, .. } => (name, args),
		Expr::Ident(name) => (name, vec![]),
		_ => unreachable!(),
	};
	let payload: Vec<_> = args.iter().filter_map(|a| Some((TypeExpr::from_expr(&a.0)?, a.1))).collect();
	if payload.len() != args.len() {
		flag("an enum variant payload needs Asts naming types");
	}
	EnumVariant {
		name,
		span,
		payload,
		..Default::default()
	}
}

fn to_arm(e: Expr) -> Option<MatchArm> {
	match e {
		Expr::Arm(arm) => Some(arm),
		_ => {
			flag("a match arm spread needs `pattern => body` Asts");
			None
		}
	}
}

// Walk a sequence position, splicing `%{..expr}` slots in verbatim and filling everything else.
fn splice(list: &mut Vec<Spanned<Expr>>, bound: &HashSet<String>, args: &HashMap<&str, Arg>, suffix: usize) {
	let mut i = 0;
	while i < list.len() {
		if let Expr::Unquote(n) = &list[i].0
			&& let Arg::Seq(items) = &args[n.as_str()]
		{
			let items = items.clone();
			let n = items.len();
			list.splice(i..i + 1, items);
			i += n;
		} else {
			fill(&mut list[i], bound, args, suffix);
			i += 1;
		}
	}
}

// Instantiate the template at `tpl`, substituting `args` by position against its unquote names.
// `args` are borrowed, not owned.
pub(crate) extern "C" fn rt_quote(tpl: usize, args: *const *mut Spanned<Expr>, len: usize) -> *mut Spanned<Expr> {
	// SAFETY: `tpl` was leaked by `register`.
	let tpl = unsafe { &*(tpl as *const Template) };
	let args = if len == 0 {
		&[]
	} else {
		unsafe { std::slice::from_raw_parts(args, len) }
	};
	let suffix = HYGIENE.fetch_add(1, Ordering::Relaxed) + 1;
	let map: HashMap<&str, Arg> = tpl
		.names
		.iter()
		.zip(args)
		.map(|(n, &p)| {
			let arg = if n.starts_with("...") {
				// SAFETY: the lowerer passes a `[]Ast` header for splat slots
				let elems = unsafe { runtime::array_elems(p.cast()) };
				Arg::Seq(elems.iter().map(|&q| unsafe { (*(q as *mut Spanned<Expr>)).clone() }).collect())
			} else {
				Arg::Ast(unsafe { &*p })
			};
			(n.as_str(), arg)
		})
		.collect();
	let mut stmts = tpl.stmts.clone();
	splice(&mut stmts, &tpl.bound, &map, suffix);
	let span = stmts.first().map_or((0..0).into(), |s| s.1);
	Box::into_raw(Box::new(one(stmts, span)))
}

// Wrap loose stmts into a block when more than one.
pub(crate) fn one(mut stmts: Vec<Spanned<Expr>>, span: Span) -> Spanned<Expr> {
	match stmts.len() {
		1 => stmts.pop().unwrap(),
		_ => (Expr::Block(stmts), span),
	}
}

// Structurally match a template's pattern against `subject`, capturing `%name` holes.
// `outs` receives leaked pointers to captured subtrees.
pub(crate) extern "C" fn rt_quote_match(tpl: usize, subject: *mut Spanned<Expr>, outs: *mut *mut Spanned<Expr>) -> i64 {
	// SAFETY: `tpl` was leaked by `register`, `subject` by the caller's Ast value.
	let (tpl, subj) = unsafe { (&*(tpl as *const Template), &*subject) };
	let mut binds = Vec::new();
	if !unify(&tpl.stmts[0], subj, &mut binds) {
		return 0;
	}
	for (i, b) in binds.into_iter().enumerate() {
		unsafe { *outs.add(i) = Box::into_raw(Box::new(b)) };
	}
	1
}

// Take `e`'s child subtrees, leaving a placeholder.
fn split(e: &Expr) -> (String, Vec<Spanned<Expr>>) {
	let (mut e, mut kids) = (e.clone(), Vec::new());
	e.for_children(|c| {
		let list = match c {
			List(list) => list.as_mut_slice(),
			One(one) => std::slice::from_mut(one),
		};
		for one in list {
			kids.push(std::mem::replace(one, (Expr::Tuple(vec![]), Span::from(0..0))));
		}
	});
	(format!("{e:?}"), kids)
}

// Structurally unify a quote pattern against a runtime Ast, capturing `%name` holes.
fn unify(pat: &Spanned<Expr>, subj: &Spanned<Expr>, binds: &mut Vec<Spanned<Expr>>) -> bool {
	if matches!(pat.0, Expr::Unquote(_)) {
		binds.push(subj.clone());
		return true;
	}
	let ((ph, pk), (sh, sk)) = (split(&pat.0), split(&subj.0));
	ph == sh && pk.iter().zip(&sk).all(|(p, s)| unify(p, s, binds))
}

pub(crate) extern "C" fn rt_ast_lit(tag: i64, bits: i64) -> *mut Spanned<Expr> {
	Box::into_raw(Box::new((super::comp::scalar(tag, bits), (0..0).into())))
}

// Process symbols.

#[unsafe(export_name = "oi_ast_ident")]
pub(crate) extern "C" fn rt_ast_ident(s: *const runtime::StrHeader) -> *mut Spanned<Expr> {
	let name = String::from_utf8_lossy(unsafe { runtime::str_bytes(s) }).into_owned();
	Box::into_raw(Box::new((Expr::Ident(name), Span::from(0..0))))
}
#[unsafe(export_name = "oi_ast_gensym")]
pub(crate) extern "C" fn rt_ast_gensym(s: *const runtime::StrHeader) -> *mut Spanned<Expr> {
	let prefix = String::from_utf8_lossy(unsafe { runtime::str_bytes(s) });
	let n = HYGIENE.fetch_add(1, Ordering::Relaxed) + 1;
	Box::into_raw(Box::new((Expr::Ident(format!("{prefix}#{n}")), Span::from(0..0))))
}
#[unsafe(export_name = "oi_ast_parse")]
pub(crate) extern "C" fn rt_ast_parse(s: *const runtime::StrHeader) -> *mut Spanned<Expr> {
	let src = String::from_utf8_lossy(unsafe { runtime::str_bytes(s) }).into_owned();
	let node = match crate::loader::parse_file(&src, 0, &HashSet::new()) {
		Ok(stmts) => one(stmts, (0..0).into()),
		Err(ds) => {
			flag(ds.first().map_or("parse failed", Diagnostic::message));
			(Expr::Tuple(vec![]), Span::from(0..0))
		}
	};
	Box::into_raw(Box::new(node))
}
#[unsafe(export_name = "oi_ast_def")]
pub(crate) extern "C" fn rt_ast_def(a: *mut Spanned<Expr>) -> *mut Spanned<Expr> {
	let name = match unsafe { &(*a).0 } {
		Expr::Ident(n) => n.clone(),
		Expr::Field { tuple, field } if let Expr::Ident(m) = &tuple.0 => format!("{m}.{field}"),
		_ => String::new(),
	};
	let def = DEFS.with_borrow(|(scope, defs)| defs.get(&scope.qualify_name(&name)).cloned());
	Box::into_raw(Box::new(def.unwrap_or_else(|| {
		flag("`def` needs the name of a definition");
		(Expr::Tuple(vec![]), Span::from(0..0))
	})))
}

// A type as an Ast.
fn type_ast(t: &TypeExpr) -> Expr {
	match t {
		TypeExpr::Name(n) => Expr::Ident(n.clone()),
		t => Expr::TypePat(t.clone()),
	}
}

// A field as the Ast a param hole takes.
fn field_ast(f: &Param) -> Expr {
	let bind = Expr::Bind {
		mutable: false,
		name: f.name.clone(),
		typ: Some((f.typ.clone(), f.span)),
		value: f.default.clone().map(Box::new),
	};
	match f.annotations.is_empty() {
		true => bind,
		false => Expr::Annotated(f.annotations.clone(), Box::new((bind, f.span))),
	}
}

// Ast dispatch for the lowerer.
pub(crate) extern "C" fn rt_ast_method(a: *mut Spanned<Expr>, m: *const runtime::StrHeader, arg: i64) -> i64 {
	let ast = |e: Expr| Box::into_raw(Box::new((e, Span::from(0..0)))) as i64;
	let list = |ptrs: Vec<i64>| runtime::array_of(&ptrs, 8) as i64;
	let m = unsafe { runtime::str_bytes(m) };
	let (notes, _, (subject, _)) = Expr::peel_meta(unsafe { &*a });
	match (m, subject) {
		(b"notes", _) => list(notes.iter().map(|n| Box::into_raw(Box::new(n.clone())) as i64).collect()),
		(b"typ", Expr::Bind { typ: Some((t, _)), .. } | Expr::Fn { ret: Some((t, _)), .. }) => ast(type_ast(t)),
		(b"typ", Expr::TypePat(TypeExpr::Array(t) | TypeExpr::FixedArray(t, _))) => ast(type_ast(t)),
		(b"typ", Expr::Fn { ret: None, .. }) => ast(Expr::TypePat(TypeExpr::Tuple(vec![]))),
		(b"typ", _) => {
			flag("this Ast has no type");
			ast(Expr::Tuple(vec![]))
		}
		(b"len", Expr::TypePat(TypeExpr::FixedArray(_, n))) => ast(n.0.clone()),
		(b"int", Expr::Int(n)) => *n,
		(b"int", _) => {
			flag("`.int()` needs an Ast holding an Int literal");
			0
		}
		// the variant name, lowercased
		(b"kind", e) => ast(Expr::Ident(
			format!("{e:?}").split(['(', ' ']).next().unwrap().to_lowercase(),
		)),
		(b"str", e) => {
			let s = match e {
				Expr::Ident(s) | Expr::String(s) => s.clone(),
				Expr::Int(n) => n.to_string(),
				Expr::Float(n) => n.to_string(),
				Expr::Bool(b) => b.to_string(),
				_ => {
					flag("`.str()` needs an Ast holding a name or literal");
					String::new()
				}
			};
			runtime::str_new(runtime::system_allocator(), s.as_bytes()) as i64
		}
		(
			b"name",
			Expr::StructDef { name, .. }
			| Expr::EnumDef { name, .. }
			| Expr::Call { name, .. }
			| Expr::MacroCall { name, .. }
			| Expr::Bind { name, .. }
			| Expr::Fn { name, .. }
			| Expr::Claim { typ: name, .. }
			| Expr::StructLit { name, .. },
		) => ast(Expr::Ident(name.clone())),
		(b"name", ident @ Expr::Ident(_)) => ast(ident.clone()),
		(b"name", _) => {
			flag("this Ast has no name");
			ast(Expr::Tuple(vec![]))
		}
		(
			b"items",
			Expr::Array(v)
			| Expr::DotArray(_, v)
			| Expr::Block(v)
			| Expr::Call { args: v, .. }
			| Expr::MacroCall { args: v, .. }
			| Expr::Claim { fills: v, .. },
		) => list(v.iter().map(|e| Box::into_raw(Box::new(e.clone())) as i64).collect()),
		(b"items", Expr::StructDef { fields, .. } | Expr::Fn { params: fields, .. }) => {
			list(fields.iter().map(|f| ast(field_ast(f))).collect())
		}
		(b"items", Expr::StructLit { fields, .. }) => {
			list(fields.iter().map(|(_, v)| Box::into_raw(Box::new(v.clone())) as i64).collect())
		}
		(b"items", Expr::EnumDef { variants, .. }) => {
			list(variants.iter().map(|v| ast(Expr::Ident(v.name.clone()))).collect())
		}
		(b"items", _) => {
			flag("this Ast has no items");
			list(vec![])
		}
		(b"fills", Expr::StructDef { fills, .. } | Expr::EnumDef { fills, .. } | Expr::Claim { fills, .. }) => {
			list(fills.iter().map(|e| Box::into_raw(Box::new(e.clone())) as i64).collect())
		}
		(b"fills", _) => {
			flag("this Ast has no fills");
			list(vec![])
		}
		(b"==", Expr::Ident(n)) => {
			let bytes = unsafe { runtime::str_bytes(arg as *const runtime::StrHeader) };
			(n.as_bytes() == bytes) as i64
		}
		(b"==", _) => 0,
		_ => {
			flag("unknown Ast method");
			ast(Expr::Tuple(vec![]))
		}
	}
}
