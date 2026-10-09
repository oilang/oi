use crate::compiler::expand;

use super::*;

// A branching construct's merge state. The first branch to yield declares `result`.
pub(super) struct Join {
	kw: &'static str,
	span: Span,
	pub result: Option<(Variable, Typ)>,
	reached: bool,
}

impl Join {
	pub(super) fn new(kw: &'static str, span: Span, result: Option<(Variable, Typ)>) -> Self {
		Join {
			kw,
			span,
			result,
			reached: false,
		}
	}
}

// An `or` fallback, and its payload once something throws.
pub(crate) struct Catch {
	block: Block,
	depth: usize,
	err: Option<(Variable, Typ)>,
}

// A header bind is a test only when its pattern can fail, otherwise it just binds.
fn infallible(cond: &Expr) -> bool {
	match cond {
		Expr::Bind { .. } => true,
		Expr::PatBind { pat, .. } => matches!(pat.0, Expr::Tuple(_) | Expr::Array(_) | Expr::StructLit { .. }),
		_ => false,
	}
}

impl<'a, M: Module> Translator<'a, M> {
	// Lower branching control flow constructs.
	pub(super) fn branching(
		&mut self,
		expr: &Spanned<Expr>,
		hint: Option<&Typ>,
		want: bool,
	) -> Result<Option<TypedVal>, Diagnostic> {
		match &expr.0 {
			Expr::If { cond, then, els } => self.conditional(cond, then, els.as_deref(), hint, expr.1, want),
			Expr::Match {
				subject,
				arms,
				else_body,
			} => self.match_expr(subject, arms, else_body.as_deref(), None, hint, expr.1),
			Expr::Loop { cond, body } => self.looped(|s| s.loop_expr(cond.as_deref(), body)),
			_ => unreachable!(),
		}
	}

	// `if`/`else` lowered to branch&merge, yielding value of the chosen branch.
	// A diverging branch contributes nothing to the merge.
	// If all branches diverge, returns None.
	pub(super) fn conditional(
		&mut self,
		cond: &Spanned<Expr>,
		then: &[Spanned<Expr>],
		els: Option<&[Spanned<Expr>]>,
		target: Option<&Typ>,
		span: Span,
		want: bool,
	) -> Result<Option<TypedVal>, Diagnostic> {
		if infallible(&cond.0) {
			return self.scoped(|s| {
				s.expr(cond)?;
				if let Some((id, arm)) = s.unwrap_arm(cond, then) {
					return s.match_expr(&id, &[arm], els, Some(&[]), target, span);
				}
				if let Some([first, ..]) = els {
					let msg = "this binding always succeeds, so `else` can never run";
					return fail(msg, first.1, "unreachable");
				}
				s.block_tail(then, target)
			});
		}

		if let Expr::PatBind { pat, value, .. } = &cond.0 {
			let arm = MatchArm {
				patterns: vec![(**pat).clone()],
				body: then.to_vec(),
				..Default::default()
			};
			return self.match_expr(value, &[arm], els, Some(&[]), target, span);
		}

		let cv = self.bool_value(cond, "`if` condition")?;
		let (then_block, else_block) = self.fork(cv);

		let merge = self.b.create_block();
		let mut join = Join::new("if", span, None);

		// a slot is settled only if every path assigns it or diverges
		let (outer, mut live) = (self.slots.clone(), vec![]);
		let (owned, mut tails) = (self.scopes.clone(), vec![]);
		self.b.switch_to_block(then_block);
		let then_flow = self.branch(then, target)?;
		self.rejoin(&outer, &mut live, then_flow.is_none());
		let tail = self.branch_tail(&owned, &mut tails, then_flow.is_some());
		if let Some(vt) = then_flow {
			self.join_branch(want, vt, &mut join, tail)?;
		}

		self.b.switch_to_block(else_block);
		let else_flow = self.else_default(els, &join, target)?;
		self.rejoin(&outer, &mut live, else_flow.is_none());
		self.slots = live;
		let tail = self.branch_tail(&owned, &mut tails, else_flow.is_some());
		if let Some(vt) = else_flow {
			self.join_branch(want, vt, &mut join, tail)?;
		}
		self.join_moves(owned, tails, merge);

		if !want {
			return Ok(join.reached.then(|| {
				self.b.switch_to_block(merge);
				self.b.seal_block(merge);
				self.unit_value()
			}));
		}
		Ok(self.finish_merge(merge, join.result))
	}

	// Route a branch's tail value.
	fn join_branch(&mut self, want: bool, vt: TypedVal, join: &mut Join, merge: Block) -> Result<(), Diagnostic> {
		if want {
			return self.contribute(vt, join, merge);
		}
		let (v, t) = vt;
		self.release_value(v, &t);
		self.b.ins().jump(merge, &[]);
		join.reached = true;
		Ok(())
	}

	// The else arm.
	fn else_default(
		&mut self,
		els: Option<&[Spanned<Expr>]>,
		join: &Join,
		target: Option<&Typ>,
	) -> Result<Option<TypedVal>, Diagnostic> {
		let Some(els) = els else {
			let t = (join.result.as_ref())
				.map(|(_, t)| t.clone())
				.or_else(|| target.cloned())
				.unwrap_or(Typ::unit());
			let ok = self.types.fallible(&t);
			return self.scoped(|s| {
				let v = if ok { s.zero(&t) } else { s.zero_or_err(&t, join.span)? };
				Ok(Some((v, t.clone())))
			});
		};
		self.branch(els, target)
	}

