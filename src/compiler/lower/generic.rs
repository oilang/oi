use super::call::{arg_slots, pack_varargs};
use super::*;
use crate::ast::TypeParam;
use crate::compiler::{MAX_GENERIC_DEPTH, has_ann, role};

// Type params being solved for, and what they're bound to so far.
pub(super) struct Infer<'d> {
	params: &'d [TypeParam],
	pub subst: HashMap<String, Typ>,
	generics: &'d Generics,
}

impl<'d> Infer<'d> {
	pub(super) fn new(params: &'d [TypeParam], generics: &'d Generics) -> Self {
		let subst = HashMap::new();
		Infer {
			params,
			subst,
			generics,
		}
	}

	// Extend `subst` by matching a declared type against a concrete one.
	pub(super) fn unify(&mut self, declared: &TypeExpr, concrete: &Typ) -> Result<(), String> {
		if let TypeExpr::Name(name) = declared
			&& self.params.iter().any(|p| &p.name == name)
		{
			return match self.subst.get(name) {
				Some(bound) if bound != concrete => Err(format!("`{name}` bound to both {bound} and {concrete}")),
				_ => {
					self.subst.insert(name.clone(), concrete.clone());
					Ok(())
				}
			};
		}
		match (declared, concrete) {
			(TypeExpr::Array(elem) | TypeExpr::Variadic(elem), Typ::Array(found)) => self.unify(elem, found),
			(TypeExpr::FixedArray(elem, len), Typ::FixedArray(found, found_len)) => {
				match &len.0 {
					Expr::Int(len) if *len != *found_len as i128 => return Ok(()),
					Expr::Ident(param) => self.unify(
						&TypeExpr::Name(param.clone()),
						&Typ::Const(Box::new(Expr::Int(*found_len as i128))),
					)?,
					_ => {}
				}
				self.unify(elem, found)
			}
			(TypeExpr::FixedArray(val, key), Typ::Map(found_key, found_val)) if let Expr::Ident(key) = &key.0 => {
				self.unify(&TypeExpr::Name(key.clone()), found_key)?;
				self.unify(val, found_val)
			}
			(TypeExpr::Map(key, val), Typ::Map(found_key, found_val)) => {
				self.unify(key, found_key)?;
				self.unify(val, found_val)
			}
			(TypeExpr::Map(key, val), found) if let Some((found_key, arr)) = found.keyed() => {
				self.unify(key, &found_key)?;
				self.unify(val, array_elem(arr))
			}
			(TypeExpr::Tuple(elems), Typ::Tuple(fields)) if elems.len() == fields.len() => elems
				.iter()
				.zip(fields)
				.try_for_each(|((_, elem), (_, found))| self.unify(elem, found)),
			(TypeExpr::Name(unit), found) if unit == "()" && !found.is_unit() => {
				Err(format!("expected (), got {found}"))
			}
			(TypeExpr::Annotated(_, inner), Typ::Annotated(_, found)) => self.unify(inner, found),
			(TypeExpr::Annotated(_, inner), found) => self.unify(inner, found),
			(TypeExpr::Fn(params, ret), Typ::Fn(found, found_ret)) if params.len() == found.len() => {
				(params.iter().zip(found)).try_for_each(|((_, _, param), arg)| self.unify(param, &arg.typ))?;
				self.unify(ret, found_ret)
			}
			(TypeExpr::Generic(name, args), found)
				if name == "Option" && !matches!(found, Typ::Enum(e) if e.starts_with(role::OPTION)) =>
			{
				self.unify(&args[0], found)
			}
			(TypeExpr::Generic(_, args), Typ::Struct(name, _) | Typ::Enum(name)) => {
				let found = self.generics.instance_args(name).unwrap_or_default();
				args.iter().zip(&found).try_for_each(|(arg, found)| self.unify(arg, found))
			}
			_ => Ok(()),
		}
	}

	// The first param nothing has pinned yet.
	pub(super) fn missing(&self) -> Option<&TypeParam> {
		self.params.iter().find(|p| !self.subst.contains_key(&p.name))
	}

	// `unify`, reported at `span`.
	pub(super) fn bind(&mut self, declared: &TypeExpr, concrete: &Typ, span: Span) -> Result<(), Diagnostic> {
		(self.unify(declared, concrete))
			.map_err(|msg| Diagnostic::new(msg, span.into_range()).with_label("type mismatch"))
	}
}

// A monomorph cache key.
pub(super) fn mangle(name: &str, subst: &HashMap<String, Typ>, order: &[TypeParam]) -> String {
	let mut sym = oi_symbol(name);
	for p in order {
		sym.push('$');
		sym.push_str(&subst[&p.name].key());
	}
	sym
}

