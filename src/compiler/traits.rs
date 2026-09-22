//! Trait declarations and impls checking.

use super::*;
use crate::loader::{fold_const, hook_method, is_hook_trait};

// A trait's supertraits, fields, and methods.
pub(crate) type TraitItem<'a> = (Vec<String>, &'a [TypeParam], &'a [Param], &'a [Spanned<Expr>]);

// A trait method's name, params, and return annotation.
pub(crate) type TraitFn<'a> = (&'a str, &'a [Param], &'a Option<Spanned<TypeExpr>>);

// A fill's params, tuple-ness, and return annotation.
pub(crate) type FillSig = (Vec<Param>, bool, Option<Spanned<TypeExpr>>);

// A trait impl body.
pub(crate) struct TraitBody<'a> {
	pub span: Span,
	pub typ: &'a str,
	pub trait_name: String,
	pub args: &'a [Spanned<TypeExpr>],
	pub via: Option<&'a str>,
	pub methods: &'a [Spanned<Expr>],
	pub scope: &'a Scope,
}

// Whether a primitive natively satisfies std trait `tn`.
pub(crate) fn builtin_claim(typ: &Typ, tn: &str) -> bool {
	use Typ::*;
	if matches!(tn, role::CONTAINS | role::ITERATOR | role::ITERABLE) {
		return false;
	}
	match typ {
		Int(_) => true,
		UInt(_) | ISize | USize => tn != role::NEG,
		Float(_) => tn != role::MOD && !role::BITWISE.contains(&tn),
		Bool | Atom => matches!(tn, role::EQ | role::ORD),
		Str => matches!(tn, role::EQ | role::ADD),
		_ => false,
	}
}

pub(crate) fn trait_fns(methods: &[Spanned<Expr>]) -> impl Iterator<Item = TraitFn<'_>> {
	methods.iter().filter_map(|m| match &m.0 {
		Expr::Fn { name, params, ret, .. } => Some((name.as_str(), params.as_slice(), ret)),
		_ => None,
	})
}

// Whether a literal's kind matches a declared field type.
fn literal_fits(lit: &Expr, want: &Typ) -> bool {
	use Typ::*;
	match lit {
		Expr::Int(_) => matches!(want, Int(_) | UInt(_) | ISize | USize | Float(_)),
		Expr::Float(_) => matches!(want, Float(_)),
		Expr::String(_) => matches!(want, Str),
		Expr::Bool(_) => matches!(want, Bool),
		_ => false,
	}
}

// Complete a fill's signature from the trait's declaration.
// An empty, non-tuple param list means the header was omitted entirely.
pub(crate) fn fill_from_decl(
	params: &[Param],
	params_tuple: bool,
	ret: &Option<Spanned<TypeExpr>>,
	(_, dps, dret): TraitFn,
	span: Span,
) -> Result<FillSig, Diagnostic> {
	let ret = ret.clone().or_else(|| dret.clone());
	if params.is_empty() && !params_tuple {
		let ps = (dps.iter().enumerate())
			.map(|(i, d)| {
				let name = if d.name == "self" {
					d.name.clone()
				} else {
					format!("${i}")
				};
				Param {
					name,
					span,
					default: None,
					..d.clone()
				}
			})
			.collect();
		return Ok((ps, dps.len() != 1, ret));
	}
	let omitted = |p: &Param| matches!(&p.typ, TypeExpr::Name(n) if n == "$?");
	// a spelled-out header of the wrong arity is left for the signature check to report
	if params.len() != dps.len() && params.iter().any(omitted) {
		let msg = format!("this fn literal expects {} param(s), got {}", dps.len(), params.len());
		return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of params"));
	}
	let ps = (params.iter().enumerate())
		.map(|(i, p)| match dps.get(i) {
			Some(d) if omitted(p) => Param {
				typ: d.typ.clone(),
				..p.clone()
			},
			_ => p.clone(),
		})
		.collect();
	Ok((ps, params_tuple, ret))
}

