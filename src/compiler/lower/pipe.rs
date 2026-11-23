use super::call::arg_inner;
use super::*;

fn piped(value: Spanned<Expr>, step: Spanned<Expr>, span: Span) -> Spanned<Expr> {
	let (value, step) = (Box::new(value), Box::new(step));
	(Expr::Pipe { value, step }, span)
}

fn called(name: String, type_args: Vec<Spanned<TypeExpr>>, args: Vec<Spanned<Expr>>, span: Span) -> Spanned<Expr> {
	(Expr::Call { name, type_args, args }, span)
}

impl<M: Module> Translator<'_, M> {
	// Pipeline operator.
	pub(super) fn pipe(
		&mut self,
		value: &Spanned<Expr>,
		step: &Spanned<Expr>,
		hint: Option<&Typ>,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let mut head = value;
		while let Expr::Pipe { value, .. } = &head.0 {
			head = value;
		}

		let Some(head_params) = self.head_fn_sig(head, hint)? else {
			let fed = self.expr(arg_inner(value))?;
			let wrapped = match &step.0 {
				Expr::Call { name, .. } if !matches!(value.0, Expr::Pipe { .. }) => {
					let first = self.callable(name).and_then(|(ps, _)| ps.into_iter().next());
					first.is_some() && first == self.types.happy(&fed.1)
				}
				_ => false,
			};

			// `$` is the place yielded by a void stage
			let place = match self.tapped.take_if(|(at, _)| *at == value.1) {
				Some((_, place)) => place,
				None if let Expr::ArgMod(..) = value.0 => value.clone(),
				None => {
					let name = format!("$pipe{}_{}", value.1.start, value.1.end);
					match Self::is_place(value) || matches!(value.0, Expr::Dollar) {
						true => self.hidden_local(name.clone(), fed.0, fed.1.clone()),
						false => {
							let owned = self.copy_bind(fed.0, &fed.1);
							self.bind_local(&name, owned, fed.1.clone(), true);
						}
					}
					(Expr::Ident(name), value.1)
				}
			};

			if let Expr::ArgMod(..) = place.0 {
				self.check_args(&[Access::Mut, Access::Read], None, &[Some(&place), Some(step)])?;
			}

			let saved = (self.dollar.replace(fed), self.place.replace(arg_inner(&place).clone()));
			let out = self.apply_step(step);
			(self.dollar, self.place) = saved;

			let out = out.map_err(|d| match wrapped {
				true => d.with_note("add `?` to unwrap the head"),
				false => d,
			})?;
			if !out.1.is_unit() {
				return Ok(out);
			}

			let out = self.expr(arg_inner(&place));
			self.tapped = Some((span, place));
			return out;
		};

		let block = (Expr::Block(vec![piped(self.feed(value), step.clone(), span)]), span);
		let sig = self.anon_hint(&block, head_params)?;
		self.lower(&block, sig.as_ref())
	}

	// The pipeline with its head turned into a stage that `$` flows into.
	fn feed(&self, expr: &Spanned<Expr>) -> Spanned<Expr> {
		let span = expr.1;
		let dollar = vec![(Expr::Dollar, span)];
		let head = match &expr.0 {
			Expr::Pipe { value, step } => return piped(self.feed(value), (**step).clone(), span),
			Expr::Ident(n) if self.vars.contains_key(n) => called(n.clone(), vec![], dollar, span),
			Expr::Ident(n) => called(self.qualify(n).into_owned(), vec![], dollar, span),
			_ => dot_fn(expr).unwrap_or_else(|| expr.clone()),
		};
		piped((Expr::Dollar, span), head, span)
	}

	// Feed `$` through one application stage.
	fn apply_step(&mut self, step: &Spanned<Expr>) -> Result<TypedVal, Diagnostic> {
		let span = step.1;
		match &step.0 {
			_ if matches!(step.0, Expr::AnonFn { .. }) || self.type_method(step).is_some() => {
				let (val, typ) = self.expr(step)?;
				let arity = match &typ {
					Typ::Fn(params, _) | Typ::Closure(params, _, _) => params.len(),
					_ => 1,
				};
				let args = self.stage_args(arity, span);
				self.call_value("this stage", Callee::Object(val), &typ, &args, None, span)
			}
			Expr::Call {
				name,
				type_args,
				args: [(Expr::Dollar, _)],
			} => {
				let arity = self.callable(name).map_or(1, |(params, _)| params.len());
				let args = self.stage_args(arity, span);
				self.expr(&called(name.clone(), type_args.clone(), args, span))
			}
			_ => {
				// r/w `$` through its place
				self.dollar = None;
				let step = self.placed(std::slice::from_ref(step));
				self.expr(&step[0])
			}
		}
	}

	// Rebinds `$` to the place it stands for in an expression stage.
	pub(super) fn placed<'e>(&self, body: &'e [Spanned<Expr>]) -> std::borrow::Cow<'e, [Spanned<Expr>]> {
		let (None, Some(place)) = (&self.dollar, &self.place) else {
			return body.into();
		};

		let mut body = body.to_vec();
		body.iter_mut().for_each(|(e, _)| rebind_dollar(e, &place.0));
		body.into()
	}

	// `$` as a stage's arguments, matching the call signature.
	fn stage_args(&self, arity: usize, span: Span) -> Vec<Spanned<Expr>> {
		let dollar = (Expr::Dollar, span);
		if arity == 1 || !matches!(&self.dollar, Some((_, Typ::Tuple(fs))) if fs.len() == arity) {
			return vec![dollar];
		}
		let field = |i: usize| {
			let (tuple, field) = (Box::new(dollar.clone()), i.to_string());
			(Expr::Field { tuple, field }, span)
		};
		(0..arity).map(field).collect()
	}

	// Params and return of a callable name.
	fn callable(&self, name: &str) -> Option<(Vec<Typ>, Typ)> {
		if let Some(local) = self.vars.get(name) {
			let (Typ::Fn(ps, ret) | Typ::Closure(ps, ret, _)) = &local.typ else {
				return None;
			};
			return Some((ps.iter().map(|p| p.typ.clone()).collect(), (**ret).clone()));
		}
		let sig = self.funcs.get(self.qualify(name).as_ref())?;
		Some((sig.params.iter().map(|p| p.typ.clone()).collect(), sig.ret.clone()))
	}

	// The parameter types of a pipeline head that is a fn value, or `None` to apply instead.
	fn head_fn_sig(&mut self, head: &Spanned<Expr>, hint: Option<&Typ>) -> Result<Option<Vec<FnParam>>, Diagnostic> {
		let name = match &head.0 {
			Expr::AnonFn { params, ret, .. } if ret.is_some() => {
				let types = self.types;
				let params = params.iter().map(|p| types.resolve(&p.typ, p.span).map(FnParam::new));
				return Ok(Some(params.collect::<Result<_, _>>()?));
			}
			Expr::Ident(name) => name,
			_ if let Some(sig) = self.type_method(head) => return Ok(Some(sig.value_params())),
			_ if let (Some(_), Some(Typ::Fn(params, _))) = (dot_fn(head), hint) => return Ok(Some(params.clone())),
			_ => return Ok(None),
		};
		if let Some((params, _)) = self.callable(name) {
			return Ok(Some(params.into_iter().map(FnParam::new).collect()));
		}
		if self.vars.contains_key(name) || !self.world.generic_fns.contains_key(self.qualify(name).as_ref()) {
			return Ok(None);
		}
		let msg = format!("cannot compose a generic function `{name}`");
		fail(msg, head.1, "instantiate it or write the fn out")
	}
}