impl<'a, M: Module> Translator<'a, M> {
	// Emit a call to a generic fn, declaring a monomorphed instance on first use.
	pub(super) fn call_generic(
		&mut self,
		name: &str,
		def: &GenericFnDef,
		type_args: &[Spanned<TypeExpr>],
		args: &[Spanned<Expr>],
		recv: Option<(TypedVal, &Spanned<Expr>)>,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if has_ann(self.types.consts.anns, name, "unsafe") {
			self.require_unsafe(name, span)?;
		}
		let self_n = recv.is_some() as usize;
		let packed = pack_varargs(
			name,
			(def.params[self_n..].iter()).map(|p| (matches!(p.typ, TypeExpr::Variadic(_)), p.default.is_some())),
			args,
			span,
		)?;
		let args = packed.as_deref().unwrap_or(args);
		let names: Vec<&str> = def.params[self_n..].iter().map(|p| p.name.as_str()).collect();
		let named = arg_slots(name, &names, args, false)?;
		let n_defaults = def.params.iter().rev().take_while(|p| p.default.is_some()).count();
		if named.is_none() && (args.len() + n_defaults < names.len() || args.len() > names.len()) {
			return arity_err(&format!("`{name}`"), names.len(), args.len(), "argument", span);
		}
		let slots: Vec<_> = named.unwrap_or_else(|| (0..names.len()).map(|i| args.get(i)).collect());
		// an `=` param's ownership waits on `T`, so trust the arg until then
		let access: Vec<Access> = (def.params.iter().enumerate())
			.map(|(i, p)| match i.checked_sub(self_n).and_then(|i| slots[i]) {
				Some((Expr::ArgMod(a, _), _)) if p.mutable => *a,
				Some(_) if p.mutable => Access::Read,
				_ => p.access,
			})
			.collect();
		self.check_args(&access, recv.as_ref().map(|(_, e)| *e), &slots)?;
		let mut infer = Infer::new(&def.type_params, self.types.generics);
		if type_args.len() > def.type_params.len() {
			return arity_err(
				&format!("`{name}`"),
				def.type_params.len(),
				type_args.len(),
				"type argument",
				span,
			);
		}
		for (param, (te, te_span)) in def.type_params.iter().zip(type_args) {
			infer.subst.insert(param.name.clone(), self.types.arg_typ(param, te, *te_span)?);
		}
		let mut vals = Vec::with_capacity(args.len() + self_n);
		let mut declared = def.params.iter();
		let (mut lent, mut given) = (Vec::new(), Vec::new());
		if let Some(((rval, rtyp), rexpr)) = &recv {
			let rparam = declared.next().unwrap();
			infer.bind(&rparam.typ, rtyp, span)?;
			if rparam.access == Access::Move {
				self.move_out(rexpr, rtyp)?;
				self.untemp(*rval);
			}
			match rparam.access == Access::Mut && rtyp.slots().is_none() {
				true => {
					let (cell, _, entry) = self.arg_value(Access::Mut, rexpr, None)?;
					lent.extend(entry);
					vals.push(cell);
				}
				false => vals.push(*rval),
			}
		}
		for ((arg, param), &access) in slots.iter().zip(declared).zip(&access[self_n..]) {
			let Some(arg) = arg else {
				given.push(None);
				continue;
			};
			// params that are already pinned
			let types = self.types.with_type_params(&infer.subst).with_scope(self.home_scope(def));
			let want = types.resolve(&param.typ, arg.1).ok();
			let (val, typ, entry) = self.arg_value(access, arg, want.as_ref())?;
			lent.extend(entry);
			infer.bind(&param.typ, &typ, arg.1)?;
			given.push(Some((val, typ)));
		}
		self.type_defaults(def, &mut infer)?;
		if let Some(missing) = infer.missing() {
			let kind = if self.types.value_param(missing) {
				"value"
			} else {
				"type"
			};
			return fail(
				format!("cannot infer {kind} parameter `{}`", missing.name),
				span,
				"not determined by any argument",
			);
		}
		for p in &def.type_params {
			let Some(bound) = &p.bound else { continue };
			if self.types.value_param(p) {
				continue;
			}
			if !self.types.traits.contains_key(bound.as_str()) {
				return fail(format!("unknown trait `{bound}`"), span, "no such trait");
			}
			let typ = &infer.subst[&p.name];
			if !self.claims(typ, bound) {
				return fail(
					format!("`{typ}` does not claim `{bound}`"),
					span,
					format!("required by `{}: {bound}`", p.name),
				);
			}
		}
		let sig = self.declare_instance(name, def, infer.subst)?;
		let params = sig.value_params();
		let access: Vec<Access> = params[self_n..].iter().map(|p| self.param_access(p)).collect();
		self.check_args(&access, None, &slots)?;
		let vals = self.fill_args(name, &params, vals, &slots, given, span)?;
		let out = self.emit_call(&sig, &vals);
		self.reload_lent(&lent)?;
		Ok(out)
	}