// Check trait impl bodies.
// Validates supertraits, required fields, method sigs.
pub(super) fn check_impls<'p>(
	trait_bodies: Vec<TraitBody<'p>>,
	traits: &HashMap<&'p str, TraitItem<'p>>,
	core_traits: &HashSet<String>,
	trait_impls: &HashSet<(String, String)>,
	types: TypeCtx,
	others: &mut Vec<FnItem<'p>>,
	consts: &mut HashMap<String, Spanned<Expr>>,
) -> Result<(), Diagnostic> {
	let mut defaults: HashMap<(String, String), String> = HashMap::new();
	for TraitBody {
		span,
		typ,
		trait_name: tn,
		args,
		via,
		methods,
		scope,
	} in trait_bodies
	{
		// vias
		if let Some(field) = via {
			let inner = types
				.structs
				.get(typ)
				.and_then(|fs| embeds(fs).find_map(|(o, sn, _)| (fs[o].name == field).then_some(sn)));
			let Some(sn) = inner else {
				let msg = format!("`{typ}` has no embedded field `{field}` to route `{tn}` through");
				return Err(Diagnostic::new(msg, span.into_range()).with_label("not an embedded field"));
			};
			if !trait_impls.contains(&(sn.to_string(), tn.to_string())) {
				let msg = format!("`{sn}` does not claim `{tn}`, so `{typ}` cannot delegate to it");
				return Err(Diagnostic::new(msg, span.into_range()).with_label("claim it first"));
			}
		}
		if is_hook_trait(&tn) {
			let hook = hook_method(&tn);
			let well_formed = methods.iter().any(|m| {
				matches!(&m.0, Expr::Fn { name, params, .. }
					if *name == hook && params.len() == 1 && params[0].name == "self" && params[0].access == Access::Mut)
			});
			if !well_formed {
				let msg = format!("`impl {tn} for {typ}` must define `fn {hook}(mut self)`");
				let label = format!("missing or wrong `{hook}` method");
				return Err(Diagnostic::new(msg, span.into_range()).with_label(label));
			}
			continue;
		}
		let Some((supers, tparams, tfields, tmethods)) = traits.get(tn.as_str()) else {
			return Err(Diagnostic::new(format!("unknown trait `{tn}`"), span.into_range()).with_label("no such trait"));
		};
		if args.len() != tparams.len() {
			let msg = format!(
				"trait `{tn}` takes {} type argument(s), got {}",
				tparams.len(),
				args.len()
			);
			return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of type arguments"));
		}
		for s in supers {
			if !trait_impls.contains(&(typ.to_string(), s.clone())) {
				let msg = format!("`{typ}` must also implement `{s}`, the supertrait of `{tn}`");
				return Err(Diagnostic::new(msg, span.into_range()).with_label("missing supertrait impl"));
			}
		}
		for tf in *tfields {
			let want = types.resolve(&tf.typ, tf.span)?;
			// let embedded structs satisfy field requirements
			let stored = types
				.structs
				.get(typ)
				.and_then(|fs| field_slot(fs, &tf.name))
				.map(|(_, f)| &f.typ);
			let missing = || {
				let msg = format!("`{typ}` is missing field `{} {want}` required by trait `{tn}`", tf.name);
				Diagnostic::new(msg, span.into_range()).with_label("required by the trait")
			};
			if stored == Some(&want) {
				continue;
			}
			if stored.is_some() {
				return Err(missing());
			}
			let key = format!("{typ}::{}", tf.name);
			let lit = match consts.get(&key) {
				Some(c) => c.clone(),
				None => match &tf.default {
					Some(default) => {
						let folded = fold_const(&default.0, &*consts, scope).unwrap_or_else(|| default.0.clone());
						let lit = (folded, default.1);
						consts.insert(key.clone(), lit.clone());
						lit
					}
					None => return Err(missing()),
				},
			};
			if !literal_fits(&lit.0, &want) {
				let msg = format!("`{key}` must be a `{want}` literal to satisfy trait `{tn}`");
				return Err(Diagnostic::new(msg, lit.1.into_range()).with_label("wrong kind of literal"));
			}
		}
		let mut sig_aliases = types.aliases.clone();
		sig_aliases.insert("Self".into(), TypeExpr::Name(typ.into()));
		let mut sig_params = types.type_params.clone();
		for (p, (te, sp)) in tparams.iter().zip(args) {
			sig_params.insert(p.name.clone(), types.with_scope(scope).resolve(te, *sp)?);
		}
		let sig_types = TypeCtx::new(
			types.structs,
			types.enums,
			&sig_aliases,
			&sig_params,
			types.generics,
			types.traits,
		)
		.with_consts(types.consts)
		.with_scope(scope);
		let sig = |ps: &[Param], ret: &Option<Spanned<TypeExpr>>| -> Result<Typ, Diagnostic> {
			let param = |p: &Param| {
				let typ = sig_types.param(&p.typ, p.span)?;
				Ok(FnParam::of(p, access_wrap(p.access, typ)))
			};
			let params = ps.iter().map(param).collect::<Result<_, _>>()?;
			let ret = match ret {
				Some((te, sp)) => sig_types.resolve(te, *sp)?,
				None => Typ::unit(),
			};
			Ok(Typ::Fn(params, Box::new(ret)))
		};
		for m in methods {
			let Expr::Fn {
				name,
				params,
				params_tuple,
				ret,
				..
			} = &m.0
			else {
				continue;
			};
			let Some(decl @ (_, tp, tr)) = trait_fns(tmethods).find(|(n, ..)| n == name) else {
				continue;
			};
			let (params, _, ret) = fill_from_decl(params, *params_tuple, ret, decl, m.1)?;
			let (mut got, want) = (sig(&params, &ret)?, sig(tp, tr)?);
			if matches!(tn.as_str(), role::ADD | role::SUB | role::MUL | role::DIV | role::MOD)
				&& core_traits.contains(tn.as_str())
				&& let (Typ::Fn(gp, _), Typ::Fn(wp, _)) = (&mut got, &want)
				&& let ([_, gother], [_, wother]) = (gp.as_mut_slice(), wp.as_slice())
			{
				gother.typ = wother.typ.clone();
			}
			if got != want {
				let msg = format!("`{typ}.{name}` is `{got}`, trait `{tn}` declares `{want}`");
				return Err(Diagnostic::new(msg, m.1.into_range()).with_label("wrong signature"));
			}
		}
		for t in *tmethods {
			let Expr::Fn {
				name,
				params,
				params_tuple,
				ret,
				body,
				..
			} = &t.0
			else {
				continue;
			};
			if methods.iter().any(|m| matches!(&m.0, Expr::Fn { name: n, .. } if n == name)) {
				continue;
			}
			// check whether an overlapping fill is already present
			let key = (typ.to_string(), name.clone());
			if !defaults.contains_key(&key)
				&& let Some(f) = others.iter().find(|f| f.key == format!("{typ}.{name}"))
			{
				let (got, want) = (sig(&f.params, &f.ret)?, sig(params, ret)?);
				if got != want {
					let msg = format!("`{typ}.{name}` is `{got}`, trait `{tn}` declares `{want}`");
					return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong signature"));
				}
				continue;
			}
			// vias
			if let Some(field) = via {
				let s = |e| (e, span);
				let recv = Expr::Field {
					tuple: Box::new(s(Expr::Ident("self".into()))),
					field: field.into(),
				};
				let call = Expr::MethodCall {
					recv: Box::new(s(recv)),
					method: name.clone(),
					type_args: vec![],
					args: params.iter().skip(1).map(|p| s(Expr::Ident(p.name.clone()))).collect(),
				};
				others.push(FnItem {
					key: format!("{typ}.{name}"),
					scope,
					params: params.clone(),
					params_tuple: *params_tuple,
					ret: ret.clone(),
					body: Box::leak(Box::new([s(call)])),
				});
				continue;
			}
			if body.is_empty() {
				let msg = format!("`{typ}` is missing method `{name}` required by trait `{tn}`");
				return Err(Diagnostic::new(msg, span.into_range()).with_label("provide this method"));
			}
			if let Some(prev) = defaults.insert(key, tn.clone())
				&& prev != tn
			{
				let msg = format!("`{typ}` takes default `{name}` from both `{prev}` and `{tn}`");
				return Err(Diagnostic::new(msg, span.into_range())
					.with_label(format!("fill `{name}` on `{typ}` to settle it")));
			}
			others.push(FnItem {
				key: format!("{typ}.{name}"),
				scope,
				params: params.clone(),
				params_tuple: *params_tuple,
				ret: ret.clone(),
				body,
			});
		}
	}
	Ok(())
}