	// A branch that is a lone `defer` arms it on the enclosing scope.
	fn branch(&mut self, body: &[Spanned<Expr>], target: Option<&Typ>) -> Result<Option<TypedVal>, Diagnostic> {
		match body {
			[(Expr::Defer { body, when }, _)] => Ok(Some(self.defer(body, *when, true))),
			_ => self.scoped(|s| s.block_tail(body, target)),
		}
	}

	// Evaluate `f` in a child scope.
	pub(super) fn scoped(
		&mut self,
		f: impl FnOnce(&mut Self) -> Result<Option<TypedVal>, Diagnostic>,
	) -> Result<Option<TypedVal>, Diagnostic> {
		self.vars.mark();
		let withs = self.withs.len();
		self.scopes.push(vec![]);
		self.defers.push(vec![]);
		let flow = f(self);
		self.vars.undo();
		self.withs.truncate(withs);
		let out = match flow? {
			Some((v, t)) => {
				let v = self.copy_bind(v, &t);
				self.release_scopes(self.scopes.len() - 1, None)?;
				Some((v, t))
			}
			None => None,
		};
		self.scopes.pop();
		self.defers.pop();
		Ok(out)
	}

	// A macro expansion scoped block, with no surface syntax of its own.
	pub(super) fn block_expr(&mut self, body: &[Spanned<Expr>], span: Span) -> Result<TypedVal, Diagnostic> {
		match self.scoped(|s| s.block_tail(body, None))? {
			Some(vt) => Ok(vt),
			None => fail(
				"this block never produces a value",
				span,
				"every path returns, but a value is needed here",
			),
		}
	}

	fn finish_merge(&mut self, merge: Block, result: Option<(Variable, Typ)>) -> Option<TypedVal> {
		result.map(|(var, typ)| {
			self.b.switch_to_block(merge);
			self.b.seal_block(merge);
			let v = self.b.use_var(var);
			self.temp(v, &typ);
			(v, typ)
		})
	}

