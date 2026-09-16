use super::*;
use crate::ast::Param;

fn unspellable(span: Span) -> Diagnostic {
	Diagnostic::new("cannot express this composed signature", span.into_range()).with_label("write the fn out")
}

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
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let mut head = value;
		while let Expr::Pipe { value, .. } = &head.0 {
			head = value;
		}
		let Some(head_params) = self.head_fn_sig(head)? else {
			let fed = self.expr(value)?;
			let saved = self.dollar.replace(fed);
			let out = self.apply_step(step);
			self.dollar = saved;
			return out;
		};
		let mut params = Vec::with_capacity(head_params.len());
		for (i, typ) in head_params.iter().enumerate() {
			let (name, typ) = (format!("${i}"), type_expr(typ).ok_or_else(|| unspellable(span))?);
			params.push(Param {
				name,
				typ,
				span,
				default: None,
				access: Access::Read,
				mutable: false,
				public: false,
				annotations: vec![],
			});
		}
		let ret = self.stage_ret(step)?;
		let body = [piped(self.feed(value), step.clone(), span)];
		self.declare_anon_fn(&None, &params, params.len() != 1, AnonSig::Explicit(&ret), &body, span)
	}

	// The pipeline with its head turned into a stage that `$` flows into.
	fn feed(&self, expr: &Spanned<Expr>) -> Spanned<Expr> {
		let span = expr.1;
		let dollar = vec![(Expr::Dollar, span)];
		let head = match &expr.0 {
			Expr::Pipe { value, step } => return piped(self.feed(value), (**step).clone(), span),
			Expr::Ident(n) if self.vars.contains_key(n) => called(n.clone(), vec![], dollar, span),
			Expr::Ident(n) => called(self.qualify(n).into_owned(), vec![], dollar, span),
			_ => expr.clone(),
		};
		piped((Expr::Dollar, span), head, span)
	}

	// Feed `$` through one application stage.
	fn apply_step(&mut self, step: &Spanned<Expr>) -> Result<TypedVal, Diagnostic> {
		let span = step.1;
		match &step.0 {
			Expr::AnonFn { .. } => {
				let (val, typ) = self.expr(step)?;
				let arity = match &typ {
					Typ::Fn(params, _) | Typ::Closure(params, _, _) => params.len(),
					_ => 1,
				};
				let args = self.stage_args(arity, span);
				self.call_value("this stage", Callee::Object(val), &typ, &args, None, span)
			}
			Expr::Call { name, type_args, args } if matches!(args[..], [(Expr::Dollar, _)]) => {
				let arity = self.callable(name).map_or(1, |(params, _)| params.len());
				let args = self.stage_args(arity, span);
				self.expr(&called(name.clone(), type_args.clone(), args, span))
			}
			_ => self.expr(step),
		}
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
	fn head_fn_sig(&self, head: &Spanned<Expr>) -> Result<Option<Vec<Typ>>, Diagnostic> {
		let name = match &head.0 {
			Expr::AnonFn { params, ret, .. } if ret.is_some() => {
				let types = self.types();
				let params = params.iter().map(|p| types.resolve(&p.typ, p.span));
				return Ok(Some(params.collect::<Result<_, _>>()?));
			}
			Expr::Ident(name) => name,
			_ => return Ok(None),
		};
		if let Some((params, _)) = self.callable(name) {
			return Ok(Some(params));
		}
		if self.vars.contains_key(name) || !self.generic_fns.contains_key(self.qualify(name).as_ref()) {
			return Ok(None);
		}
		let msg = format!("cannot compose a generic function `{name}`");
		Err(Diagnostic::new(msg, head.1.into_range()).with_label("instantiate it or write the fn out"))
	}

	// A composition returns whatever its last stage does.
	fn stage_ret(&self, stage: &Spanned<Expr>) -> Result<Spanned<TypeExpr>, Diagnostic> {
		let vague = || {
			Diagnostic::new("cannot infer the composed return type", stage.1.into_range())
				.with_label("end the pipeline with a fn, or write the fn out")
		};
		let ret = match &stage.0 {
			Expr::AnonFn { ret: Some(ret), .. } => return Ok(ret.clone()),
			Expr::Ident(name) | Expr::Call { name, .. } => self.callable(name).ok_or_else(vague)?.1,
			_ => return Err(vague()),
		};
		Ok((type_expr(&ret).ok_or_else(|| unspellable(stage.1))?, stage.1))
	}
}
