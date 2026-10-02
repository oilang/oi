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

// A trait slot annotated `type` is an associated type, filled per claim as an alias.
pub(crate) fn is_assoc_type(p: &Param) -> bool {
	matches!(&p.typ, TypeExpr::Name(n) if n == "type")
}

// Bind `Self`, plus this type's associated types under their bare slot names.
pub(crate) fn bind_self(aliases: &mut HashMap<String, TypeExpr>, typ: &str) {
	aliases.insert("Self".into(), TypeExpr::Name(typ.into()));
	let prefix = format!("{typ}::");
	let slots: Vec<_> = (aliases.iter())
		.filter_map(|(k, v)| Some((k.strip_prefix(&prefix)?.to_string(), v.clone())))
		.collect();
	aliases.extend(slots);
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
		Expr::Int(_) => matches!(want, Int(_) | UInt(_) | ISize | USize | Float(_) | Rune),
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
		return arity_err("this fn literal", dps.len(), params.len(), "param", span);
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

// Embedding promotes the embedded type's claims, routed like an implicit `via`.
pub(super) fn promote_embeds<'p>(
	structs: &HashMap<String, Vec<FieldDef>>,
	impls: &mut HashSet<(String, String)>,
	claims: &[TraitBody<'p>],
	scope_of: impl Fn(&str) -> &'p Scope,
) -> Vec<TraitBody<'p>> {
	let (mut out, mut settled) = (vec![], usize::MAX);
	while out.len() != settled {
		settled = out.len();
		for (typ, fields) in structs {
			for f in fields.iter().filter(|f| f.embedded) {
				let sn = lends(f);
				// a trait object claims its own trait
				let tns: Vec<String> = match f.typ {
					Typ::Struct(..) => (impls.iter().filter(|(t, _)| *t == sn)).map(|(_, tn)| tn.clone()).collect(),
					Typ::Trait(_) | Typ::Error => vec![sn.clone()],
					_ => continue,
				};
				for tn in tns {
					if is_hook_trait(&tn) || !impls.insert((typ.clone(), tn.clone())) {
						continue;
					}
					// a promoted claim inherits the embedded type's type arguments
					let args = (claims.iter().chain(out.iter()))
						.find(|b| b.typ == sn && b.trait_name == tn)
						.map_or(&[][..], |b| b.args);
					out.push(TraitBody {
						span: Span::default(),
						typ: Box::leak(typ.clone().into_boxed_str()),
						trait_name: tn,
						args,
						via: Some(Box::leak(f.name.clone().into_boxed_str())),
						methods: &[],
						scope: scope_of(typ),
					});
				}
			}
		}
	}
	out
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
		let mut lent = None;
		if let Some(field) = via {
			let held = (types.structs.get(typ)).and_then(|fs| fs.iter().find(|f| f.name == field));
			let Some(sn) = held.map(lends) else {
				let msg = format!("`{typ}` has no field `{field}` to route `{tn}` through");
				return fail(msg, span, "no such field");
			};
			// check whether a via actually claims the mentioned trait
			if sn != tn && !trait_impls.contains(&(sn.clone(), tn.to_string())) {
				let msg = format!("`{sn}` does not claim `{tn}`, so `{typ}` cannot delegate to it");
				return fail(msg, span, "claim it first");
			}
			lent = Some(sn);
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
				return fail(msg, span, label);
			}
			continue;
		}
		let Some((supers, tparams, tfields, tmethods)) = traits.get(tn.as_str()) else {
			return fail(format!("unknown trait `{tn}`"), span, "no such trait");
		};
		let required = tparams.iter().filter(|p| p.default.is_none()).count();
		if args.len() > tparams.len() || args.len() < required {
			let want = match required == tparams.len() {
				true => required.to_string(),
				false => format!("{required} to {}", tparams.len()),
			};
			return arity_err(&format!("trait `{tn}`"), want, args.len(), "type argument", span);
		}
		for s in supers {
			if !trait_impls.contains(&(typ.to_string(), s.clone())) {
				let msg = format!("`{typ}` must also implement `{s}`, the supertrait of `{tn}`");
				return fail(msg, span, "missing supertrait impl");
			}
		}
		let mut sig_params = types.type_params.clone();
		let me = [("Self".to_string(), TypeExpr::Name(typ.into()))];
		for (i, p) in tparams.iter().enumerate() {
			let arg = (args.get(i).cloned()).or_else(|| p.default.as_ref().map(|(te, sp)| (subst(te, &me), *sp)));
			let Some((te, sp)) = arg else { continue };
			sig_params.insert(p.name.clone(), types.with_scope(scope).resolve(&te, sp)?);
		}
		for tf in *tfields {
			if is_assoc_type(tf) {
				if !types.aliases.contains_key(&format!("{typ}::{}", tf.name)) {
					let msg = format!("`{typ}` is missing associated type `{}` of trait `{tn}`", tf.name);
					return fail(msg, span, "fill it in the claim");
				}
				continue;
			}
			let want = types.with_type_params(&sig_params).resolve(&tf.typ, tf.span)?;
			// let embedded structs satisfy field requirements
			let stored = types
				.structs
				.get(typ)
				.and_then(|fs| field_slot(fs, &tf.name))
				.map(|(_, f)| &f.typ);
			let missing = || {
				let msg = format!("`{typ}` is missing field `{} {want}` required by trait `{tn}`", tf.name);
				fail(msg, span, "required by the trait")
			};
			if stored == Some(&want) {
				continue;
			}
			if stored.is_some() {
				return missing();
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
					None => return missing(),
				},
			};
			if !literal_fits(&lit.0, &want) {
				let msg = format!("`{key}` must be a `{want}` literal to satisfy trait `{tn}`");
				return fail(msg, lit.1, "wrong kind of literal");
			}
		}
		let mut sig_aliases = types.aliases.clone();
		bind_self(&mut sig_aliases, typ);
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
				return fail(msg, m.1, "wrong signature");
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
					return fail(msg, span, "wrong signature");
				}
				continue;
			}
			// vias
			if let (Some(field), Some(lent)) = (via, &lent) {
				let s = |e| (e, span);
				// a method without `self` belongs to the lent type, not the instance
				let lent_fn = params.first().is_none_or(|p| p.name != "self");
				let recv = match lent_fn {
					true => Expr::Ident(lent.clone()),
					false => Expr::Field {
						tuple: Box::new(s(Expr::Ident("self".into()))),
						field: field.into(),
					},
				};
				let call = Expr::MethodCall {
					recv: Box::new(s(recv)),
					method: name.clone(),
					type_args: vec![],
					args: params
						.iter()
						.skip(!lent_fn as usize)
						.map(|p| s(Expr::Ident(p.name.clone())))
						.collect(),
				};
				let body = match lent_fn && matches!(ret, Some((TypeExpr::Name(n), _)) if n == "Self") {
					true => Expr::StructLit {
						name: typ.into(),
						type_args: vec![],
						fields: vec![(Some(field.into()), s(call))],
					},
					false => call,
				};
				let by: Vec<_> = (tparams.iter().zip(args).map(|(p, (te, _))| (p.name.clone(), te.clone())))
					.chain([("Self".to_string(), TypeExpr::Name(typ.into()))])
					.collect();
				others.push(FnItem {
					key: format!("{typ}.{name}"),
					scope,
					params: params
						.iter()
						.map(|p| Param {
							typ: subst(&p.typ, &by),
							..p.clone()
						})
						.collect(),
					params_tuple: *params_tuple,
					ret: ret.as_ref().map(|(te, sp)| (subst(te, &by), *sp)),
					body: Box::leak(Box::new([s(body)])),
				});
				continue;
			}
			if body.is_empty() {
				let msg = format!("`{typ}` is missing method `{name}` required by trait `{tn}`");
				return fail(msg, span, "provide this method");
			}
			if let Some(prev) = defaults.insert(key, tn.clone())
				&& prev != tn
			{
				let msg = format!("`{typ}` takes default `{name}` from both `{prev}` and `{tn}`");
				return fail(msg, span, format!("fill `{name}` on `{typ}` to settle it"));
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