	// Match.
	pub(super) fn match_expr<'e>(
		&mut self,
		subject: &Spanned<Expr>,
		arms: &[MatchArm],
		mut else_body: Option<&'e [Spanned<Expr>]>,
		// whatever the arms leave uncovered
		gap: Option<&'e [Spanned<Expr>]>,
		target: Option<&Typ>,
		span: Span,
	) -> Result<Option<TypedVal>, Diagnostic> {
		// patterns see through a ref, like `.` does
		let (sv, st) = self.expr(subject)?;
		let (sv, st) = self.deref(sv, &st);
		let sv_var = self.b.declare_var(cl_type(&st, self.int));
		self.b.def_var(sv_var, sv);

		// a non-enum match has no coverage to analyse, so it is all a gap
		if !st.is_enumish() && else_body.is_none() {
			else_body = gap.filter(|g| !g.is_empty());
		}

		// ensure match covers every variant when applicable
		if st.is_enumish() {
			let pats = || arms.iter().flat_map(|a| &a.patterns);
			let catch_all = else_body.is_some() || pats().any(|p| matches!(&p.0, Expr::Ident(w) if w == "_"));
			if !catch_all && st == Typ::Any {
				return Err(Diagnostic::new("a match on `any` needs `else`", span.into_range()));
			}
			if !catch_all {
				let variants = self.variants_of(&st);
				let covered = pats()
					.map(|p| self.enum_pattern(p, &st).map(|(d, _)| d))
					.collect::<Result<Vec<_>, _>>()?;
				let missing: Vec<_> = variants
					.iter()
					.filter(|v| !covered.contains(&v.disc))
					.map(|v| v.name.clone())
					.collect();
				if !missing.is_empty() {
					if gap.is_none() {
						let msg = format!("non-exhaustive match, missing: {}", missing.join(", "));
						return fail(msg, span, "cover these variants or add `else`");
					}
					else_body = gap.filter(|g| !g.is_empty());
				}
			}
		}

		let merge = self.b.create_block();
		let mut join = Join::new("match", span, None);
		let (outer, mut live) = (self.slots.clone(), vec![]);
		let (owned, mut tails) = (self.scopes.clone(), vec![]);

		// pre-create each arm's entry block so each arm knows where to fall through to on failure
		let arm_entries: Vec<Block> = arms.iter().map(|_| self.b.create_block()).collect();
		let else_blk = self.b.create_block();
		self.b.ins().jump(arm_entries.first().copied().unwrap_or(else_blk), &[]);

		for (i, arm) in arms.iter().enumerate() {
			let arm_body = self.b.create_block();
			let fail = arm_entries.get(i + 1).copied().unwrap_or(else_blk);

			self.b.switch_to_block(arm_entries[i]);
			self.b.seal_block(arm_entries[i]);

			// bindings
			let mut binds = vec![];
			let mut quote_binds: Option<(Value, Vec<String>)> = None;
			for (j, pat) in arm.patterns.iter().enumerate() {
				let eq = if matches!(&pat.0, Expr::Ident(w) if w == "_") {
					// `_` wildcard
					self.b.ins().iconst(types::I8, 1)
				} else if let Some(bounds) = pat.0.bounds() {
					let sv = self.b.use_var(sv_var);
					self.range_pattern(sv, &st, bounds, pat.1)?
				} else if st.is_enumish() {
					let (disc, b) = self.enum_pattern(pat, &st)?;
					if arm.patterns.len() == 1 {
						binds = b;
					}
					let sv = self.b.use_var(sv_var);
					let tag = self.enum_tag(&st, sv);
					self.b.ins().icmp_imm(IntCC::Equal, tag, disc)
				} else if matches!(&pat.0, Expr::Tuple(_) | Expr::StructLit { .. } | Expr::Array(_)) {
					let b = self.pat_binds(pat, &st)?;
					if arm.patterns.len() == 1 {
						binds = b;
					}
					match &pat.0 {
						Expr::Array(elems) => {
							let sv = self.b.use_var(sv_var);
							let (_, len) = self.array_parts(sv, &st);
							let count = self.b.ins().iconst(self.int, elems.len() as i64);
							self.b.ins().icmp(IntCC::Equal, len, count)
						}
						_ => self.b.ins().iconst(types::I8, 1),
					}
				} else if st == Typ::Ast
					&& let Expr::Quote(q) = &pat.0
				{
					if arm.patterns.len() != 1 || q.len() != 1 {
						let msg = "a quote pattern is a single expression, alone in its arm";
						return Err(Diagnostic::new(msg, pat.1.into_range()));
					}
					let (tpl, slots) = expand::register(q, pat.1)?;
					let names: Vec<String> = slots
						.into_iter()
						.map(|s| match s {
							expand::Slot::Name(n) => Some(n),
							_ => None,
						})
						.collect::<Option<_>>()
						.ok_or_else(|| {
							let msg = "only `%name` captures are allowed in a quote pattern";
							Diagnostic::new(msg, pat.1.into_range())
						})?;
					let outs = self.stack_slot((names.len().max(1) * 8) as u32);
					let sv = self.b.use_var(sv_var);
					let tplv = self.b.ins().iconst(self.int, tpl as i64);
					let func = self.import_fn(expand::RT_QUOTE_MATCH, &[self.int; 3], Some(self.int));
					let call = self.b.ins().call(func, &[tplv, sv, outs]);
					let matched = self.b.inst_results(call)[0];
					quote_binds = Some((outs, names));
					self.b.ins().icmp_imm(IntCC::NotEqual, matched, 0)
				} else {
					let sv = self.b.use_var(sv_var);
					let (pv, pt) = self.check_expr(pat, &st)?;
					if pt != st {
						return Err(Diagnostic::new(
							format!("match pattern ({pt}) does not match subject ({st})"),
							pat.1.into_range(),
						)
						.with_label("type mismatch"));
					}
					self.emit_eq(sv, pv, &st)
				};
				if j + 1 < arm.patterns.len() {
					let next = self.b.create_block();
					self.b.ins().brif(eq, arm_body, &[], next, &[]);
					self.b.seal_block(next);
					self.b.switch_to_block(next);
				} else {
					self.b.ins().brif(eq, arm_body, &[], fail, &[]);
				}
			}

			self.b.seal_block(arm_body);
			self.b.switch_to_block(arm_body);
			let flow = self.scoped(|s| {
				let cap = s.sum_capture(arm, &st);
				if let Some(name) = &arm.binding
					&& cap.is_none()
				{
					s.vars.insert(name.clone(), Local::plain(sv_var, st.clone(), false));
				}
				for (name, typ, off) in cap.iter().chain(&binds) {
					let sv = s.b.use_var(sv_var);
					let fv = s.load_bind(sv, &st, typ, *off, span);
					let fv = s.copy_bind(fv, typ);
					s.bind_local(name, fv, typ.clone(), false);
				}
				if let Some((outs, names)) = &quote_binds {
					for (i, name) in names.iter().enumerate() {
						let ptr = s.ld_word(*outs, (i * 8) as i32);
						s.bind_local(name, ptr, Typ::Ast, false);
					}
				}
				s.block_tail(&arm.body, target)
			})?;
			self.rejoin(&outer, &mut live, flow.is_none());
			let tail = self.branch_tail(&owned, &mut tails, flow.is_some());
			if let Some(vt) = flow {
				self.contribute(vt, &mut join, tail)?;
			}
		}

		self.b.switch_to_block(else_blk);
		self.b.seal_block(else_blk);
		let else_flow = self.else_default(else_body, &join, target)?;
		let unreachable = else_flow.is_none() || (st.is_enumish() && else_body.is_none());
		self.rejoin(&outer, &mut live, unreachable);
		self.slots = live;
		let tail = self.branch_tail(&owned, &mut tails, else_flow.is_some());
		if let Some(vt) = else_flow {
			self.contribute(vt, &mut join, tail)?;
		}
		self.join_moves(owned, tails, merge);

		Ok(self.finish_merge(merge, join.result))
	}

	// Write (v, t) into the shared result variable and jump to `merge`.
	// All branches must agree on type. The first one declares the variable.
	pub(super) fn contribute(&mut self, (v, t): TypedVal, join: &mut Join, merge: Block) -> Result<(), Diagnostic> {
		match &mut join.result {
			Some((_, rt)) if rt != &t => fail(
				format!("`{}` branches have mismatched types: {rt} and {t}", join.kw),
				join.span,
				"must yield the same type",
			),
			Some((var, _)) => {
				self.b.def_var(*var, v);
				self.b.ins().jump(merge, &[]);
				Ok(())
			}
			None => {
				let var = self.b.declare_var(cl_type(&t, self.int));
				self.b.def_var(var, v);
				self.b.ins().jump(merge, &[]);
				join.result = Some((var, t));
				Ok(())
			}
		}
	}

	// A propagator payload, and the error type if it's a Result.
	pub(super) fn fallible_split(&self, typ: &Typ) -> Option<(Typ, Option<Typ>)> {
		let res = self.types.result_parts(typ).map(|(ok, err)| (ok, Some(err)));
		res.or_else(|| Some((self.types.option_inner(typ)?, None)))
	}

	// `or` blocks, for unwrapping Options and Results.
	// The happy branch yields the inner value, the sad branch executes a block and yields its value.
	pub(super) fn or_else(
		&mut self,
		value: &Spanned<Expr>,
		body: &[Spanned<Expr>],
		span: Span,
		want: bool,
	) -> Result<TypedVal, Diagnostic> {
		let mut catch = Catch {
			block: self.b.create_block(),
			depth: self.scopes.len(),
			err: None,
		};

		// a pipeline is a try scope, with its `?` steps landing here
		let lowered = if let Expr::Pipe { .. } = value.0 {
			let outer = self.catch.replace(catch);
			let lowered = self.expr(value);
			catch = std::mem::replace(&mut self.catch, outer).expect("set above");
			lowered
		} else {
			self.expr(value)
		};

		let (val, typ) = lowered?;
		let (payload, inner) = match self.fallible_split(&typ) {
			Some((inner, err)) => {
				let tag = self.enum_tag(&typ, val);
				let is_happy = self.b.ins().icmp_imm(IntCC::Equal, tag, err.is_none() as i64);
				let (happy_block, sad_block) = self.fork(is_happy);
				self.b.switch_to_block(sad_block);
				let err = match err {
					Some(e) => (self.ld_typ(val, 8, &e), e),
					None => self.unit_value(),
				};
				self.throw(&mut catch, err, value.1)?;
				self.b.switch_to_block(happy_block);
				(self.opt_payload(val, &typ, &inner, 8), inner)
			}
			None if catch.err.is_some() => (val, typ),
			None => {
				let msg = format!("`or` needs a `?T`/`!T` value, got {typ}");
				return fail(msg, value.1, "not an Option or Result");
			}
		};
		let merge = self.b.create_block();
		let mut join = Join::new("or", span, None);
		let payload = self.copy_bind(payload, &inner);
		self.join_branch(want, (payload, inner.clone()), &mut join, merge)?;

		self.b.switch_to_block(catch.block);
		self.b.seal_block(catch.block);
		let (var, err) = catch.err.expect("every path here threw");
		let saved_dollar = self.dollar.replace((self.b.use_var(var), err));
		let flow = self.branch(body, want.then_some(&inner))?;
		self.dollar = saved_dollar;
		if let Some(vt) = flow {
			self.join_branch(want, vt, &mut join, merge)?;
		}

		if !want {
			self.b.switch_to_block(merge);
			self.b.seal_block(merge);
			return Ok(self.unit_value());
		}
		Ok(self.finish_merge(merge, join.result).expect("`or` always yields"))
	}

	// Jump to an `or` fallback, the first throw pins `$`.
	fn throw(&mut self, catch: &mut Catch, (v, t): TypedVal, span: Span) -> Result<(), Diagnostic> {
		let (var, want) = catch
			.err
			.get_or_insert_with(|| (self.b.declare_var(cl_type(&t, self.int)), t.clone()))
			.clone();
		let (v, got) = self.coerce(v, &t, &want, span)?;
		if got != want {
			return fail(format!("cannot catch {t} as {want}"), span, "mismatched error type");
		}
		self.release_scopes(catch.depth, None)?;
		self.b.def_var(var, v);
		self.b.ins().jump(catch.block, &[]);
		Ok(())
	}

	// Check that error `from` can flow into `to`, returning the `From` conversion it needs, if any.
	fn err_into(&self, from: &Typ, to: &Typ, msg: String, span: Span) -> Result<Option<FnSig>, Diagnostic> {
		let in_sum = self.through_sum(Some(to), |t| t == from).as_ref() == Some(from);
		if to == from || in_sum || (*to == Typ::Error && self.open_error(from)) {
			return Ok(None);
		}
		let sig = self
			.claims(to, "core::From")
			.then(|| self.find_fill(&format!("{to}.from"), 0, from));
		let diag = Diagnostic::new(msg, span.into_range());
		sig.flatten().map(Some).ok_or_else(|| match *to == Typ::Error {
			true => diag.with_label(format!("`{from}` does not claim Error")),
			false => diag
				.with_label("mismatched error type")
				.with_note(format!("claim `{to} : From[{from}]`")),
		})
	}

	// Rewrap a value's none/error as a `target`.
	fn pass_sad(
		&mut self,
		val: Value,
		typ: &Typ,
		target: &Typ,
		from: Option<&FnSig>,
		span: Span,
	) -> Result<Value, Diagnostic> {
		let Some(((_, err), (_, target_err))) = self.types.result_parts(typ).zip(self.types.result_parts(target))
		else {
			return Ok(self.make_option(target, None));
		};
		let e = self.ld_word(val, 8);
		let e = match from {
			Some(sig) => self.emit_call(sig, &[e]).0,
			None => self.coerce(e, &err, &target_err, span)?.0,
		};
		let variants = self.variants_of(target);
		Ok(self.make_enum(&variants, 1, &[e]))
	}

	// `and` blocks.
	// The happy branch yields the body, and the sad branch passes through.
	pub(super) fn and_then(
		&mut self,
		value: &Spanned<Expr>,
		body: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let (val, typ) = self.expr(value)?;
		let Some((inner, err)) = self.fallible_split(&typ) else {
			let msg = format!("`and` needs a `?T`/`!T` value, got {typ}");
			return fail(msg, value.1, "not an Option or Result");
		};

		let tag = self.enum_tag(&typ, val);
		let is_happy = self.b.ins().icmp_imm(IntCC::Equal, tag, err.is_none() as i64);
		let (happy_block, sad_block) = self.fork(is_happy);
		let merge = self.b.create_block();
		let mut join = Join::new("and", span, None);

		self.b.switch_to_block(happy_block);
		let payload = self.opt_payload(val, &typ, &inner, 8);
		let saved_dollar = self.dollar.replace((payload, inner));
		let flow = self.branch(body, None);
		self.dollar = saved_dollar;
		let target = match flow? {
			Some((v, t)) => {
				let (v, t) = match (&err, self.fallible_split(&t)) {
					(Some(_), Some((_, Some(_)))) | (None, Some((_, None))) => (v, t),
					(_, Some(_)) => {
						return fail("`and` cannot mix `?T` and `!T`", span, format!("the body yields {t}"));
					}
					(Some(e), _) => {
						let r = self.types.core_enum(role::RESULT, &[t, e.clone()]);
						(self.make_enum(&self.variants_of(&r), 0, &[v]), r)
					}
					(None, _) => {
						let o = self.types.core_enum(role::OPTION, &[t]);
						(self.make_option(&o, Some(v)), o)
					}
				};
				self.contribute((v, t.clone()), &mut join, merge)?;
				t
			}
			None => typ.clone(),
		};
		let from = match (&err, self.types.result_parts(&target)) {
			(Some(e), Some((_, to))) => {
				self.err_into(e, &to, format!("cannot pass `{e}` through `and` into {target}"), span)?
			}
			_ => None,
		};

		self.b.switch_to_block(sad_block);
		let sad = self.pass_sad(val, &typ, &target, from.as_ref(), span)?;
		self.contribute((sad, target), &mut join, merge)?;
		Ok(self.finish_merge(merge, join.result).expect("`and` always yields"))
	}

	// Unwraps `?T`/`!T`.
	// Returns `none`/error from the enclosing fn on the sad path.
	// Panics when called in `main`.
	pub(super) fn propagate(&mut self, value: &Spanned<Expr>, span: Span) -> Result<TypedVal, Diagnostic> {
		let (val, typ) = self.expr(value)?;
		let Some((inner, err)) = self.fallible_split(&typ) else {
			let msg = format!("`?` needs a `?T` or `!T` value, got {typ}");
			return fail(msg, value.1, "not a `?T` or `!T` value");
		};
		let (is_result, err_typ) = (err.is_some(), err.unwrap_or(Typ::Error));
		let shape = if is_result { "!T" } else { "?T" };
		let panic_in_main = self.ret.is_none() && self.is_main;
		let mut target_err = err_typ.clone();
		let mut from = None;
		let declared = self.ret.as_ref().filter(|_| self.catch.is_none()).map(|(t, _)| t.clone());
		let target = match &declared {
			Some(d) if is_result && let Some((t, e)) = self.types.result_parts(d) => {
				from = self.err_into(
					&err_typ,
					&e,
					format!("cannot propagate `{err_typ}` into a fn returning {d}"),
					span,
				)?;
				target_err = e;
				t
			}
			Some(d) if !is_result && let Some(t) = self.types.option_inner(d) => t,
			Some(other) => {
				let msg = format!("`?` needs an enclosing fn returning `{shape}`, found {other}");
				return fail(msg, span, format!("not a `{shape}` fn"));
			}
			None => inner.clone(),
		};
		let target_typ = match is_result {
			true => self.types.core_enum(role::RESULT, &[target.clone(), target_err.clone()]),
			false => self.types.core_enum(role::OPTION, std::slice::from_ref(&target)),
		};

		let tag = self.enum_tag(&typ, val);
		let is_happy = self.b.ins().icmp_imm(IntCC::Equal, tag, !is_result as i64);

		let (happy_block, sad_block) = self.fork(is_happy);

		self.b.switch_to_block(sad_block);
		if let Some(mut catch) = self.catch.take() {
			let err = match is_result {
				true => (self.ld_word(val, 8), err_typ.clone()),
				false => self.unit_value(),
			};
			self.throw(&mut catch, err, span)?;
			self.catch = Some(catch);
		} else if panic_in_main {
			let msg = if is_result {
				let e = self.ld_word(val, 8);
				self.derived_str(e, &err_typ, false)
			} else {
				self.str_const("unwrapped `none`")
			};
			self.ctx_panic("panic", msg, span)?;
		} else {
			let sad_val = self.pass_sad(val, &typ, &target_typ, from.as_ref(), span)?;
			self.emit_return(sad_val, target_typ, span)?;
		}

		self.b.switch_to_block(happy_block);
		let payload = self.opt_payload(val, &typ, &inner, 8);
		Ok((payload, inner))
	}

	// Report the error and exit 1 (main's sad path).
	pub(crate) fn emit_fail(&mut self, val: Value, typ: &Typ) {
		let Some((_, err)) = self.types.result_parts(typ) else {
			return;
		};
		let tag = self.enum_tag(typ, val);
		let (sad, done) = self.fork(tag);
		self.b.switch_to_block(sad);
		let e = self.ld_word(val, 8);
		let mut msg = self.derived_str(e, &err, false);
		if let Some(sig) = (err == Typ::Error).then(|| self.funcs.get(role::ORIGIN).cloned()).flatten() {
			let (at, _) = self.emit_call(&sig, &[e]);
			msg = self.rt_call("str_concat", &[msg, at]).unwrap();
		}
		self.rt_call("fail", &[msg]);
		self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);
		self.b.switch_to_block(done);
	}

	// `Enum.from(v)`.
	pub(super) fn enum_from(&mut self, name: &str, args: &[Spanned<Expr>], span: Span) -> Result<TypedVal, Diagnostic> {
		if args.len() != 1 {
			let msg = format!("`{name}.from` takes 1 argument, got {}", args.len());
			return fail(msg, span, "wrong number of arguments");
		}
		let (av, at) = self.expr(&args[0])?;
		if !matches!(
			at,
			Typ::Str | Typ::Atom | Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize
		) {
			let msg = format!("`{name}.from` needs an int, string, or atom. Got {at}");
			return fail(msg, args[0].1, "not an int, string, or atom");
		}

		let target = self.types.core_enum(role::RESULT, &[Typ::Enum(name.to_string()), Typ::Error]);
		let target_variants = self.variants_of(&target);
		let variants = self.enum_variants(name);

		let msg = self.str_const("no matching variant");
		let err = self.box_error(msg, &Typ::Str);
		let mut result = self.make_enum(&target_variants, 1, &[err]);
		for v in &variants {
			let matched = match at {
				Typ::Str => {
					let name_const = self.str_const(&v.name);
					self.emit_eq(av, name_const, &Typ::Str)
				}
				Typ::Atom => {
					let name_const = self.atom_const(&v.name);
					self.b.ins().icmp(IntCC::Equal, av, name_const)
				}
				_ => {
					let disc = self.b.ins().iconst(cl_type(&at, self.int), v.disc);
					self.b.ins().icmp(IntCC::Equal, av, disc)
				}
			};
			let fields: Vec<Value> = v.payload.iter().map(|t| self.zero(t)).collect();
			let inner = self.make_enum(&variants, v.disc, &fields);
			let wrapped = self.make_enum(&target_variants, 0, &[inner]);
			result = self.b.ins().select(matched, wrapped, result);
		}
		Ok((result, target))
	}

	pub(super) fn loop_expr(
		&mut self,
		cond: Option<&Spanned<Expr>>,
		body: &[Spanned<Expr>],
	) -> Result<Option<TypedVal>, Diagnostic> {
		let top = self.b.create_block();
		self.b.ins().jump(top, &[]);
		self.b.switch_to_block(top);

		// a conditional loop branches, into the body or out through `fallthrough`
		let (exit, fallthrough) = match cond {
			Some((Expr::PatBind { .. }, _)) | None => (None, None),
			Some((c, _)) if infallible(c) => (None, None),
			Some(cond) => {
				let (cv, ct) = self.expr(cond)?;
				if ct != Typ::Bool {
					// non-bool headers are iterated
					self.b.seal_block(top);
					let wild = (Expr::Ident("_".into()), cond.1);
					return self.for_value(&wild, (cv, ct), cond.1, body).map(Some);
				}
				let exit = self.b.create_block();
				let (body_block, fallthrough) = self.fork(cv);
				self.b.switch_to_block(body_block);
				(Some(exit), Some(fallthrough))
			}
		};

		// a body expression that can fall through ends the loop when it does
		let (frame, flow) = self.in_loop(top, exit, fallthrough, |s| match (cond, body) {
			(Some(c), _) if infallible(&c.0) => {
				s.expr(c)?;
				match s.unwrap_arm(c, body) {
					Some((id, arm)) => s.match_expr(&id, &[arm], None, Some(&[(Expr::Break(None), c.1)]), None, c.1),
					None => s.block(body),
				}
			}
			(Some((Expr::PatBind { pat, value, .. }, sp)), _) => {
				let arm = MatchArm {
					patterns: vec![(**pat).clone()],
					body: body.to_vec(),
					..Default::default()
				};
				s.match_expr(value, &[arm], None, Some(&[(Expr::Break(None), *sp)]), None, *sp)
			}
			(_, [(Expr::Match { subject, arms, .. }, sp)]) => {
				s.match_expr(subject, arms, None, Some(&[(Expr::Break(None), *sp)]), None, *sp)
			}
			_ => s.block(body),
		})?;

		if let Some((v, t)) = flow {
			// a discarded body value is released here, once per iteration
			self.release_value(v, &t);
			self.b.ins().jump(top, &[]);
		}
		self.b.seal_block(top);

		match frame.exit {
			Some(exit) => Ok(Some(self.loop_end(frame, exit))),
			// an infinite loop with no `break` never falls through
			None => Ok(None),
		}
	}

	// `v := opt` re-matches `v` to bind the payload
	fn unwrap_arm(&self, cond: &Spanned<Expr>, body: &[Spanned<Expr>]) -> Option<(Spanned<Expr>, MatchArm)> {
		let Expr::Bind { name, .. } = &cond.0 else { return None };
		self.fallible_split(&self.vars.get(name)?.typ)?;
		let id = (Expr::Ident(name.clone()), cond.1);
		let arm = MatchArm {
			patterns: vec![id.clone()],
			body: body.to_vec(),
			..Default::default()
		};
		Some((id, arm))
	}

	// Push a loop frame, run `body` in a child scope, then pop the frame.
	fn in_loop(
		&mut self,
		top: Block,
		exit: Option<Block>,
		fallthrough: Option<Block>,
		body: impl FnOnce(&mut Self) -> Result<Option<TypedVal>, Diagnostic>,
	) -> Result<(LoopFrame, Option<TypedVal>), Diagnostic> {
		let depth = self.scopes.len();
		self.loops.push(LoopFrame {
			top,
			exit,
			depth,
			result: None,
			fallthrough,
		});
		let flow = self.scoped(body)?;
		Ok((self.loops.pop().expect("loop frame"), flow))
	}

	// Merge at `exit` and take the loop's value.
	fn loop_end(&mut self, frame: LoopFrame, exit: Block) -> TypedVal {
		if let Some(fallthrough) = frame.fallthrough {
			self.b.switch_to_block(fallthrough);
			if let Some((var, t)) = &frame.result {
				let v = match self.types.option_inner(t) {
					Some(_) => self.make_option(&t.clone(), None),
					None => self.unit_value().0,
				};
				self.b.def_var(*var, v);
			}
			self.b.ins().jump(exit, &[]);
		}
		self.b.seal_block(exit);
		self.b.switch_to_block(exit);
		match frame.result {
			Some((var, t)) => (self.b.use_var(var), t),
			None => self.unit_value(),
		}
	}

	// Drive an Iterator.
	fn iter_loop(
		&mut self,
		pat: &Spanned<Expr>,
		body: &[Spanned<Expr>],
		(val, typ): TypedVal,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let (it, it_typ) = match self.find_fill(&format!("{}.iter", typ.key()), 0, &typ) {
			Some(sig) if self.claims(&typ, role::ITERABLE) => self.emit_call(&sig, &[val]),
			_ => (val, typ),
		};
		let next = self.find_fill(&format!("{}.next", it_typ.key()), 0, &it_typ);
		let item = next.as_ref().and_then(|n| self.types.option_inner(&n.ret));
		let (Some(next), Some(item)) = (next, item) else {
			return fail(
				format!("cannot iterate over {it_typ}"),
				span,
				"its `Iterator` claim has no `next(mut self) ?T`",
			);
		};
		let opt = next.ret.clone();

		let (header, exit) = (self.b.create_block(), self.b.create_block());
		self.b.ins().jump(header, &[]);

		self.b.switch_to_block(header);
		let (yielded, _) = self.emit_call(&next, &[it]);
		let tag = self.enum_tag(&opt, yielded);
		let (body_block, fallthrough) = self.fork(tag);

		self.b.switch_to_block(body_block);
		let (frame, flow) = self.in_loop(header, Some(exit), Some(fallthrough), |s| {
			let v = s.opt_payload(yielded, &opt, &item, 8);
			s.bind_pat(pat, v, &item, Some(false))?;
			s.block(body)
		})?;
		if let Some((v, t)) = flow {
			self.release_value(v, &t);
			self.b.ins().jump(header, &[]);
		}
		self.b.seal_block(header);
		Ok(self.loop_end(frame, exit))
	}

	pub(super) fn for_loop(
		&mut self,
		pat: &Spanned<Expr>,
		iter: &Spanned<Expr>,
		body: &[Spanned<Expr>],
	) -> Result<TypedVal, Diagnostic> {
		let tv = self.expr(iter)?;
		self.for_value(pat, tv, iter.1, body)
	}

	fn for_value(
		&mut self,
		pat: &Spanned<Expr>,
		(val, typ): TypedVal,
		span: Span,
		body: &[Spanned<Expr>],
	) -> Result<TypedVal, Diagnostic> {
		if self.claims(&typ, role::ITERABLE) || self.claims(&typ, role::ITERATOR) {
			return self.iter_loop(pat, body, (val, typ), span);
		}
		let zero = self.b.ins().iconst(self.int, 0);
		let (limit, src, vals): (_, TypedVal, Option<TypedVal>) = match typ {
			Typ::Array(_) | Typ::FixedArray(..) | Typ::Str => {
				let (data, len) = self.array_parts(val, &typ);
				(len, (data, array_elem(&typ).clone()), None)
			}
			Typ::Map(k, v) => {
				let (keys, vals) = (self.map_entries(val, true, &k), self.map_entries(val, false, &v));
				self.temp(keys, &Typ::Array(k.clone()));
				self.temp(vals, &Typ::Array(v.clone()));
				let (kdata, len) = self.array_parts(keys, &Typ::Array(k.clone()));
				let vdata = self.array_data(vals);
				(len, (kdata, *k), Some((vdata, *v)))
			}
			_ => {
				return fail(format!("cannot iterate over {typ}"), span, "not iterable");
			}
		};
		let counter = self.b.declare_var(self.b.func.dfg.value_type(zero));
		self.b.def_var(counter, zero);

		let (header, latch, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
		self.b.ins().jump(header, &[]);

		self.b.switch_to_block(header);
		let iv = self.b.use_var(counter);
		let more = self.b.ins().icmp(IntCC::SignedLessThan, iv, limit);
		let (body_block, fallthrough) = self.fork(more);

		self.b.switch_to_block(body_block);
		let iv = self.b.use_var(counter);
		let (frame, flow) = self.in_loop(latch, Some(exit), Some(fallthrough), |s| {
			let (data, elem) = &src;
			let item = s.load_nth(*data, iv, elem);
			match (&vals, &pat.0) {
				(None, _) => s.bind_pat(pat, item, elem, Some(false))?,
				(Some((vdata, vt)), Expr::Tuple(te)) if te.len() == 2 => {
					let vv = s.load_nth(*vdata, iv, vt);
					s.bind_pat(&te[0].1, item, elem, Some(false))?;
					s.bind_pat(&te[1].1, vv, vt, Some(false))?;
				}
				_ => {
					return fail("destructure map entries with `(k, v)`", pat.1, "expected `(k, v)`");
				}
			}
			s.block(body)
		})?;

		if let Some((v, t)) = flow {
			self.release_value(v, &t);
			self.b.ins().jump(latch, &[]);
		}
		self.b.seal_block(latch);

		self.b.switch_to_block(latch);
		let iv = self.b.use_var(counter);
		let next = self.b.ins().iadd_imm(iv, 1);
		self.b.def_var(counter, next);
		self.b.ins().jump(header, &[]);
		self.b.seal_block(header);

		Ok(self.loop_end(frame, exit))
	}

	// Bind or assign a pattern's names against a value.
	pub(super) fn bind_pat(
		&mut self,
		pat: &Spanned<Expr>,
		val: Value,
		typ: &Typ,
		mutable: Option<bool>,
	) -> Result<(), Diagnostic> {
		let parts = !matches!(&pat.0, Expr::Ident(_));
		let binds = match &pat.0 {
			Expr::Ident(name) if name == "_" => return Ok(()),
			Expr::Ident(name) => vec![(name.clone(), typ.clone(), 0)],
			_ => self.pat_binds(pat, typ)?,
		};
		for (name, ftyp, off) in binds {
			let v = if parts {
				self.load_bind(val, typ, &ftyp, off, pat.1)
			} else {
				val
			};
			let v = self.copy_bind(v, &ftyp);
			let Some(mutable) = mutable else {
				let local = self.mutable_local(&name, pat.1.into_range(), Mutation::Assign)?;
				if local.typ != ftyp {
					let msg = format!("cannot assign {ftyp} to `{name}`, which is {}", local.typ);
					return fail(msg, pat.1, "type mismatch");
				}
				let old = self.read_local(&local);
				self.write_local(&local, v);
				self.release_value(old, &ftyp);
				continue;
			};
			self.bind_local(&name, v, ftyp, mutable);
		}
		Ok(())
	}

	// The names a pattern binds, with their offsets into the subject.
	// Tuple and struct offsets are in bytes, array offsets are element indices. I couldn't think of a nicer way to do it.
	pub(super) fn pat_binds(&self, pat: &Spanned<Expr>, typ: &Typ) -> Result<Vec<Bind>, Diagnostic> {
		match (&pat.0, typ) {
			(Expr::Tuple(elems), Typ::Tuple(fields)) => {
				if elems.len() != fields.len() {
					let msg = format!(
						"pattern binds {} names but the tuple has {} fields",
						elems.len(),
						fields.len()
					);
					return fail(msg, pat.1, "wrong number of fields");
				}
				field_binds(elems.iter().zip(fields).map(|((_, e), (_, t))| (e, t)), 0, 8)
			}
			(Expr::StructLit { name, fields, .. }, Typ::Struct(sname, fdefs)) => {
				for (fname, e) in fields {
					let Expr::Ident(local) = &e.0 else { continue };
					self.check_member(sname, fname.as_deref().unwrap_or(local), e.1)?;
				}
				struct_pattern(fdefs, &self.qualify(name), sname, fields, pat.1)
			}
			(Expr::Array(elems), Typ::Array(elem) | Typ::FixedArray(elem, _)) => {
				field_binds(elems.iter().map(|e| (e, &**elem)), 0, 1)
			}
			_ => fail(
				format!("cannot destructure {typ} with this pattern"),
				pat.1,
				"wrong shape",
			),
		}
	}

	// Load a name from `pat_binds` out of the subject.
	pub(super) fn load_bind(&mut self, subject: Value, typ: &Typ, field: &Typ, off: i32, span: Span) -> Value {
		match typ {
			Typ::Array(_) | Typ::FixedArray(..) => {
				let (data, len) = self.array_parts(subject, typ);
				let idx = self.b.ins().iconst(self.int, off as i64);
				self.load_index(data, len, field, idx, span)
			}
			_ => self.opt_payload(subject, typ, field, off),
		}
	}
}
