//! Type resolution.

use super::*;
use crate::loader::{Scope, fold_const};

// resolved params with an optional return annotation
type ParamsRet = (Vec<(String, Typ, Access)>, Option<(Typ, Span)>);

// Assign discriminants and resolve payload types against `types`.
pub(super) fn build_variants(variants: &[EnumVariant], types: TypeCtx) -> Result<Vec<VariantInfo>, Diagnostic> {
	let mut next = 0;
	let mut seen = HashSet::new();
	variants
		.iter()
		.map(|v| {
			let disc = match &v.disc {
				Some((e, span)) => match fold_const(e, types.consts.map, types.scope) {
					Some(Expr::Int(n)) => n,
					_ => {
						return fail("a discriminant must be a constant int", *span, "not a constant int");
					}
				},
				None => next,
			};
			if !seen.insert(disc) {
				let msg = format!("discriminant value `{disc}` assigned more than once");
				return fail(msg, v.span, "already taken");
			}
			next = disc + 1;
			let payload = v
				.payload
				.iter()
				.map(|(te, span)| types.resolve(te, *span))
				.collect::<Result<Vec<_>, _>>()?;
			Ok(VariantInfo {
				name: v.name.clone(),
				disc,
				raw: v.raw.clone(),
				payload,
				names: v.names.clone(),
				backing: None,
			})
		})
		.collect()
}

// Resolve and validate an enum backing.
pub(super) fn apply_backing(
	backing: &Spanned<TypeExpr>,
	variants: &mut [VariantInfo],
	ast: &[EnumVariant],
	types: TypeCtx,
) -> Result<(), Diagnostic> {
	let (te, span) = (&backing.0, backing.1);
	let bt = types.resolve(te, span)?;
	if variants.iter().any(|v| !v.payload.is_empty()) {
		return fail(
			"a backed enum cannot have payload variants",
			span,
			"payloads exclude a backing",
		);
	}
	if bt != Typ::Str && variants.iter().any(|v| v.raw.is_some()) {
		return fail("a raw value needs a string backing", span, "not a string backing");
	}
	if bt == Typ::Str {
		if ast.iter().any(|a| a.disc.is_some()) {
			return fail(
				"a string-backed enum uses raw values, not discriminants",
				span,
				"not a raw value",
			);
		}
		// raws default to the variant name at the use site
		let raws: Vec<_> = variants.iter().map(|v| v.raw.as_ref().unwrap_or(&v.name)).collect();
		if let Some(r) = raws.iter().enumerate().find_map(|(i, r)| raws[..i].contains(r).then_some(*r)) {
			return fail(
				format!("raw value `{r}` assigned more than once"),
				span,
				"duplicate raw value",
			);
		}
	} else {
		let (lo, hi) = match &bt {
			Typ::Int(w) if *w < 64 => (-(1i64 << (w - 1)), (1i64 << (w - 1)) - 1),
			Typ::Int(_) | Typ::ISize => (i64::MIN, i64::MAX),
			Typ::UInt(w) if *w < 64 => (0, (1i64 << w) - 1),
			Typ::UInt(_) | Typ::USize => (0, i64::MAX),
			t => {
				// TODO: come up with a better label
				return fail(
					format!("enum backing type `{t}` is unsupported"),
					span,
					"not an enum-able type",
				);
			}
		};
		if let Some(v) = variants.iter().find(|v| v.disc < lo || v.disc > hi) {
			return fail(
				format!("discriminant `{}` is out of range for its backing type", v.disc),
				span,
				"out of range",
			);
		}
	}
	for v in variants.iter_mut() {
		v.backing = Some(bt.clone());
	}
	Ok(())
}

static NO_SCOPE: std::sync::LazyLock<Scope> = std::sync::LazyLock::new(Scope::default);
type ConstMaps = (HashMap<String, Spanned<Expr>>, HashMap<String, Vec<Annotation>>);
static NO_CONSTS: std::sync::LazyLock<ConstMaps> = std::sync::LazyLock::new(ConstMaps::default);

// What a const expression in a type can name.
#[derive(Clone, Copy)]
pub(crate) struct Consts<'a> {
	pub map: &'a HashMap<String, Spanned<Expr>>,
	pub anns: &'a HashMap<String, Vec<Annotation>>,
}

