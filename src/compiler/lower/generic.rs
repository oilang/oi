use super::call::{arg_slots, pack_varargs};
use super::*;
use crate::ast::TypeParam;
use crate::compiler::{ann, role};

// Extend `subst` by matching a declared type against a concrete arg type.
pub(super) fn unify(
	declared: &TypeExpr,
	concrete: &Typ,
	params: &[TypeParam],
	subst: &mut HashMap<String, Typ>,
	generics: &Generics,
) -> Result<(), String> {
	if let TypeExpr::Name(n) = declared
		&& params.iter().any(|p| &p.name == n)
	{
		return match subst.get(n) {
			Some(bound) if bound != concrete => Err(format!("`{n}` bound to both {bound} and {concrete}")),
			_ => {
				subst.insert(n.clone(), concrete.clone());
				Ok(())
			}
		};
	}
	match (declared, concrete) {
		(TypeExpr::Array(e), Typ::Array(c)) | (TypeExpr::Variadic(e), Typ::Array(c)) => {
			unify(e, c, params, subst, generics)
		}
		(TypeExpr::FixedArray(e, n), Typ::FixedArray(c, cn)) if matches!(n.0, Expr::Int(n) if n == *cn as i64) => {
			unify(e, c, params, subst, generics)
		}
		(TypeExpr::Map(k, v), Typ::Map(ck, cv)) => {
			unify(k, ck, params, subst, generics)?;
			unify(v, cv, params, subst, generics)
		}
		(TypeExpr::Tuple(elems), Typ::Tuple(fields)) if elems.len() == fields.len() => elems
			.iter()
			.zip(fields)
			.try_for_each(|((_, e), (_, f))| unify(e, f, params, subst, generics)),
		(TypeExpr::Generic(_, gargs), Typ::Struct(name, _) | Typ::Enum(name)) => match generics.instance_args(name) {
			Some(cargs) => gargs
				.iter()
				.zip(&cargs)
				.try_for_each(|(g, c)| unify(g, c, params, subst, generics)),
			None => Ok(()),
		},
		// a non-param name, atom-sum, etc: trust it, the call emits against the real signature anyway
		_ => Ok(()),
	}
}