	// Instantiate a generic fill from the receiver type.
	pub(super) fn recv_instance(&mut self, key: &str, typ: &Typ) -> Option<FnSig> {
		self.instance(key, typ, |def| Some(&def.params.first()?.typ))
	}

	// Instantiate a generic fill from its return type.
	pub(super) fn ret_instance(&mut self, key: &str, typ: &Typ) -> Option<FnSig> {
		self.instance(key, typ, |def| Some(&def.ret.as_ref()?.0))
	}

	fn instance(&mut self, key: &str, typ: &Typ, at: fn(&GenericFnDef) -> Option<&TypeExpr>) -> Option<FnSig> {
		let def = self.world.generic_fns.get(key).cloned()?;
		let mut infer = Infer::new(&def.type_params, self.types.generics);
		infer.unify(at(&def)?, typ).ok()?;
		self.type_defaults(&def, &mut infer).ok()?;
		infer.missing().is_none().then_some(())?;
		self.declare_instance(key, &def, infer.subst).ok()
	}

	// Fill any params inference didn't pin from their declared defaults.
	fn type_defaults(&self, def: &GenericFnDef, infer: &mut Infer) -> Result<(), Diagnostic> {
		for p in &def.type_params {
			let (Some(bound), Some(w)) = (&p.bound, infer.subst.get(&p.name)) else {
				continue;
			};
			let key = (rc::base_name(&w.key()).to_string(), bound.clone());
			let Some(args) = self.world.claim_args.get(&key) else {
				continue;
			};
			let names = self
				.world
				.generic_claims
				.get(&key)
				.into_iter()
				.flatten()
				.map(|p| p.name.clone());
			let own = names
				.zip(self.types.generics.instance_args(&w.key()).unwrap_or_default())
				.collect();
			for ((te, _), (arg, span)) in p.bound_args.iter().zip(args) {
				let typ = self.types_at(*span).with_type_params(&own).resolve(arg, *span)?;
				infer.bind(te, &typ, *span)?;
			}
		}

		for p in &def.type_params {
			let Some((te, span)) = &p.default else { continue };
			if infer.subst.contains_key(&p.name) {
				continue;
			}
			let typ = self.types.with_scope(self.home_scope(def)).resolve(te, *span)?;
			infer.subst.insert(p.name.clone(), typ);
		}
		Ok(())
	}

	// Declare a monomorphed instance's signature, reusing a prior one if it exists.
	pub(super) fn declare_instance(
		&mut self,
		name: &str,
		def: &GenericFnDef,
		subst: HashMap<String, Typ>,
	) -> Result<FnSig, Diagnostic> {
		let sym = mangle(name, &subst, &def.type_params);
		if let Some(sig) = self.out.mono.get(&sym) {
			return Ok(sig.clone());
		}
		let depth = self.out.depths.get(&self.out.lowering).map_or(1, |d| d + 1);
		if depth > MAX_GENERIC_DEPTH {
			let name = name.split('#').next().unwrap_or(name);
			return fail(
				format!("`{name}` recurses without end"),
				def.span,
				"instantiates itself forever",
			);
		}
		self.out.depths.insert(sym.clone(), depth);
		let types = self.types.with_type_params(&subst).with_scope(self.home_scope(def));
		let params = types.resolve_params(&def.params)?;
		let ret = match &def.ret {
			Some((ret_te, ret_span)) => types.resolve(ret_te, *ret_span)?,
			None => Typ::unit(),
		};

		let mut sig = self.module.make_signature();
		sig.params
			.extend(params.iter().map(|(_, t, a)| AbiParam::new(param_cl(t, *a, self.int))));
		if def.ctx.is_some() {
			sig.params.push(AbiParam::new(self.int)); // hidden `ctx`
		}
		if !def.captures.is_empty() {
			sig.params.push(AbiParam::new(self.int));
		}
		if !ret.is_unit() {
			sig.returns.push(AbiParam::new(cl_type(&ret, self.int)));
		}
		let id = self
			.module
			.declare_function(&sym, Linkage::Local, &sig)
			.expect("declare function");
		let fn_sig = FnSig {
			id,
			access: params.iter().map(|(_, _, a)| *a).collect(),
			params: (params.into_iter().zip(&def.params))
				.map(|((_, t, _), p)| FnParam {
					escapes: escapes(&p.name, &t, &def.body),
					..FnParam::of(p, t)
				})
				.collect(),
			ret,
			foreign: false,
			ctx: def.ctx.clone(),
			unsafe_call: false,
			pure: def.pure || has_ann(self.types.consts.anns, name, role::PURE),
			default: false,
		};
		self.out.mono.insert(sym.clone(), fn_sig.clone());
		self.out.pending.push((sym, def.clone(), subst));
		Ok(fn_sig)
	}
}