// The named types in scope for resolution.
#[derive(Clone, Copy)]
pub(crate) struct TypeCtx<'a> {
	pub structs: &'a HashMap<String, Vec<FieldDef>>,
	pub enums: &'a RefCell<HashMap<String, Vec<VariantInfo>>>,
	pub aliases: &'a HashMap<String, TypeExpr>,
	pub type_params: &'a HashMap<String, Typ>,
	pub generics: &'a Generics,
	pub traits: &'a HashMap<&'a str, TraitItem<'a>>,
	pub consts: Consts<'a>,
	pub scope: &'a Scope,
	// keep track of generic instantiations to catch recursion
	depth: usize,
}

impl<'a> TypeCtx<'a> {
	pub fn new(
		structs: &'a HashMap<String, Vec<FieldDef>>,
		enums: &'a RefCell<HashMap<String, Vec<VariantInfo>>>,
		aliases: &'a HashMap<String, TypeExpr>,
		type_params: &'a HashMap<String, Typ>,
		generics: &'a Generics,
		traits: &'a HashMap<&'a str, TraitItem<'a>>,
	) -> Self {
		TypeCtx {
			structs,
			enums,
			aliases,
			type_params,
			generics,
			traits,
			consts: Consts {
				map: &NO_CONSTS.0,
				anns: &NO_CONSTS.1,
			},
			scope: &NO_SCOPE,
			depth: 0,
		}
	}

	// Resolve names through a module's scope.
	pub fn with_scope(self, scope: &'a Scope) -> Self {
		TypeCtx { scope, ..self }
	}

	pub fn with_aliases(self, aliases: &'a HashMap<String, TypeExpr>) -> Self {
		TypeCtx { aliases, ..self }
	}

	// Const folding.
	pub fn with_consts(self, consts: Consts<'a>) -> Self {
		TypeCtx { consts, ..self }
	}

	pub fn field(&self, p: &Param) -> Result<FieldDef, Diagnostic> {
		Ok(FieldDef {
			name: p.name.clone(),
			typ: self.resolve(&p.typ, p.span)?,
			default: p.default.clone(),
			embedded: embedded(p),
			annotations: qualify_anns(self.scope, &p.annotations),
		})
	}

	// Named types with a C layout.
	pub fn is_c(&self, name: &str) -> bool {
		is_c_struct(self.consts.anns, name)
			|| (self.enums.borrow().get(name)).is_some_and(|vs| !vs.is_empty() && !enum_boxed(vs))
	}

	// A generic instance's substitution.
	pub fn with_type_params(self, type_params: &'a HashMap<String, Typ>) -> Self {
		TypeCtx { type_params, ..self }
	}
}

// The fixed-name builtin types.
fn builtin(name: &str) -> Option<Typ> {
	Some(match name {
		"int" => Typ::Int(64),
		"uint" => Typ::UInt(64),
		"isize" => Typ::ISize,
		"usize" => Typ::USize,
		"float" => Typ::Float(64),
		"rune" => Typ::Rune,
		"bool" => Typ::Bool,
		"string" => Typ::Str,
		"cstr" => Typ::CStr,
		"atom" => Typ::Atom,
		"any" => Typ::Any,
		"()" => Typ::unit(),
		"Error" => Typ::Error,
		"Ast" => Typ::Ast,
		_ => return None,
	})
}

// Try to parse `name` as `<prefix><width>`.
fn int_width(
	name: &str,
	prefix: char,
	ctor: fn(u16) -> Typ,
	label: &str,
	span: Span,
) -> Option<Result<Typ, Diagnostic>> {
	let rest = name.strip_prefix(prefix)?;
	let w = rest.parse::<u16>().ok()?;
	if w == 0 || w > 64 {
		return Some(fail(
			format!("{label} width {w} out of range"),
			span,
			"width must be 1-64",
		));
	}
	Some(Ok(ctor(w)))
}

// nested generic instantiations allowed before calling it recursive
const MAX_GENERIC_DEPTH: usize = 64;