// A monomorph cache key.
fn mangle(name: &str, subst: &HashMap<String, Typ>, order: &[TypeParam]) -> String {
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
		if self
			.types
			.consts
			.anns
			.get(name)
			.is_some_and(|a| a.iter().any(|x| ann(x, "unsafe").is_some()))
		{
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
		if named
			.as_ref()
			.map_or(args.len() != names.len(), |s| s.iter().any(Option::is_none))
		{
			return Err(Diagnostic::new(
				format!("`{name}` expects {} argument(s), got {}", names.len(), args.len()),
				span.into_range(),
			)
			.with_label("wrong number of arguments"));
		}
		let slots: Vec<_> = named.unwrap_or_else(|| args.iter().map(Some).collect());
		let access: Vec<Access> = def.params.iter().map(|p| p.access).collect();
		self.check_args(&access, recv.as_ref().map(|(_, e)| *e), &slots)?;
		let mut subst = HashMap::new();
		if type_args.len() > def.type_params.len() {
			return Err(Diagnostic::new(
				format!(
					"`{name}` expects {} type argument(s), got {}",
					def.type_params.len(),
					type_args.len()
				),
				span.into_range(),
			)
			.with_label("wrong number of type arguments"));
		}
		for (param, (te, te_span)) in def.type_params.iter().zip(type_args) {
			subst.insert(param.name.clone(), self.types().resolve(te, *te_span)?);
		}
		let mut vals = Vec::with_capacity(args.len() + self_n);
		let mut declared = def.params.iter();
		if let Some(((rval, rtyp), rexpr)) = &recv {
			let rparam = declared.next().unwrap();
			unify(&rparam.typ, rtyp, &def.type_params, &mut subst, self.types.generics)
				.map_err(|msg| Diagnostic::new(msg, span.into_range()).with_label("type mismatch"))?;
			if rparam.access == Access::Move {
				self.move_out(rexpr, rtyp)?;
			}
			vals.push(*rval);
		}
		let mut lent = Vec::new();
		for (arg, param) in slots.iter().flatten().zip(declared) {
			let (val, typ, entry) = self.arg_value(param.access, arg, None)?;
			lent.extend(entry.map(|e| (val, e)));
			unify(&param.typ, &typ, &def.type_params, &mut subst, self.types.generics)
				.map_err(|msg| Diagnostic::new(msg, arg.1.into_range()).with_label("type mismatch"))?;
			vals.push(val);
		}
		self.type_defaults(def, &mut subst)?;
		if let Some(missing) = def.type_params.iter().find(|p| !subst.contains_key(&p.name)) {
			return Err(Diagnostic::new(
				format!("cannot infer type parameter `{}`", missing.name),
				span.into_range(),
			)
			.with_label("not determined by any argument"));
		}
		for p in &def.type_params {
			let Some(bound) = &p.bound else { continue };
			if !self.types.traits.contains_key(bound.as_str()) {
				return Err(
					Diagnostic::new(format!("unknown trait `{bound}`"), span.into_range()).with_label("no such trait")
				);
			}
			let typ = &subst[&p.name];
			if !self.claims(typ, bound) {
				return Err(
					Diagnostic::new(format!("`{typ}` does not claim `{bound}`"), span.into_range())
						.with_label(format!("required by `{}: {bound}`", p.name)),
				);
			}
		}
		let sig = self.declare_instance(name, def, subst)?;
		let out = self.emit_call(&sig, &vals);
		self.reload_lent(&lent);
		Ok(out)
	}

	// Instantiate a generic fill from the receiver type.
	pub(super) fn recv_instance(&mut self, key: &str, typ: &Typ) -> Option<FnSig> {
		let def = self.generic_fns.get(key).cloned()?;
		let mut subst = HashMap::new();
		let recv = &def.params.first()?.typ;
		unify(recv, typ, &def.type_params, &mut subst, self.types.generics).ok()?;
		self.type_defaults(&def, &mut subst).ok()?;
		def.type_params.iter().all(|p| subst.contains_key(&p.name)).then_some(())?;
		self.declare_instance(key, &def, subst).ok()
	}

	// Fill any params inference didn't pin from their declared defaults.
	fn type_defaults(&self, def: &GenericFnDef, subst: &mut HashMap<String, Typ>) -> Result<(), Diagnostic> {
		for p in &def.type_params {
			let Some((te, span)) = &p.default else { continue };
			if subst.contains_key(&p.name) {
				continue;
			}
			let typ = self.types().with_scope(self.home_scope(&def.module)).resolve(te, *span)?;
			subst.insert(p.name.clone(), typ);
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
		if let Some(sig) = self.mono.get(&sym) {
			return Ok(sig.clone());
		}
		let types = self.types.with_type_params(&subst).with_scope(self.home_scope(&def.module));
		let params = types.resolve_params(&def.params)?;
		let ret = match &def.ret {
			Some((ret_te, ret_span)) => types.resolve(ret_te, *ret_span)?,
			None => Typ::unit(),
		};

		let mut sig = self.module.make_signature();
		sig.params
			.extend(params.iter().map(|(_, t, _)| AbiParam::new(cl_type(t, self.int))));
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
				.map(|((_, t, _), p)| FnParam::of(p, t))
				.collect(),
			ret,
			foreign: false,
			unsafe_call: false,
			pure: self
				.types
				.consts
				.anns
				.get(name)
				.is_some_and(|a| a.iter().any(|x| ann(x, role::PURE).is_some())),
		};
		self.mono.insert(sym.clone(), fn_sig.clone());
		self.pending.push((sym, def.clone(), subst));
		Ok(fn_sig)
	}
}
