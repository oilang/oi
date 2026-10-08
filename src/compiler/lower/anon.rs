use super::*;
use crate::ast::{Capture, Param};
use crate::compiler::CONTEXT;

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
			return fail(msg, p.span, "remove the default");
		}
		let self_name = self.self_name.take();
		let inferred;
		let captures: &[Capture] = match captures {
			Some(list) => list,
			None => {
				let mut names: Vec<_> = free_vars(body)
					.into_iter()
					.filter(|n| n != CTX && self.vars.contains_key(n) && !params.iter().any(|p| &p.name == n))
					.collect();
				names.sort();
				inferred = names.into_iter().map(Capture::ReadOnly).collect::<Vec<_>>();
				&inferred
			}
		};
		if self.pure && !captures.is_empty() {
			let msg = "a `@pure` fn can't capture";
			return fail(msg, span, "it only sees its params");
		}
		let owns = !captures.is_empty() && captures.iter().all(|c| matches!(c, Capture::Move(_)));
		let mut resolved = Vec::with_capacity(captures.len());
		for c in captures {
			let (name, boxed) = match c {
				Capture::Mut(name) => (name, true),
				Capture::ReadOnly(name) | Capture::Move(name) => (name, false),
			};
			check_reserved(name, span)?;
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
			AnonSig::Inferred(Typ::Fn(ptyps, ret) | Typ::Closure(ptyps, ret, _)) => {
				let name = |i: usize| format!("${i}");
				if !params.is_empty() && params.len() != ptyps.len() {
					return arity_err("this fn literal", ptyps.len(), params.len(), "param", span);
				}
				// a fn header keeps its names and fills its omitted types from the type sig
				let (params, tuple) = match params.is_empty() {
					true => (
						(0..ptyps.len())
							.map(|i| Param::new(name(i), TypeExpr::Name(name(i)), span))
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

		// context of the scope
		let base = self.types.named(CONTEXT, span)?;
		let mut needs = false;
		Expr::Block(body.to_vec()).walk(&mut |e| {
			needs |= match e {
				Expr::Field { tuple, field } => {
					matches!(&tuple.0, Expr::Ident(n) if n == CTX)
						&& !matches!(&base, Typ::Struct(_, fs) if fs.iter().any(|f| f.name == *field))
				}
				Expr::Call { name, .. } => (self.funcs.get(self.qualify(name).as_ref()))
					.is_some_and(|s| s.ctx.as_deref().is_some_and(|c| c != CONTEXT)),
				_ => false,
			}
		});
		let ctx = match (self.anon_ctx.take(), self.vars.get(CTX)) {
			(Some(t), _) => (t != "none").then_some(t),
			(None, Some(l)) if needs => Some(l.typ.key()),
			_ => Some(CONTEXT.into()),
		};

		let def = GenericFnDef {
			params,
			params_tuple,
			ret: Some(ret_te),
			body: body.to_vec(),
			type_params: vec![],
			captures: resolved.iter().map(|(n, t, boxed, _)| (n.clone(), t.clone(), *boxed)).collect(),
			self_name,
			module: self.types.scope.module.clone(),
			span,
			pure: self.pure,
			ctx,
		};
		let sym = format!("anon${}_{}", span.start, self.mono.len());
		if self.c_callback {
			self.roots.push(oi_symbol(&sym));
		}
		let sig = self.declare_instance(&sym, &def, subst)?;
		let params = sig.value_params();
		if resolved.is_empty() {
			let typ = Typ::Fn(params, Box::new(sig.ret.clone()));
			let typ = match self.pure {
				true => Typ::Annotated(vec![role::PURE.into()], typ.into()),
				false => sig.ctx_marked(typ),
			};
			return Ok((self.fn_object(sig.id), typ));
		}

		self.wanted.push(sig.id);
		let func_ref = self.module.declare_func_in_func(sig.id, self.b.func);
		let addr = self.b.ins().func_addr(self.int, func_ref);
		let slots: Vec<_> = std::iter::once(addr).chain(resolved.iter().map(|r| r.3)).collect();
		let env = self.heap_slots(&slots);
		let typ = Typ::Closure(params, Box::new(sig.ret.clone()), owns);
		Ok((env, sig.ctx_marked(typ)))
	}
}

// Every identifier referenced in `body`.
fn free_vars(body: &[Spanned<Expr>]) -> HashSet<String> {
	let mut out = HashSet::new();
	body.iter().for_each(|(e, _)| e.idents(&mut out));
	out
}
