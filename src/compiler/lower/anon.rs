use super::*;
use crate::ast::{Capture, Param};

// An anon fn's signature.
pub(super) enum AnonSig<'a> {
	Explicit(&'a Spanned<TypeExpr>),
	Inferred(Typ),
}

impl<'a, M: Module> Translator<'a, M> {
	// Declare an anon fn literal.
	pub(super) fn declare_anon_fn(
		&mut self,
		captures: &Option<Vec<Capture>>,
		params: &[Param],
		params_tuple: bool,
		sig: AnonSig,
		body: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if let Some(p) = params.iter().find(|p| p.default.is_some()) {
			let msg = "default params are only supported on named fns";
			return Err(Diagnostic::new(msg, p.span.into_range()).with_label("remove the default"));
		}
		let self_name = self.self_name.take();
		let inferred;
		let captures: &[Capture] = match captures {
			Some(list) => list,
			None => {
				let mut names: Vec<_> = free_vars(body)
					.into_iter()
					.filter(|n| self.vars.contains_key(n) && !params.iter().any(|p| &p.name == n))
					.collect();
				names.sort();
				inferred = names.into_iter().map(Capture::ReadOnly).collect::<Vec<_>>();
				&inferred
			}
		};
		let owns = !captures.is_empty() && captures.iter().all(|c| matches!(c, Capture::Move(_)));
		let mut resolved = Vec::with_capacity(captures.len());
		for c in captures {
			let (name, boxed) = match c {
				Capture::Mut(name) => (name, true),
				Capture::ReadOnly(name) | Capture::Move(name) => (name, false),
			};
			let local = self.local(name, span.into_range())?;
			let val = match c {
				Capture::Mut(_) => self.box_local(name, &local, span.into_range())?,
				Capture::Move(_) => self.move_local(name, &local, span.into_range())?,
				Capture::ReadOnly(_) => self.read_local(&local),
			};
			resolved.push((name.clone(), local.typ, boxed, val));
		}

		let (params, params_tuple, ret_te, subst) = match sig {
			AnonSig::Explicit(te) => (params.to_vec(), params_tuple, te.clone(), HashMap::new()),
			AnonSig::Inferred(Typ::Fn(ptyps, ret)) => {
				let name = |i: usize| format!("${i}");
				if !params.is_empty() && params.len() != ptyps.len() {
					let msg = format!("this fn literal expects {} param(s), got {}", ptyps.len(), params.len());
					return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of params"));
				}
				// a fn header keeps its names and fills its omitted types from the type sig
				let (params, tuple) = match params.is_empty() {
					true => (
						(0..ptyps.len())
							.map(|i| Param {
								typ: TypeExpr::Name(name(i)),
								name: name(i),
								span,
								default: None,
								access: Access::Read,
								mutable: false,
								public: false,
								annotations: vec![],
							})
							.collect(),
						ptyps.len() != 1,
					),
					false => (
						(params.iter().enumerate())
							.map(|(i, p)| match &p.typ {
								TypeExpr::Name(n) if n == "$?" => Param {
									typ: TypeExpr::Name(name(i)),
									..p.clone()
								},
								_ => p.clone(),
							})
							.collect(),
						params_tuple,
					),
				};
				let subst = (ptyps.into_iter().enumerate().map(|(i, p)| (name(i), p.typ)))
					.chain([("$ret".into(), *ret)])
					.collect();
				(params, tuple, (TypeExpr::Name("$ret".into()), span), subst)
			}
			AnonSig::Inferred(_) => unreachable!("a fn literal is only inferred against a fn target"),
		};
		let def = GenericFnDef {
			params,
			params_tuple,
			ret: Some(ret_te),
			body: body.to_vec(),
			type_params: vec![],
			captures: resolved.iter().map(|(n, t, boxed, _)| (n.clone(), t.clone(), *boxed)).collect(),
			self_name,
			module: self.scope.module.clone(),
		};
		let sym = format!("anon${}_{}", span.start, self.mono.len());
		let sig = self.declare_instance(&sym, &def, subst)?;
		let params = sig.value_params();
		if resolved.is_empty() {
			return Ok((self.fn_object(sig.id), Typ::Fn(params, Box::new(sig.ret))));
		}

		self.wanted.push(sig.id);
		let func_ref = self.module.declare_func_in_func(sig.id, self.b.func);
		let addr = self.b.ins().func_addr(self.int, func_ref);
		let env = self.call_alloc_bytes(((1 + resolved.len()) * 8) as i64);
		self.b.ins().store(MemFlags::new(), addr, env, 0);
		for (i, (_, _, _, val)) in resolved.iter().enumerate() {
			self.b.ins().store(MemFlags::new(), *val, env, ((i + 1) * 8) as i32);
		}
		Ok((env, Typ::Closure(params, Box::new(sig.ret), owns)))
	}
}

// Every identifier referenced in `body`.
fn free_vars(body: &[Spanned<Expr>]) -> HashSet<String> {
	let mut out = HashSet::new();
	body.iter().for_each(|(e, _)| e.idents(&mut out));
	out
}