impl TypeCtx<'_> {
	// Resolve a type expression to a concrete `Typ`.
	pub fn resolve(&self, te: &TypeExpr, span: Span) -> Result<Typ, Diagnostic> {
		match te {
			TypeExpr::Name(name) => self.named(name, span),
			TypeExpr::Tuple(elems) => {
				let fields = elems
					.iter()
					.map(|(n, e)| Ok((n.clone(), self.resolve(e, span)?)))
					.collect::<Result<Vec<_>, _>>()?;
				Ok(Typ::Tuple(fields))
			}
			TypeExpr::Variadic(_) => fail("`..T` is only allowed as a parameter type", span, "not a parameter"),
			TypeExpr::Unquote(_) => fail("unquote outside a macro template", span, "stray unquote"),
			TypeExpr::Infer(e) => super::static_typ(&e.0, self, e.1),
			TypeExpr::Array(elem) => Ok(Typ::Array(Box::new(self.resolve(elem, span)?))),
			TypeExpr::Const(n) => Ok(Typ::Const(*n)),
			TypeExpr::FixedArray(elem, len) => {
				if let Expr::Ident(name) = &len.0
					&& let Ok(k) = self.named(name, span)
				{
					let elem = Box::new(self.resolve(elem, span)?);
					return match k {
						Typ::Const(n) => Ok(Typ::FixedArray(elem, n as usize)),
						_ => Ok(Typ::Map(Box::new(k), elem)),
					};
				}
				Ok(Typ::FixedArray(
					Box::new(self.resolve(elem, span)?),
					self.array_len(len)?,
				))
			}
			TypeExpr::Ref(inner) => Ok(Typ::Ref(Box::new(self.resolve(inner, span)?))),
			TypeExpr::AtomSum(names) => {
				let mut seen = HashSet::new();
				if let Some(dup) = names.iter().find(|n| !seen.insert(*n)) {
					return fail(format!("duplicate atom `:{dup}` in sum type"), span, "repeated atom");
				}
				Ok(Typ::Sum(String::new(), atom_sum_variants(names)))
			}
			TypeExpr::Sum(ms) => self.resolve_sum(ms, span),
			TypeExpr::AnonStruct(params) => {
				let fields = params.iter().map(|p| self.field(p)).collect::<Result<Vec<_>, _>>()?;
				let shape: Vec<_> = fields.iter().map(|f| format!("{}: {}", f.name, f.typ.key())).collect();
				Ok(Typ::Struct(format!("struct{{{}}}", shape.join(", ")), fields))
			}
			TypeExpr::TupleStruct(name, fields) => {
				let fields = fields
					.iter()
					.map(|(n, te)| Ok((n.clone(), self.resolve(te, span)?)))
					.collect::<Result<_, Diagnostic>>()?;
				Ok(Typ::TupleStruct(name.clone(), fields))
			}
			TypeExpr::Fn(params, ret) => {
				let params = params
					.iter()
					.map(|(n, a, p)| {
						Ok(FnParam {
							name: n.clone(),
							variadic: matches!(p, TypeExpr::Variadic(_)),
							..FnParam::new(access_wrap(*a, self.param(p, span)?))
						})
					})
					.collect::<Result<_, Diagnostic>>()?;
				Ok(Typ::Fn(params, Box::new(self.resolve(ret, span)?)))
			}
			TypeExpr::Annotated(anns, inner) => {
				let (names, inner) = (ann_names(self.scope, anns), self.resolve(inner, span)?);
				check_ann_typ(*self, &names, &inner, span)?;
				Ok(Typ::Annotated(names, Box::new(inner)))
			}
			TypeExpr::Map(k, v) => Ok(Typ::Map(
				Box::new(self.resolve(k, span)?),
				Box::new(self.resolve(v, span)?),
			)),
			TypeExpr::Generic(name, args) => {
				let name = self.scope.env.get(name).unwrap_or(name);
				if let Some(def) = self.generics.structs.get(name) {
					let subst = self.generic_subst(name, &def.type_params, args, span)?;
					return self.instantiate(name, def, &subst, span);
				}
				if let Some(def) = self.generics.enums.get(name) {
					let subst = self.generic_subst(name, &def.type_params, args, span)?;
					return self.instantiate_enum(name, def, &subst, span);
				}
				if let Some((params, te)) = self.generics.aliases.get(name) {
					let type_params = &self.generic_subst(name, params, args, span)?;
					let inner = TypeCtx { type_params, ..*self };
					return Ok(match inner.resolve(te, span)? {
						Typ::TupleStruct(_, fields) => {
							let keys: Vec<_> = params.iter().map(|p| type_params[&p.name].key()).collect();
							Typ::TupleStruct(format!("{name}[{}]", keys.join(", ")), fields)
						}
						typ => typ,
					});
				}
				let msg = match self.structs.contains_key(name) || self.enums.borrow().contains_key(name) {
					true => format!("`{name}` is not generic"),
					false => format!("unknown type `{name}`"),
				};
				fail(msg, span, "no type arguments expected here")
			}
		}
	}

	// Fold a fixed-array length.
	fn array_len(&self, (e, span): &Spanned<Expr>) -> Result<usize, Diagnostic> {
		if let Expr::Ident(path) = e
			&& let Some((name, "size")) = path.split_once('.')
			&& let Ok(t) = self.named(name, *span)
			&& let Some((size, _)) = t.c_size_align(self)
		{
			return Ok(size as usize);
		}
		match fold_const(e, self.consts.map, self.scope) {
			Some(Expr::Int(n)) if n >= 0 => Ok(n as usize),
			_ => fail("an array length must be a constant", *span, "not a constant int"),
		}
	}

	// A bound that names a type rather than a trait makes the param a value.
	pub fn value_param(&self, p: &TypeParam) -> bool {
		p.bound.as_deref().is_some_and(|b| {
			!self.traits.contains_key(b)
				&& (self.named(b, (0..0).into()).is_ok() || Self::builtin_type(b.rsplit("::").next().unwrap_or(b)))
		})
	}

	// Check that value params take literals and type param take types.
	pub fn check_arg(&self, p: &TypeParam, typ: &Typ, span: Span) -> Result<(), Diagnostic> {
		let want = self.value_param(p);
		if want == matches!(typ, Typ::Const(_)) {
			return Ok(());
		}
		let kind = if want { "value" } else { "type" };
		fail(
			format!("`{}` is a {kind} parameter", p.name),
			span,
			format!("got `{typ}`"),
		)
	}

	// Resolve one bracket argument.
	pub fn arg_typ(&self, p: &TypeParam, te: &TypeExpr, span: Span) -> Result<Typ, Diagnostic> {
		let typ = match self.resolve(te, span) {
			Ok(typ) => typ,
			Err(e) => match te {
				TypeExpr::Name(n) if self.value_param(p) => {
					match fold_const(&Expr::Ident(n.clone()), self.consts.map, self.scope) {
						Some(Expr::Int(v)) => Typ::Const(v),
						_ => return Err(e),
					}
				}
				_ => return Err(e),
			},
		};
		self.check_arg(p, &typ, span)?;
		Ok(typ)
	}

	// Resolve `args` against `params`.
	fn generic_subst(
		&self,
		name: &str,
		params: &[TypeParam],
		args: &[TypeExpr],
		span: Span,
	) -> Result<HashMap<String, Typ>, Diagnostic> {
		if args.len() != params.len() {
			return arity_err(&format!("`{name}`"), params.len(), args.len(), "type argument", span);
		}
		let mut subst = HashMap::new();
		for (param, arg) in params.iter().zip(args) {
			subst.insert(param.name.clone(), self.arg_typ(param, arg, span)?);
		}
		Ok(subst)
	}

	// Move one level deeper into a self-referential type, failing past the limit.
	fn nested(&self, name: &str, span: Span, label: &str) -> Result<Self, Diagnostic> {
		if self.depth > MAX_GENERIC_DEPTH {
			return fail(format!("`{name}` recurses without end"), span, label);
		}
		Ok(TypeCtx {
			depth: self.depth + 1,
			..*self
		})
	}

	// Record an instance's type args under its display name.
	fn register_instance(&self, name: &str, params: &[TypeParam], subst: &HashMap<String, Typ>) -> String {
		let concrete: Vec<Typ> = params.iter().map(|p| subst[&p.name].clone()).collect();
		let args: Vec<_> = concrete.iter().map(Typ::key).collect();
		let display = format!("{name}[{}]", args.join(", "));
		self.generics
			.instance_args
			.borrow_mut()
			.entry(display.clone())
			.or_insert(concrete);
		display
	}

	// Substitute `subst` into a generic struct's fields, yielding an ordinary `Typ::Struct`.
	pub fn instantiate(
		&self,
		name: &str,
		def: &GenericStructDef,
		subst: &HashMap<String, Typ>,
		span: Span,
	) -> Result<Typ, Diagnostic> {
		let inner = self
			.nested(name, span, "would require infinitely nested fields")?
			.with_type_params(subst);
		let fields = def.fields.iter().map(|f| inner.field(f)).collect::<Result<Vec<_>, _>>()?;
		Ok(Typ::Struct(
			self.register_instance(name, &def.type_params, subst),
			fields,
		))
	}

	// Substitute `subst` into a generic enum's variants.
	pub fn instantiate_enum(
		&self,
		name: &str,
		def: &GenericEnumDef,
		subst: &HashMap<String, Typ>,
		span: Span,
	) -> Result<Typ, Diagnostic> {
		let inner = self
			.nested(name, span, "would require infinitely nested variants")?
			.with_type_params(subst);
		let display = self.register_instance(name, &def.type_params, subst);
		if self.enums.borrow().contains_key(&display) {
			return Ok(Typ::Enum(display));
		}
		// name-only first, so a self-referential payload resolves instead of recursing
		self.enums.borrow_mut().insert(display.clone(), Vec::new());
		let variants = build_variants(&def.variants, inner)?;
		self.enums.borrow_mut().insert(display.clone(), variants);
		Ok(Typ::Enum(display))
	}

	// Type names owned by the compiler.
	pub fn builtin_type(name: &str) -> bool {
		builtin(name).is_some()
			|| matches!(name, "array" | "map")
			|| name.strip_prefix(['i', 'u', 'f']).is_some_and(|w| w.parse::<u16>().is_ok())
	}

	// An instance of a core generic enum.
	pub fn core_enum(&self, name: &str, args: &[Typ]) -> Typ {
		let def = self.generics.enums.get(name).expect("core declares it");
		let subst = (def.type_params.iter().zip(args))
			.map(|(p, a)| (p.name.clone(), a.clone()))
			.collect();
		(self.instantiate_enum(name, def, &subst, Span::default()))
			.unwrap_or_else(|_| unreachable!("{name} does not recurse"))
	}

	// The type args of a core generic enum instance.
	fn instance_of(&self, typ: &Typ, base: &str) -> Option<Vec<Typ>> {
		match typ {
			Typ::Enum(n) if n.split('[').next() == Some(base) => self.generics.instance_args(n),
			_ => None,
		}
	}

	pub fn option_inner(&self, typ: &Typ) -> Option<Typ> {
		self.instance_of(typ, role::OPTION)?.pop()
	}

	pub fn result_parts(&self, typ: &Typ) -> Option<(Typ, Typ)> {
		let [ok, err] = <[Typ; 2]>::try_from(self.instance_of(typ, role::RESULT)?).ok()?;
		Some((ok, err))
	}

	// The `some`/`ok` payload type.
	pub fn happy(&self, typ: &Typ) -> Option<Typ> {
		self.option_inner(typ).or_else(|| Some(self.result_parts(typ)?.0))
	}

	// Whether a fn specifies a Result.
	pub fn fallible(&self, typ: &Typ) -> bool {
		self.result_parts(typ).is_some_and(|(ok, _)| ok.is_unit())
	}

	// Resolve a named type.
	pub fn named(&self, name: &str, span: Span) -> Result<Typ, Diagnostic> {
		if let Some(typ) = self.type_params.get(name) {
			return Ok(typ.clone());
		}
		if name == "$?" {
			// an omitted param type that no expected fn type filled in
			return fail("parameter needs a type", span, "nothing here supplies one");
		}
		if let Some((m, t)) = name.split_once('.').filter(|_| !name.contains("::")) {
			let Some(vis) = self.scope.visible.get(m) else {
				return fail(format!("unknown module `{m}`"), span, "not imported");
			};
			let Some(t) = vis.only.as_ref().map_or(Some(t), |only| only.get(t).map(String::as_str)) else {
				return fail(format!("`{name}` is not part of `{m}`"), span, "not in this import");
			};
			return self.named(&format!("{}::{t}", vis.module), span);
		}
		if let Some(typ) = builtin(name) {
			return Ok(typ);
		}
		if let Some(result) = int_width(name, 'i', Typ::Int, "integer", span) {
			return result;
		}
		if let Some(result) = int_width(name, 'u', Typ::UInt, "unsigned integer", span) {
			return result;
		}
		if let Some(rest) = name.strip_prefix('f')
			&& let Ok(w) = rest.parse::<u16>()
		{
			return match w {
				16 => Ok(Typ::Float(16)),
				32 => Ok(Typ::Float(32)),
				64 => Ok(Typ::Float(64)),
				128 => Ok(Typ::Float(128)),
				_ => fail(
					format!("unsupported float width f{w}"),
					span,
					"supported widths: f16, f32, f64, f128",
				),
			};
		}
		let name = match self.scope.env.get(name) {
			Some(q) => q.as_str(),
			None if !self.scope.module.is_empty() && name != "Self" && !name.contains("::") => {
				return fail(format!("unknown type `{name}`"), span, "not a known type");
			}
			_ => name,
		};
		if let Some(te) = self.aliases.get(name) {
			if matches!(te, TypeExpr::Sum(_) | TypeExpr::AtomSum(_)) {
				if self.depth == 0 {
					self.named_sum(name, span)?;
				}
				return Ok(Typ::Sum(name.to_string(), vec![]));
			}
			return self.resolve(te, span);
		}
		if let Some(fields) = self.structs.get(name) {
			return Ok(Typ::Struct(name.to_string(), fields.clone()));
		}
		if self.enums.borrow().contains_key(name) {
			return Ok(Typ::Enum(name.to_string()));
		}
		if self.generics.structs.contains_key(name) || self.generics.enums.contains_key(name) {
			return fail(
				format!("`{name}` needs type arguments"),
				span,
				format!("try `{name}[...]`"),
			);
		}
		if self.traits.contains_key(name) {
			return Ok(Typ::Trait(name.to_string()));
		}
		fail(format!("unknown type `{name}`"), span, "not a known type")
	}

	// A named sum's members.
	pub fn named_sum(&self, name: &str, span: Span) -> Result<Vec<VariantInfo>, Diagnostic> {
		Ok(
			match self.nested(name, span, "splices itself")?.resolve(&self.aliases[name], span)? {
				Typ::Sum(_, vs) => vs,
				_ => vec![],
			},
		)
	}

	// Resolve a sum type.
	fn resolve_sum(&self, members: &[TypeExpr], span: Span) -> Result<Typ, Diagnostic> {
		let mut variants: Vec<VariantInfo> = Vec::with_capacity(members.len());
		for m in members {
			match m {
				TypeExpr::AtomSum(a) if a.len() == 1 => variants.push(VariantInfo::new(a[0].clone(), 0, vec![])),
				_ => match self.resolve(m, span)? {
					Typ::Sum(n, inner) if inner.is_empty() => variants.extend(self.named_sum(&n, span)?),
					Typ::Sum(_, inner) => variants.extend(inner),
					t => variants.push(VariantInfo::new(t.to_string(), 0, vec![t])),
				},
			}
		}
		let mut seen = HashSet::new();
		for (disc, v) in variants.iter_mut().enumerate() {
			v.disc = disc as i64;
			if !seen.insert(v.name.clone()) {
				let msg = format!("duplicate member `{}` in sum type", v.name);
				return fail(msg, span, "repeated member");
			}
		}
		Ok(Typ::Sum(String::new(), variants))
	}

	// A param type, mapping varargs to its array type.
	pub fn param(&self, te: &TypeExpr, span: Span) -> Result<Typ, Diagnostic> {
		match te {
			TypeExpr::Variadic(elem) => Ok(Typ::Array(Box::new(self.resolve(elem, span)?))),
			_ => self.resolve(te, span),
		}
	}

	pub fn resolve_params(&self, params: &[Param]) -> Result<Vec<(String, Typ, Access)>, Diagnostic> {
		params
			.iter()
			.map(|p| {
				check_reserved(&p.name, p.span)?;
				let typ = self.param(&p.typ, p.span)?;
				let lendable = matches!(
					typ,
					Typ::Int(_)
						| Typ::UInt(_) | Typ::ISize
						| Typ::USize | Typ::Float(_)
						| Typ::Bool | Typ::Rune
						| Typ::Array(_) | Typ::FixedArray(..)
						| Typ::Map(..) | Typ::Struct(..)
						| Typ::TupleStruct(..)
				) && typ.newtype().is_none();
				if p.access == Access::Mut && !lendable {
					return fail(
						"`mut` parameters must be scalars, arrays, maps, or structs for now",
						p.span,
						format!("{typ} has no address to lend"),
					);
				}
				Ok((p.name.clone(), typ, p.access))
			})
			.collect()
	}

	// Resolve a param list and optional return type annotation.
	pub fn resolve_params_ret(
		&self,
		params: &[Param],
		ret: &Option<Spanned<TypeExpr>>,
	) -> Result<ParamsRet, Diagnostic> {
		let params = self.resolve_params(params)?;
		let ret = ret
			.as_ref()
			.map(|(te, span)| Ok::<_, Diagnostic>((self.resolve(te, *span)?, *span)))
			.transpose()?;
		Ok((params, ret))
	}
}
