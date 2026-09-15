//! Type resolution.

use super::*;
use crate::loader::{Scope, fold_const};

// resolved params with an optional return annotation
type ParamsRet = (Vec<(String, Typ, Access)>, Option<(Typ, Span)>);

// Assign discriminants and resolve payload types against `types`.
pub(super) fn build_variants(variants: &[EnumVariant], types: TypeCtx) -> Result<Vec<VariantInfo>, Diagnostic> {
	let mut next = 0;
	variants
		.iter()
		.map(|v| {
			let disc = v.disc.unwrap_or(next);
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
	let (te, span) = backing;
	let err = |msg: String, label| Err(Diagnostic::new(msg, span.into_range()).with_label(label));
	let bt = types.resolve(te, *span)?;
	if variants.iter().any(|v| !v.payload.is_empty()) {
		return err(
			"a backed enum cannot have payload variants".into(),
			"payloads exclude a backing",
		);
	}
	if bt != Typ::Str && variants.iter().any(|v| v.raw.is_some()) {
		return err("a raw value needs a string backing".into(), "not a string backing");
	}
	if bt == Typ::Str {
		if ast.iter().any(|a| a.disc.is_some()) {
			return err(
				"a string-backed enum uses raw values, not discriminants".into(),
				"not a raw value",
			);
		}
		// raws default to the variant name at the use site
		let raws: Vec<_> = variants.iter().map(|v| v.raw.as_ref().unwrap_or(&v.name)).collect();
		if let Some(r) = raws.iter().enumerate().find_map(|(i, r)| raws[..i].contains(r).then_some(*r)) {
			return err(
				format!("raw value `{r}` assigned more than once"),
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
				return err(
					format!("enum backing type `{t}` is unsupported"),
					// TODO: come up with a better label
					"not an enum-able type",
				);
			}
		};
		if let Some(v) = variants.iter().find(|v| v.disc < lo || v.disc > hi) {
			return err(
				format!("discriminant `{}` is out of range for its backing type", v.disc),
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
	pub enums: &'a HashMap<String, Vec<VariantInfo>>,
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
		enums: &'a HashMap<String, Vec<VariantInfo>>,
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

	// Const folding.
	pub fn with_consts(self, consts: Consts<'a>) -> Self {
		TypeCtx { consts, ..self }
	}
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
		return Some(Err(Diagnostic::new(
			format!("{label} width {w} out of range"),
			span.into_range(),
		)
		.with_label("width must be 1-64")));
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
			TypeExpr::Variadic(_) => Err(Diagnostic::new(
				"`..T` is only allowed as a parameter type",
				span.into_range(),
			)
			.with_label("not a parameter")),
			TypeExpr::Unquote(_) => {
				Err(Diagnostic::new("unquote outside a macro template", span.into_range()).with_label("stray unquote"))
			}
			TypeExpr::Array(elem) => Ok(Typ::Array(Box::new(self.resolve(elem, span)?))),
			TypeExpr::FixedArray(elem, len) => {
				if let Expr::Ident(name) = &len.0
					&& let Ok(k) = self.named(name, span)
				{
					return Ok(Typ::Map(Box::new(k), Box::new(self.resolve(elem, span)?)));
				}
				Ok(Typ::FixedArray(
					Box::new(self.resolve(elem, span)?),
					self.array_len(len)?,
				))
			}
			TypeExpr::Option(inner) => Ok(Typ::Option(Box::new(self.resolve(inner, span)?))),
			TypeExpr::Ref(inner) => {
				let it = self.resolve(inner, span)?;
				if !matches!(it, Typ::Struct(..)) {
					return Err(Diagnostic::new(
						format!("references to {it} aren't supported yet, only structs"),
						span.into_range(),
					)
					.with_label("not a struct"));
				}
				Ok(Typ::Ref(Box::new(it)))
			}
			TypeExpr::Result(inner, err) => {
				let err = match err {
					Some(e) => self.resolve(e, span)?,
					None => Typ::Error,
				};
				Ok(Typ::Result(Box::new(self.resolve(inner, span)?), Box::new(err)))
			}
			TypeExpr::AtomSum(names) => {
				let mut seen = HashSet::new();
				if let Some(dup) = names.iter().find(|n| !seen.insert(*n)) {
					return Err(
						Diagnostic::new(format!("duplicate atom `:{dup}` in sum type"), span.into_range())
							.with_label("repeated atom"),
					);
				}
				Ok(Typ::Sum(String::new(), atom_sum_variants(names)))
			}
			TypeExpr::Sum(ms) => self.resolve_sum(ms, span),
			TypeExpr::AnonStruct(params) => {
				let fields = params
					.iter()
					.map(|p| {
						Ok(FieldDef {
							name: p.name.clone(),
							typ: self.resolve(&p.typ, p.span)?,
							default: p.default.clone(),
							embedded: embedded(p),
							annotations: qualify_anns(self.scope, &p.annotations),
						})
					})
					.collect::<Result<Vec<_>, Diagnostic>>()?;
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
				check_ann_typ(&names, &inner, span)?;
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
				let msg = match self.structs.contains_key(name) || self.enums.contains_key(name) {
					true => format!("`{name}` is not generic"),
					false => format!("unknown type `{name}`"),
				};
				Err(Diagnostic::new(msg, span.into_range()).with_label("no type arguments expected here"))
			}
		}
	}

	// Fold a fixed-array length.
	fn array_len(&self, (e, span): &Spanned<Expr>) -> Result<usize, Diagnostic> {
		if let Expr::Ident(path) = e
			&& let Some((name, "size")) = path.split_once('.')
			&& let Ok(t) = self.named(name, *span)
			&& let Some((size, _)) = t.c_size_align(&|n: &str| is_c_struct(self.consts.anns, n))
		{
			return Ok(size as usize);
		}
		match fold_const(e, self.consts.map, self.scope) {
			Some(Expr::Int(n)) if n >= 0 => Ok(n as usize),
			_ => Err(Diagnostic::new("an array length must be a constant", span.into_range())
				.with_label("not a constant int")),
		}
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
			return Err(Diagnostic::new(
				format!("`{name}` expects {} type argument(s), got {}", params.len(), args.len()),
				span.into_range(),
			)
			.with_label("wrong number of type arguments"));
		}
		let mut subst = HashMap::new();
		for (param, arg) in params.iter().zip(args) {
			subst.insert(param.name.clone(), self.resolve(arg, span)?);
		}
		Ok(subst)
	}

	// Substitute `subst` into a generic struct's fields, yielding an ordinary `Typ::Struct`.
	pub fn instantiate(
		&self,
		name: &str,
		def: &GenericStructDef,
		subst: &HashMap<String, Typ>,
		span: Span,
	) -> Result<Typ, Diagnostic> {
		if self.depth > MAX_GENERIC_DEPTH {
			return Err(
				Diagnostic::new(format!("`{name}` recurses without end"), span.into_range())
					.with_label("would require infinitely nested fields"),
			);
		}
		let inner = TypeCtx {
			type_params: subst,
			depth: self.depth + 1,
			..*self
		};
		let fields = def
			.fields
			.iter()
			.map(|f| {
				Ok(FieldDef {
					name: f.name.clone(),
					typ: inner.resolve(&f.typ, f.span)?,
					default: f.default.clone(),
					embedded: embedded(f),
					annotations: qualify_anns(self.scope, &f.annotations),
				})
			})
			.collect::<Result<Vec<_>, _>>()?;
		let concrete: Vec<Typ> = def.type_params.iter().map(|p| subst[&p.name].clone()).collect();
		let args: Vec<_> = concrete.iter().map(Typ::key).collect();
		let display = format!("{name}[{}]", args.join(", "));
		self.generics
			.instance_args
			.borrow_mut()
			.entry(display.clone())
			.or_insert(concrete);
		Ok(Typ::Struct(display, fields))
	}

	// Substitute `subst` into a generic enum's variants.
	pub fn instantiate_enum(
		&self,
		name: &str,
		def: &GenericEnumDef,
		subst: &HashMap<String, Typ>,
		span: Span,
	) -> Result<Typ, Diagnostic> {
		if self.depth > MAX_GENERIC_DEPTH {
			return Err(
				Diagnostic::new(format!("`{name}` recurses without end"), span.into_range())
					.with_label("would require infinitely nested variants"),
			);
		}
		let args: Vec<_> = def.type_params.iter().map(|p| subst[&p.name].key()).collect();
		let display = format!("{name}[{}]", args.join(", "));
		if self.generics.instances.borrow().contains_key(&display) {
			return Ok(Typ::Enum(display));
		}
		self.generics.instances.borrow_mut().insert(display.clone(), Vec::new());
		let inner = TypeCtx {
			type_params: subst,
			depth: self.depth + 1,
			..*self
		};
		let variants = build_variants(&def.variants, inner)?;
		self.generics.instances.borrow_mut().insert(display.clone(), variants);
		Ok(Typ::Enum(display))
	}

	// Type names owned by the compiler.
	pub fn builtin_type(name: &str) -> bool {
		matches!(
			name,
			"int"
				| "isize" | "usize"
				| "float" | "bool"
				| "string" | "cstr"
				| "atom" | "any"
				| "array" | "map"
				| "Option" | "Result"
				| "Error" | "Ast"
		) || name.strip_prefix(['i', 'u', 'f']).is_some_and(|w| w.parse::<u16>().is_ok())
	}

	// Resolve a named type.
	pub fn named(&self, name: &str, span: Span) -> Result<Typ, Diagnostic> {
		if let Some(typ) = self.type_params.get(name) {
			return Ok(typ.clone());
		}
		if name == "$?" {
			// an omitted param type that no expected fn type filled in
			return Err(
				Diagnostic::new("parameter needs a type", span.into_range()).with_label("nothing here supplies one")
			);
		}
		if let Some((m, t)) = name.split_once('.') {
			let err = |msg: String, label| Diagnostic::new(msg, span.into_range()).with_label(label);
			let vis = self
				.scope
				.visible
				.get(m)
				.ok_or_else(|| err(format!("unknown module `{m}`"), "not imported"))?;
			let t = vis.only.as_ref().map_or(Some(t), |only| only.get(t).map(String::as_str));
			let t = t.ok_or_else(|| err(format!("`{name}` is not part of `{m}`"), "not in this import"))?;
			return self.named(&format!("{}::{t}", vis.module), span);
		}
		match name {
			"int" => return Ok(Typ::Int(32)),
			"isize" => return Ok(Typ::ISize),
			"usize" => return Ok(Typ::USize),
			"float" => return Ok(Typ::Float(64)),
			"bool" => return Ok(Typ::Bool),
			"string" => return Ok(Typ::Str),
			"cstr" => return Ok(Typ::CStr),
			"atom" => return Ok(Typ::Atom),
			"any" => return Ok(Typ::Any),
			"()" => return Ok(Typ::unit()),
			"Error" => return Ok(Typ::Error),
			"Ast" => return Ok(Typ::Ast),
			_ => {}
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
				_ => Err(
					Diagnostic::new(format!("unsupported float width f{w}"), span.into_range())
						.with_label("supported widths: f16, f32, f64, f128"),
				),
			};
		}
		let name = match self.scope.env.get(name) {
			Some(q) => q.as_str(),
			None if !self.scope.module.is_empty() && name != "Self" && !name.contains("::") => {
				return Err(
					Diagnostic::new(format!("unknown type `{name}`"), span.into_range()).with_label("not a known type")
				);
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
		if self.enums.contains_key(name) {
			return Ok(Typ::Enum(name.to_string()));
		}
		if self.generics.structs.contains_key(name) || self.generics.enums.contains_key(name) {
			return Err(
				Diagnostic::new(format!("`{name}` needs type arguments"), span.into_range())
					.with_label(format!("try `{name}[...]`")),
			);
		}
		if let Some((.., tmethods)) = self.traits.get(name) {
			// dyn dispatch erases the concrete type, so `Self` only works as the receiver
			for (m, ps, ret) in trait_fns(tmethods) {
				let in_ret = matches!(ret, Some((te, _)) if mentions(te, "Self"));
				if in_ret || ps.iter().skip(1).any(|p| mentions(&p.typ, "Self")) {
					let msg = format!("trait `{name}` is not object-safe: `{m}` uses `Self` beyond the receiver");
					return Err(Diagnostic::new(msg, span.into_range()).with_label("can't be a trait object"));
				}
			}
			return Ok(Typ::Trait(name.to_string()));
		}
		Err(Diagnostic::new(format!("unknown type `{name}`"), span.into_range()).with_label("not a known type"))
	}

	// A named sum's members.
	pub fn named_sum(&self, name: &str, span: Span) -> Result<Vec<VariantInfo>, Diagnostic> {
		if self.depth > MAX_GENERIC_DEPTH {
			let msg = format!("`{name}` recurses without end");
			return Err(Diagnostic::new(msg, span.into_range()).with_label("splices itself"));
		}
		let inner = TypeCtx {
			depth: self.depth + 1,
			..*self
		};
		Ok(match inner.resolve(&self.aliases[name], span)? {
			Typ::Sum(_, vs) => vs,
			_ => vec![],
		})
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
				return Err(Diagnostic::new(msg, span.into_range()).with_label("repeated member"));
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
				let typ = self.param(&p.typ, p.span)?;
				let lendable = matches!(
					typ,
					Typ::Array(_) | Typ::FixedArray(..) | Typ::Map(..) | Typ::Struct(..) | Typ::TupleStruct(..)
				) && typ.newtype().is_none();
				if p.access == Access::Mut && !lendable {
					return Err(Diagnostic::new(
						"`mut` parameters must be arrays, maps, or structs for now",
						p.span.into_range(),
					)
					.with_label(format!("{typ} has no address to lend")));
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
