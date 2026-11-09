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
	throws: Vec<(Block, Block, TypedVal, Span)>,
}

// A header bind is a test only when its pattern can fail, otherwise it just binds.
fn infallible(cond: &Expr) -> bool {
	match cond {
		Expr::Bind { .. } => true,
		Expr::PatBind { pat, .. } => matches!(pat.0, Expr::Tuple(_) | Expr::Array(_) | Expr::StructLit { .. }),
		Expr::Binary(BinOp::And, lhs, rhs) => infallible(&lhs.0) && infallible(&rhs.0),
		Expr::Block(stmts) => !stmts.is_empty() && stmts.iter().all(|stmt| infallible(&stmt.0)),
		_ => false,
	}
}

// The clauses of a header chain.
fn chain(cond: &Spanned<Expr>) -> Vec<Spanned<Expr>> {
	match &cond.0 {
		Expr::Binary(BinOp::And, lhs, rhs) => [chain(lhs), chain(rhs)].concat(),
		Expr::Block(stmts) => stmts.iter().flat_map(chain).collect(),
		_ => vec![cond.clone()],
	}
}

// An `if` on a header chain's clauses, whose `else` takes a miss.
fn clause_if(cond: Spanned<Expr>, then: &[Spanned<Expr>], els: Option<&[Spanned<Expr>]>) -> Expr {
	let (cond, then, els) = (Box::new(cond), then.to_vec(), els.map(<[_]>::to_vec));
	Expr::If {
		cond,
		then,
		els,
		chain: true,
	}
}

// A `:=` header bind's body, behind a mutable copy of what it unwrapped.
fn mutable_copy(cond: &Spanned<Expr>, body: &[Spanned<Expr>]) -> Vec<Spanned<Expr>> {
	let Expr::Bind {
		name, mutable: true, ..
	} = &cond.0
	else {
		return body.to_vec();
	};
	let (name, value) = (name.clone(), Some(Box::new((Expr::Ident(name.clone()), cond.1))));
	let copy = Expr::Bind {
		mutable: true,
		name,
		typ: None,
		value,
	};
	[vec![(copy, cond.1)], body.to_vec()].concat()
}

// Whether a body ends on every path without falling out.
pub(super) fn diverges(body: &[Spanned<Expr>]) -> bool {
	match body.last().map(|stmt| &stmt.0) {
		Some(Expr::Return(_) | Expr::Break(..) | Expr::Continue(_)) => true,
		Some(Expr::MacroCall { name, .. }) => matches!(name.as_str(), "panic" | "todo" | "unreachable"),
		Some(Expr::Block(inner)) => diverges(inner),
		Some(Expr::If { then, els, .. }) => diverges(then) && els.as_deref().is_some_and(diverges),
		Some(Expr::Match { arms, else_body, .. }) => {
			arms.iter().all(|arm| diverges(&arm.body)) && else_body.as_deref().is_none_or(diverges)
		}
		_ => false,
	}
}

// Whether a condition contains a header chain.
fn chained(cond: &Expr) -> bool {
	match cond {
		Expr::Bind { .. } | Expr::PatBind { .. } | Expr::Block(_) => true,
		Expr::Binary(BinOp::And, lhs, rhs) => chained(&lhs.0) || chained(&rhs.0),
		_ => false,
	}
}

// Labels the inner levels of a `;` nest.
pub(super) const DIM: &str = "$dim";

// Whether a clause group has a generator.
fn has_gen(group: &Expr) -> bool {
	match group {
		Expr::Gen { .. } => true,
		Expr::Binary(BinOp::And, lhs, rhs) => has_gen(&lhs.0) || has_gen(&rhs.0),
		_ => false,
	}
}

type Pats<'e> = (&'e Spanned<Expr>, Option<&'e Spanned<Expr>>);

type Emit<'e, 'a, M> = Box<dyn FnOnce(&mut Translator<'a, M>) -> Result<(), Diagnostic> + 'e>;

// A generator's step at the top of each round, and its cleanup after the loop.
type Cursor<'e, 'a, M> = (Emit<'e, 'a, M>, Option<Emit<'e, 'a, M>>);

impl<'a, M: Module> Translator<'a, M> {
	// Lower branching control flow constructs.
	pub(super) fn branching(
		&mut self,
		expr: &Spanned<Expr>,
		hint: Option<&Typ>,
		want: bool,
	) -> Result<Option<TypedVal>, Diagnostic> {
		self.header_scoped(|s| match &expr.0 {
			Expr::If { .. } => s.conditional(expr, hint, want),
			Expr::Match {
				subject,
				arms,
				else_body,
			} => s.match_expr(subject, arms, else_body.as_deref(), None, hint, expr.1),
			Expr::Loop { label, cond, body } => {
				let want = want && label.as_deref() != Some(DIM);
				s.collected(want, hint, |s| s.loop_expr(label.as_deref(), cond.as_deref(), body))
			}
			Expr::Labeled { label, body } => s.looped(|s| s.labeled(label, body, expr.1)),
			_ => unreachable!(),
		})
	}

	// Header binds are visible to the construct alone and owned by the enclosing scope.
	fn header_scoped<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Diagnostic>) -> Result<T, Diagnostic> {
		let outer = (*self.vars).clone();
		let out = f(self);
		let fresh = self.vars.iter().filter(|(n, l)| outer.get(*n).is_none_or(|o| o.var != l.var));
		for name in fresh.map(|(n, _)| n.clone()).collect::<Vec<_>>() {
			match outer.get(&name) {
				Some(local) => self.vars.insert(name, local.clone()),
				None => self.vars.remove(&name),
			};
		}
		out
	}

	// Block headers share scope with their body chains.
	fn header<'e>(&mut self, h: &'e Spanned<Expr>) -> Result<&'e Spanned<Expr>, Diagnostic> {
		let Expr::Block(body) = &h.0 else { return Ok(h) };
		let [stmts @ .., tail] = &body[..] else { return Ok(h) };
		match self.block(stmts)? {
			Some(_) => self.header(tail),
			None => fail(
				"this block never produces a value",
				h.1,
				"every path returns, but a value is needed here",
			),
		}
	}

	// Chains a header block's binds and tests as clauses, and its other statements just run.
	fn clauses(&mut self, h: &Spanned<Expr>) -> Result<Spanned<Expr>, Diagnostic> {
		let Expr::Block(body) = &h.0 else { return Ok(h.clone()) };
		let (stmt, rest) = match &body[..] {
			[] => return Ok(h.clone()),
			[stmt] => return self.clauses(stmt),
			[stmt, rest @ ..] => (stmt, (Expr::Block(rest.to_vec()), h.1)),
		};
		let mut test = stmt.clone();
		if !chained(&stmt.0) {
			let never = || Diagnostic::new("this never produces a value", stmt.1.into_range());
			let (val, typ) = self.block(std::slice::from_ref(stmt))?.ok_or_else(never)?;
			if typ != Typ::Bool {
				return self.clauses(&rest);
			}
			let name = format!("$test{}", self.vars.len());
			self.hidden_local(name.clone(), val, typ);
			test.0 = Expr::Ident(name);
		}
		Ok((Expr::Binary(BinOp::And, Box::new(test), Box::new(rest)), h.1))
	}

	// `if`/`else` lowered to branch&merge, yielding value of the chosen branch.
	// A diverging branch contributes nothing to the merge.
	// If all branches diverge, returns None.
	pub(super) fn conditional(
		&mut self,
		(node, span): &Spanned<Expr>,
		target: Option<&Typ>,
		want: bool,
	) -> Result<Option<TypedVal>, Diagnostic> {
		let Expr::If { cond, then, els, chain } = node else {
			unreachable!()
		};
		let (els, span) = (els.as_deref(), *span);
		let cond = &self.clauses(cond)?;
		if let Expr::Binary(BinOp::And, lhs, rhs) = &cond.0
			&& chained(&cond.0)
		{
			let inner = (clause_if((**rhs).clone(), then, els), cond.1);
			let head = clause_if((**lhs).clone(), &[inner], els);
			return self.conditional(&(head, span), target, want);
		}

		if infallible(&cond.0) {
			let flow = self.scoped(|s| {
				// a local `?T` owning nothing unwraps in place, uncopied
				if let Expr::Bind {
					name,
					typ: None,
					value: Some(src),
					..
				} = &cond.0
					&& let Expr::Ident(local) = &src.0
					&& let Some((inner, None)) = s.vars.get(local).and_then(|l| s.fallible_split(&l.typ))
					&& !s.needs_release(&inner)
				{
					let arm = MatchArm {
						patterns: vec![(Expr::Ident(name.clone()), cond.1)],
						body: mutable_copy(cond, then),
						..Default::default()
					};
					return s.match_expr(src, &[arm], els, Some(&[]), target, span);
				}
				s.pack_operand(cond)?;
				if let Some((id, arm)) = s.unwrap_arm(cond, then) {
					return s.match_expr(&id, &[arm], els, Some(&[]), target, span);
				}

				if let Some([first, ..]) = els
					&& !chain
				{
					let msg = "this binding always succeeds, so `else` can never run";
					return fail(msg, first.1, "unreachable");
				}

				s.block_tail(then, target)
			})?;

			if let Some((v, t)) = &flow {
				// yields a temp, so a statement drops it
				self.temp(*v, t);
			}

			return Ok(flow);
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
		let then_flow = self.unwrapped(&cond.0, true, then, target)?;
		self.rejoin(&outer, &mut live, then_flow.is_none());
		let tail = self.branch_tail(&owned, &mut tails, then_flow.is_some());
		if let Some(vt) = then_flow {
			self.join_branch(want, vt, &mut join, tail)?;
		}

		self.b.switch_to_block(else_block);
		let else_flow = match els {
			Some(els) => self.unwrapped(&cond.0, false, els, target)?,
			None => self.else_default(None, &join, target)?,
		};
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

	// The locals unwrapped by a condition.
	fn unwrapping(&mut self, cond: &Expr, holds: bool) -> Vec<Unwrapping> {
		match cond {
			Expr::Binary(BinOp::And, lhs, rhs) if holds => {
				[self.unwrapping(&lhs.0, true), self.unwrapping(&rhs.0, true)].concat()
			}
			Expr::Is { subject, typ, negated } => self.unwrap_is(subject, typ, holds != *negated).into_iter().collect(),
			_ => vec![],
		}
	}

	// The type an `is` test unwraps its local to.
	fn unwrap_is(&mut self, subject: &Spanned<Expr>, te: &TypeExpr, holds: bool) -> Option<Unwrapping> {
		let (name, local) = match &subject.0 {
			Expr::Ident(name) => (name.clone(), self.vars.get(name)?.clone()),
			Expr::Dollar | Expr::Field { .. } => {
				let name = match &subject.0 {
					Expr::Dollar => "$".into(),
					e => place_key(e)?,
				};
				let (v, t) = self.expr(subject).ok()?;
				let var = self.b.declare_var(cl_type(&t, self.int));
				self.b.def_var(var, v);
				(name, Local::plain(var, t, false))
			}
			_ => return None,
		};
		let member = match te {
			TypeExpr::Name(n) if n.starts_with('.') => n[1..].to_string(),
			_ => self.sum_display(te, subject.1).ok()?,
		};
		let set: Vec<_> = (self.variants_of(&local.typ).into_iter())
			.filter(|v| (v.name == member) == holds)
			.collect();
		let t = match (&local.typ, &set[..]) {
			(Typ::Any | Typ::Trait(_) | Typ::Error, _) if holds => self.types.resolve(te, subject.1).ok()?,
			(Typ::Enum(_), [v]) if !v.payload.is_empty() => local.typ.refined(&v.name),
			(Typ::Sum(..), [v]) if enum_boxed(&set) => v.payload[0].clone(),
			(Typ::Sum(..), [_, _, ..]) if enum_boxed(&set) => Typ::Sum(String::new(), set),
			_ => return None,
		};
		Some((name, local, t))
	}

	// Ends unwraps written by a loop body.
	fn rewrap_writes(&mut self, body: &[Spanned<Expr>]) {
		let mut written = HashSet::new();
		Expr::Block(body.to_vec()).walk(&mut |e| match e {
			Expr::Assign { name, .. }
			| Expr::DerefAssign { name, .. }
			| Expr::FieldAssign { name, .. }
			| Expr::IndexAssign { name, .. }
			| Expr::Append { name, .. } => _ = written.insert(name.clone()),
			Expr::PatBind { pat, mutable: None, .. } => pat.0.idents(&mut written),
			Expr::ArgMod(Access::Mut, place) | Expr::MethodCall { recv: place, .. } => {
				written.extend(place_key(&Self::place_root(place).0))
			}
			_ => {}
		});
		written.iter().for_each(|root| self.vars.forget(root));
	}

	// A branch that sees its condition's subjects unwrapped.
	pub(super) fn unwrapped(
		&mut self,
		cond: &Expr,
		holds: bool,
		body: &[Spanned<Expr>],
		target: Option<&Typ>,
	) -> Result<Option<TypedVal>, Diagnostic> {
		let unwrapping = self.unwrapping(cond, holds);
		if unwrapping.is_empty() {
			return self.branch(body, target);
		}
		self.scoped(|s| {
			s.rebind_unwrapped(unwrapping)?;
			s.block_tail(body, target)
		})
	}

	// Rebinds each subject to its unwrapped type.
	fn rebind_unwrapped(&mut self, unwrapping: impl IntoIterator<Item = Unwrapping>) -> Result<(), Diagnostic> {
		for (name, mut local, t) in unwrapping {
			let view = if matches!(t, Typ::Sum(..) | Typ::Annotated(..)) && local.typ != Typ::Any {
				Local { typ: t, ..local }
			} else {
				let object = matches!(local.typ, Typ::Trait(_) | Typ::Error);
				local.mutable &= !object;
				let mut boxp = self.read_local(&local);
				if local.mutable {
					// unshare the box so writes land in it alone
					let payload = self.ld_typ(boxp, 8, &t);
					let payload = self.copy_in(payload, &t);
					let fresh = self.coerce(payload, &t, &local.typ, Span::default())?.0;
					self.untemp(fresh);
					self.write_local(&local, fresh);
					self.release_value(boxp, &local.typ);
					boxp = fresh;
				}
				let payload = match object && t.slots().is_none() {
					true => self.ld_word(boxp, 8),
					false => self.b.ins().iadd_imm(boxp, 8),
				};
				let view = Local {
					boxed: true,
					..Local::plain(self.b.declare_var(self.int), t, local.mutable)
				};
				self.b.def_var(view.var, payload);
				view
			};
			if name == "$" {
				self.dollar = Some((self.read_local(&view), view.typ));
			} else {
				self.vars.insert(name, view);
			}
		}
		Ok(())
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
			let v = if self.types.fallible(&t) {
				self.zero(&t)
			} else {
				self.zero_or_err(&t, join.span)?
			};
			return Ok(Some((v, t)));
		};
		self.branch(els, target)
	}

	// A branch that is a lone `defer` arms it on the enclosing scope.
	fn branch(&mut self, body: &[Spanned<Expr>], target: Option<&Typ>) -> Result<Option<TypedVal>, Diagnostic> {
		match body {
			[(Expr::Defer { label, body, when }, _)] => Ok(Some(self.defer(label, body, *when, true))),
			_ => self.scoped(|s| s.block_tail(body, target)),
		}
	}

	// Evaluate `f` in a child scope.
	pub(super) fn scoped(
		&mut self,
		f: impl FnOnce(&mut Self) -> Result<Option<TypedVal>, Diagnostic>,
	) -> Result<Option<TypedVal>, Diagnostic> {
		self.vars.mark();
		let (withs, type_params, dollar) = (self.withs.len(), self.types.type_params, self.dollar.clone());
		self.scopes.push(vec![]);
		self.defers.push(vec![]);
		let flow = f(self);
		self.vars.undo();
		self.types.type_params = type_params;
		self.withs.truncate(withs);
		self.dollar = dollar;
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

	pub(super) fn finish_merge(&mut self, merge: Block, result: Option<(Variable, Typ)>) -> Option<TypedVal> {
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
		let subject = self.header(subject)?;
		if matches!(subject.0, Expr::PatBind { .. }) && !infallible(&subject.0) {
			let note = "use `if` to test it, or move the pattern into an arm";
			return fail("a `match` header can't refute", subject.1, note);
		}
		let (sv, st) = self.flow_operand(subject)?;
		let (sv, st) = self.deref(sv, &st);

		// a non-enum match has no coverage to analyse, so it is all a gap
		if !st.is_enumish() && else_body.is_none() {
			else_body = gap.filter(|g| !g.is_empty());
		}

		// ensure match covers every variant when applicable
		if st.is_enumish() {
			let unguarded = || arms.iter().filter(|a| a.guard.is_none()).flat_map(|a| &a.patterns);
			let pats = || unguarded().filter(|p| !refutable_subs(p));
			let catch_all = else_body.is_some() || pats().any(|p| matches!(&p.0, Expr::Ident(w) if w == "_"));
			if !catch_all && st == Typ::Any {
				return Err(Diagnostic::new("a match on `any` needs `else`", span.into_range()));
			}
			if !catch_all {
				let variants = self.variants_of(&st);
				let covered = pats()
					.map(|p| self.enum_pattern(&split_subs(p).0, &st).map(|(d, _)| d))
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
		let bodies: Vec<Block> = arms.iter().map(|_| self.b.create_block()).collect();
		let else_blk = self.b.create_block();
		self.b.ins().jump(arm_entries.first().copied().unwrap_or(else_blk), &[]);

		for (i, arm) in arms.iter().enumerate() {
			let arm_body = bodies[i];
			let miss = arm_entries.get(i + 1).copied().unwrap_or(else_blk);

			self.b.switch_to_block(arm_entries[i]);
			self.b.seal_block(arm_entries[i]);

			// bindings
			let (alone, mut binds) = (arm.patterns.len() == 1, vec![]);
			for (j, pat) in arm.patterns.iter().enumerate() {
				let last = j + 1 == arm.patterns.len();
				let next = if last { miss } else { self.b.create_block() };
				let (hit, found) = self.pat_test(pat, (sv, &st), alone, next)?;
				binds = found;
				self.b.ins().brif(hit, arm_body, &[], next, &[]);
				if !last {
					self.b.seal_block(next);
					self.b.switch_to_block(next);
				}
			}

			self.b.seal_block(arm_body);
			self.b.switch_to_block(arm_body);

			let unwrapping = match &arm.patterns[..] {
				[(Expr::Is { typ, .. }, _)] => self.unwrap_is(subject, typ, true),
				_ => None,
			};
			let binds_names = binds.iter().any(|(_, names)| !names.is_empty());
			if i > 0 && falls(&arms[i - 1]) && (unwrapping.is_some() || binds_names) {
				let msg = "`fallthrough` into an arm that binds names";
				return fail(msg, arm.patterns[0].1, "nothing to bind");
			}

			let fell = falls(arm);
			if fell && i + 1 == arms.len() && else_body.is_none() {
				return fail(
					"`fallthrough` in the last arm",
					arm.body[arm.body.len() - 1].1,
					"no next arm",
				);
			}

			let flow = self.scoped(|s| {
				s.rebind_unwrapped(unwrapping)?;
				if arm.with {
					let Expr::Ident(name) = &subject.0 else {
						return fail("`with` needs a named subject", arm.patterns[0].1, "ex: `match x := ..`");
					};
					s.withs.push((name.clone(), None));
				}
				for ((base, base_typ), list) in &binds {
					for (name, typ, off) in list {
						let field = s.load_bind(*base, base_typ, typ, *off, span);
						let field = s.copy_bind(field, typ);
						s.bind_local(name, field, typ.clone(), false);
					}
				}
				if let Some(guard) = &arm.guard {
					let pass = s.bool_value(guard, "match guard")?;
					let (yes, no) = s.fork(pass);
					s.b.switch_to_block(no);
					s.release_scopes(s.scopes.len() - 1, None)?;
					s.b.ins().jump(miss, &[]);
					s.b.switch_to_block(yes);
				}
				match fell {
					true => s.block_tail(&arm.body[..arm.body.len() - 1], None),
					false => s.block_tail(&arm.body, target),
				}
			})?;

			let flow = match (fell, flow) {
				(true, Some((v, t))) => {
					self.release_value(v, &t);
					self.b.ins().jump(bodies.get(i + 1).copied().unwrap_or(else_blk), &[]);
					None
				}
				(_, flow) => flow,
			};

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

	// Tests a pattern against the subject.
	fn pat_test(
		&mut self,
		pat: &Spanned<Expr>,
		(sv, st): (Value, &Typ),
		alone: bool,
		miss: Block,
	) -> Result<(Value, Vec<ArmBinds>), Diagnostic> {
		let yes = |s: &mut Self| s.b.ins().iconst(types::I8, 1);
		let subject = |binds| Some(((sv, st.clone()), binds));
		let (pat, subs) = &split_subs(pat);
		let (mut hit, found) = match &pat.0 {
			Expr::Ident(w) if w == "_" => (yes(self), None),
			_ if let Some(bounds) = pat.0.bounds() => (self.range_pattern(sv, st, bounds, pat.1)?, None),
			_ if st.is_enumish() => {
				let (disc, binds) = self.enum_pattern(pat, st)?;
				let tag = self.enum_tag(st, sv);
				(self.b.ins().icmp_imm(IntCC::Equal, tag, disc), subject(binds))
			}
			Expr::Is { .. } => {
				let outer = self.dollar.replace((sv, st.clone()));
				let hit = self.expr(pat);
				self.dollar = outer;
				(hit?.0, None)
			}
			Expr::Tuple(_) | Expr::StructLit { .. } | Expr::Array(_) => {
				let binds = self.pat_binds(pat, st)?;
				let hit = match &pat.0 {
					Expr::Array(elems) => {
						let (_, len) = self.array_parts(sv, st);
						self.b.ins().icmp_imm(IntCC::Equal, len, elems.len() as i64)
					}
					_ => yes(self),
				};
				(hit, subject(binds))
			}
			Expr::Quote(q) if *st == Typ::Ast => {
				if !alone || q.len() != 1 {
					let msg = "a quote pattern is a single expression, alone in its arm";
					return Err(Diagnostic::new(msg, pat.1.into_range()));
				}
				let (tpl, slots) = expand::register(q, pat.1)?;
				let names: Vec<(String, Typ)> = (slots.into_iter())
					.map(|slot| match slot {
						expand::Slot::Name(name) => Some((name, Typ::Ast)),
						expand::Slot::Splat((Expr::Ident(name), _)) => Some((name, Typ::Array(Box::new(Typ::Ast)))),
						_ => None,
					})
					.collect::<Option<_>>()
					.ok_or_else(|| {
						let msg = "only `%name` and `%{..name}` captures are allowed in a quote pattern";
						Diagnostic::new(msg, pat.1.into_range())
					})?;
				let outs = self.stack_slot((names.len().max(1) * 8) as u32);
				let tplv = self.b.ins().iconst(self.int, tpl as i64);
				let matched = self.extern_call(expand::RT_QUOTE_MATCH, &[tplv, sv, outs], Some(self.int));
				let hit = self.b.ins().icmp_imm(IntCC::NotEqual, matched.unwrap(), 0);
				let binds = names.into_iter().enumerate().map(|(i, (name, typ))| (name, typ, i as i32 * 8));
				(hit, Some(((outs, Typ::Ast), binds.collect())))
			}
			_ => {
				let (pv, pt) = self.check_expr(pat, st)?;
				if pt != *st {
					let msg = format!("match pattern ({pt}) does not match subject ({st})");
					return fail(msg, pat.1, "type mismatch");
				}
				(self.emit_eq(sv, pv, st), None)
			}
		};

		// test each sub-pattern's field once the outer pattern holds
		let mut out = vec![];
		if let Some((base, binds)) = found {
			let (hidden, named): (Vec<_>, _) = binds.into_iter().partition(|(name, ..)| name.starts_with("$sub"));
			out.push((base.clone(), named));
			for ((_, field_typ, off), sub) in hidden.iter().zip(subs) {
				let holds = self.b.create_block();
				self.b.ins().brif(hit, holds, &[], miss, &[]);
				self.b.seal_block(holds);
				self.b.switch_to_block(holds);
				let field = self.load_bind(base.0, &base.1, field_typ, *off, sub.1);
				let (sub_hit, sub_binds) = self.pat_test(sub, (field, field_typ), alone, miss)?;
				hit = sub_hit;
				out.extend(sub_binds);
			}
		}
		Ok((hit, if alone { out } else { vec![] }))
	}

	// Write (v, t) into the shared result variable and jump to `merge`.
	// All branches must agree on type. The first one declares the variable.
	pub(super) fn contribute(&mut self, (v, t): TypedVal, join: &mut Join, merge: Block) -> Result<(), Diagnostic> {
		// var owned by the merged value
		self.untemp(v);

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

	// A propagator payload, and the error type if it's a Result or partial pack.
	pub(super) fn fallible_split(&self, typ: &Typ) -> Option<(Typ, Option<Typ>)> {
		if let (Some((lead, opt)), Typ::Tuple(fields)) = (self.trailing(typ), typ) {
			return Some((lead_typ(&fields[..lead]), self.types.option_inner(opt)));
		}
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
			throws: vec![],
		};

		// a pipeline is a try scope, with its `?` steps landing here
		let lowered = if let Expr::Pipe { .. } = value.0 {
			let held = self.scopes.last().cloned().unwrap_or_default();
			let outer = self.catch.replace(catch);
			let lowered = self.expr(value);
			catch = std::mem::replace(&mut self.catch, outer).expect("set above");

			let late = self.scopes.last().into_iter().flatten().filter(|l| !held.contains(l));
			self.flagged.extend(late.map(|(v, _)| *v));
			lowered
		} else {
			self.take_operand(value)
		};
		let (val, typ) = lowered?;

		let (payload, inner) = match self.fallible_split(&typ) {
			Some((inner, err)) => {
				let tag = self.enum_tag(&typ, val);
				let is_happy = self.b.ins().icmp_imm(IntCC::Equal, tag, err.is_none() as i64);
				let (happy_block, sad_block) = self.fork(is_happy);
				self.b.switch_to_block(sad_block);
				let err = match err {
					Some(e) => (self.flow_err(val, &typ, &e), e),
					None => self.unit_value(),
				};
				self.throw(&mut catch, err, value.1)?;
				self.b.switch_to_block(happy_block);
				(self.take_payload(val, &typ, &inner), inner)
			}
			None if !catch.throws.is_empty() => (val, typ),
			None => {
				let msg = format!("`or` needs a `?T`/`!T` value, got {typ}");
				return fail(msg, value.1, "not an Option or Result");
			}
		};
		let merge = self.b.create_block();
		let payload = self.copy_bind(payload, &inner);
		let happy_tail = self.b.create_block();
		self.b.ins().jump(happy_tail, &[]);
		self.b.seal_block(happy_tail);

		let (var, err) = self.land(&catch)?;
		self.b.switch_to_block(catch.block);
		self.b.seal_block(catch.block);
		let saved_dollar = self.dollar.replace((self.b.use_var(var), err));
		let flow = self.branch(body, want.then_some(&inner))?;
		self.dollar = saved_dollar;
		let target = match &flow {
			Some((_, t)) if !want || *t == inner => inner.clone(),
			Some((_, t)) if *t == Typ::Error => self.types.core_enum(role::RESULT, &[inner.clone(), Typ::Error]),
			Some((_, t)) if self.fallible_split(t).is_some_and(|(i, _)| i == inner) => t.clone(),
			_ => inner.clone(),
		};
		let seed = self.b.declare_var(cl_type(&target, self.int));
		let mut join = Join::new("or", span, Some((seed, target.clone())));
		if let Some((v, t)) = flow {
			let vt = self.coerce(v, &t, &target, span)?;
			self.join_branch(want, vt, &mut join, merge)?;
		}

		self.b.switch_to_block(happy_tail);
		let happy = self.coerce(payload, &inner, &target, span)?;
		self.join_branch(want, happy, &mut join, merge)?;

		if !want {
			self.b.switch_to_block(merge);
			self.b.seal_block(merge);
			return Ok(self.unit_value());
		}
		Ok(self.finish_merge(merge, join.result).expect("`or` always yields"))
	}

	// Lowers an unwrap operand, moving out a binding with an affine payload.
	pub(super) fn take_operand(&mut self, value: &Spanned<Expr>) -> Result<TypedVal, Diagnostic> {
		let (val, typ) = self.pack_operand(value)?;
		if let Expr::Ident(_) = value.0
			&& self.enum_box(&typ)
			&& self.fallible_split(&typ).is_some_and(|(t, _)| self.is_affine(&t))
		{
			self.move_out(value, &typ)?;
			self.temp(val, &typ);
		}
		Ok((val, typ))
	}

	// A some payload, moved out of an owned box or pack if affine.
	fn take_payload(&mut self, val: Value, typ: &Typ, inner: &Typ) -> Value {
		let payload = self.flow_payload(val, typ, inner);
		let Some(&owner) = self.temps.get(&val).filter(|_| self.is_affine(inner)) else {
			return payload;
		};

		if self.enum_box(typ) {
			let gone = self.b.ins().iconst(self.int, -1);
			self.st(val, 0, gone);
			return self.unbox_ret(payload, inner);
		}

		// a happy pack holds nothing but its payload
		if self.trailing(typ).is_some() {
			let null = self.b.ins().iconst(self.int, 0);
			self.b.def_var(owner, null);
			self.flagged.push(owner);
		}
		payload
	}

	// Take a happy `?T` or `!T` payload, else make a zero.
	pub(super) fn payload_or_zero(&mut self, val: Value, typ: &Typ, span: Span) -> Result<Value, Diagnostic> {
		let (inner, err) = self.fallible_split(typ).expect("a `?T` or `!T`");
		let tag = self.enum_tag(typ, val);
		let happy = self.b.ins().icmp_imm(IntCC::Equal, tag, err.is_none() as i64);

		// a shared payload, copied in by the caller
		if !self.is_affine(&inner) {
			let payload = self.flow_payload(val, typ, &inner);
			return Ok(self.or_zero(happy, payload, &inner, span)?.0);
		}

		let (out, join) = (self.b.declare_var(cl_type(&inner, self.int)), self.b.create_block());

		let (take, sad) = self.fork(happy);
		self.b.switch_to_block(sad);

		let zero = self.zero_or_err(&inner, span)?;
		self.b.def_var(out, zero);
		self.b.ins().jump(join, &[]);
		self.b.switch_to_block(take);

		let payload = self.take_payload(val, typ, &inner);
		self.b.def_var(out, payload);
		self.enter(join);
		Ok(self.b.use_var(out))
	}

	// Jump to an `or` fallback, via a conversion block filled in once `$` is known.
	fn throw(&mut self, catch: &mut Catch, err: TypedVal, span: Span) -> Result<(), Diagnostic> {
		let (conv, rel) = (self.b.create_block(), self.b.create_block());
		self.b.ins().jump(conv, &[]);
		self.b.seal_block(conv);
		self.b.switch_to_block(rel);
		self.release_scopes(catch.depth, None)?;
		self.b.ins().jump(catch.block, &[]);
		catch.throws.push((conv, rel, err, span));
		Ok(())
	}

	// `$` is the first caught error type if the rest fit it, otherwise Error.
	fn land(&mut self, catch: &Catch) -> Result<(Variable, Typ), Diagnostic> {
		let first = &catch.throws[0].2.1;
		let fits = (catch.throws.iter()).all(|(.., (_, t), s)| self.err_into(t, first, String::new(), *s).is_ok());
		let want = if fits { first.clone() } else { Typ::Error };
		let var = self.b.declare_var(cl_type(&want, self.int));
		for (conv, rel, (v, t), span) in &catch.throws {
			self.b.switch_to_block(*conv);
			if t.is_unit() != want.is_unit() {
				return fail("cannot catch both `?T` and `!T`", *span, format!("this throws {t}"));
			}
			let v = match self.err_into(t, &want, format!("cannot catch {t} as {want}"), *span)? {
				Some(sig) => self.emit_call(&sig, &[*v]).0,
				None => self.coerce(*v, t, &want, *span)?.0,
			};
			self.b.def_var(var, v);
			self.b.ins().jump(*rel, &[]);
			self.b.seal_block(*rel);
		}
		Ok((var, want))
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
		let Some(((_, Some(err)), (_, Some(target_err)))) = self.fallible_split(typ).zip(self.fallible_split(target))
		else {
			return Ok(match self.trailing(target) {
				Some(_) => self.owned_zero(target),
				None => self.make_option(target, None),
			});
		};
		let e = self.flow_err(val, typ, &err);
		let e = match from {
			Some(sig) => self.emit_call(sig, &[e]).0,
			None => self.coerce(e, &err, &target_err, span)?.0,
		};
		if let Some((lead, opt)) = self.trailing(target) {
			let (pack, some) = (self.zero(target), self.make_option(opt, Some(e)));
			self.untemp(some);
			self.st(pack, lead as i32 * 8, some);
			return Ok(pack);
		}
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
		let (val, typ) = self.pack_operand(value)?;
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
		let payload = self.flow_payload(val, &typ, &inner);
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
		let (val, typ) = self.take_operand(value)?;
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
			Some(d) if is_result && let Some((t, Some(e))) = self.fallible_split(d) => {
				from = self.err_into(
					&err_typ,
					&e,
					format!("cannot propagate `{err_typ}` into a fn returning {d}"),
					span,
				)?;
				target_err = e;
				t
			}
			Some(d) if !is_result && let Some((t, None)) = self.fallible_split(d) => t,
			Some(other) => {
				let msg = format!("`?` needs an enclosing fn returning `{shape}`, found {other}");
				return fail(msg, span, format!("not a `{shape}` fn"));
			}
			None => inner.clone(),
		};
		let target_typ = match (is_result, declared) {
			(_, Some(d)) if self.trailing(&d).is_some() => d,
			(true, _) => self.types.core_enum(role::RESULT, &[target.clone(), target_err.clone()]),
			(false, _) => self.types.core_enum(role::OPTION, std::slice::from_ref(&target)),
		};

		let tag = self.enum_tag(&typ, val);
		let is_happy = self.b.ins().icmp_imm(IntCC::Equal, tag, !is_result as i64);

		let (happy_block, sad_block) = self.fork(is_happy);

		self.b.switch_to_block(sad_block);
		if let Some(mut catch) = self.catch.take() {
			let err = match is_result {
				true => (self.flow_err(val, &typ, &err_typ), err_typ.clone()),
				false => self.unit_value(),
			};
			self.throw(&mut catch, err, span)?;
			self.catch = Some(catch);
		} else if panic_in_main {
			let msg = if is_result {
				let e = self.flow_err(val, &typ, &err_typ);
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
		let payload = self.take_payload(val, &typ, &inner);
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
		self.release_value(val, typ);
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
		label: Option<&str>,
		cond: Option<&Spanned<Expr>>,
		body: &[Spanned<Expr>],
	) -> Result<Option<TypedVal>, Diagnostic> {
		// `;` nests, each generator group after the first opens an inner level
		if let Some((Expr::Block(groups), span)) = cond
			&& groups.iter().any(|group| has_gen(&group.0))
		{
			let and = |lhs, rhs| (Expr::Binary(BinOp::And, Box::new(lhs), Box::new(rhs)), *span);
			let mut levels = groups
				.chunk_by(|_, next| !has_gen(&next.0))
				.map(|level| level.iter().cloned().reduce(and));
			let outer = levels.next().flatten();
			let body = levels.rev().fold(body.to_vec(), |body, cond| {
				let (label, cond) = (Some(DIM.into()), cond.map(Box::new));
				vec![(Expr::Loop { label, cond, body }, *span)]
			});
			return self.loop_expr(label, outer.as_ref(), &body);
		}
		self.rewrap_writes(body);
		let mut gens = vec![];
		let rest = cond.map(|c| self.generators(c, &mut gens)).transpose()?.flatten();
		let (steps, drops): (Vec<_>, Vec<_>) = gens.into_iter().unzip();
		let zipped = !steps.is_empty();
		let top = self.b.create_block();
		self.b.ins().jump(top, &[]);
		self.b.switch_to_block(top);
		let cond = rest.as_ref().map(|c| self.clauses(c)).transpose()?;
		let cond = cond.as_ref();

		// a conditional loop branches, into the body or out through `fallthrough`
		let (exit, fallthrough) = match cond {
			None => (None, None),
			Some((c, _)) if zipped || infallible(c) || chained(c) => (None, None),
			Some(cond) => {
				let (cv, ct) = self.expr(cond)?;
				if ct != Typ::Bool {
					// non-bool headers are iterated
					self.b.seal_block(top);
					self.hidden_local("$iter".into(), cv, ct);
					let ident = |name: &str| Box::new((Expr::Ident(name.into()), cond.1));
					let iter = Expr::Gen {
						pat: ident("_"),
						key: None,
						iter: ident("$iter"),
					};
					return self.loop_expr(label, Some(&(iter, cond.1)), body);
				}

				let exit = self.b.create_block();
				let (body_block, fallthrough) = (self.b.create_block(), self.b.create_block());
				self.b.ins().brif(cv, body_block, &[], fallthrough, &[]);
				self.b.seal_block(body_block);
				self.b.switch_to_block(body_block);

				(Some(exit), Some(fallthrough))
			}
		};

		// a body expression that can fall through ends the loop when it does
		let (frame, _) = self.in_loop(label, Some(top), exit, fallthrough, |s| {
			for step in steps {
				step(s)?;
			}

			// breaks yield an Option once a clause can fail
			if cond.is_some_and(|c| chained(&c.0) && !infallible(&c.0)) {
				s.ended();
			}

			let end = |span| [(Expr::Break(None, None), span)];
			match (cond, body) {
				// binds run in turn until one can miss
				(Some(c), _) if infallible(&c.0) => {
					let clauses = chain(c);
					for (at, clause) in clauses.iter().enumerate() {
						s.pack_operand(clause)?;
						let Some((id, mut arm)) = s.unwrap_arm(clause, body) else {
							continue;
						};
						if let rest @ [_, ..] = &clauses[at + 1..] {
							let then = std::mem::take(&mut arm.body);
							arm.body = vec![(
								clause_if((Expr::Block(rest.to_vec()), c.1), &then, Some(&end(c.1))),
								c.1,
							)];
						}
						s.ended();
						return s.match_expr(&id, &[arm], None, Some(&end(c.1)), None, c.1);
					}
					s.block(body)
				}
				(Some((Expr::PatBind { pat, value, .. }, sp)), _) => {
					let arm = MatchArm {
						patterns: vec![(**pat).clone()],
						body: body.to_vec(),
						..Default::default()
					};
					s.match_expr(value, &[arm], None, Some(&end(*sp)), None, *sp)
				}
				(Some(c), _) if zipped || chained(&c.0) => {
					s.conditional(&(clause_if(c.clone(), body, Some(&end(c.1))), c.1), None, true)
				}
				(_, [(Expr::Match { subject, arms, .. }, sp)]) if !zipped => {
					s.match_expr(subject, arms, None, Some(&end(*sp)), None, *sp)
				}
				(Some(c), _) => s.unwrapped(&c.0, true, body, None),
				(None, _) => s.block(body),
			}
		})?;

		if let Some(ended) = frame.fallthrough {
			self.b.seal_block(ended);
		}

		self.b.seal_block(top);

		let Some(exit) = frame.exit else {
			// an infinite loop with no break never falls through
			return Ok(None);
		};
		let out = self.loop_end(frame, exit);
		for drop in drops.into_iter().flatten() {
			drop(self)?;
		}
		Ok(Some(out))
	}

	// Set up a header's generators, leaving its binds and tests in order.
	fn generators<'e>(
		&mut self,
		cond: &'e Spanned<Expr>,
		gens: &mut Vec<Cursor<'e, 'a, M>>,
	) -> Result<Option<Spanned<Expr>>, Diagnostic> {
		Ok(match &cond.0 {
			Expr::Gen { pat, key, iter } => {
				self.for_loop((pat, key.as_deref()), iter, gens)?;
				None
			}
			Expr::Binary(BinOp::And, lhs, rhs) => match (self.generators(lhs, gens)?, self.generators(rhs, gens)?) {
				(Some(lhs), Some(rhs)) => Some((Expr::Binary(BinOp::And, Box::new(lhs), Box::new(rhs)), cond.1)),
				(lhs, rhs) => lhs.or(rhs),
			},
			_ => Some(cond.clone()),
		})
	}

	// `v := opt` re-matches `v` to bind the payload, and a pack's `v` views its leading slots.
	fn unwrap_arm(&mut self, cond: &Spanned<Expr>, body: &[Spanned<Expr>]) -> Option<(Spanned<Expr>, MatchArm)> {
		let Expr::Bind { name, .. } = &cond.0 else { return None };
		let local = self.vars.get(name)?.clone();
		let (inner, err) = self.fallible_split(&local.typ)?;
		let (mut id, mut pat) = (Expr::Ident(name.clone()), Expr::Ident(name.clone()));
		if self.trailing(&local.typ).is_some() {
			let pack = self.read_local(&local);
			let tag = self.enum_tag(&local.typ, pack);
			let payload = self.flow_payload(pack, &local.typ, &inner);
			self.hidden_local(name.clone(), payload, inner);
			let flag = format!("$err{}", self.vars.len());
			self.hidden_local(flag.clone(), tag, Typ::Int(64));
			(id, pat) = (Expr::Ident(flag), Expr::Int(err.is_none() as i128));
		}
		let arm = MatchArm {
			patterns: vec![(pat, cond.1)],
			body: mutable_copy(cond, body),
			..Default::default()
		};
		Some(((id, cond.1), arm))
	}

	// A labeled block.
	fn labeled(&mut self, label: &str, body: &[Spanned<Expr>], span: Span) -> Result<Option<TypedVal>, Diagnostic> {
		let exit = self.b.create_block();
		let (mut frame, flow) = self.in_loop(Some(label), None, Some(exit), None, |s| s.block_tail(body, None))?;
		if let Some(vt) = flow {
			let mut join = Join::new("break", span, frame.result.take());
			self.contribute(vt, &mut join, exit)?;
			frame.result = join.result;
		}
		// nothing reaches `exit` when every path returns
		Ok(frame.result.is_some().then(|| self.loop_end(frame, exit)))
	}

	// Push a loop frame, run `body` in a child scope, then pop the frame.
	fn in_loop(
		&mut self,
		label: Option<&str>,
		top: Option<Block>,
		exit: Option<Block>,
		fallthrough: Option<Block>,
		body: impl FnOnce(&mut Self) -> Result<Option<TypedVal>, Diagnostic>,
	) -> Result<(LoopFrame, Option<TypedVal>), Diagnostic> {
		let depth = self.scopes.len();
		self.loops.push(LoopFrame {
			label: label.map(str::to_owned),
			top,
			exit,
			depth,
			result: None,
			fallthrough,
			collect: top.and_then(|_| self.collect.take()).map(|h| (h, Typ::unit())),
		});
		let flow = self.scoped(body)?;
		if let (Some(top), Some(vt)) = (top, flow.clone()) {
			// a round's tail goes back to `top`, and an inner `;` level's goes back to the loop it nests in
			let at = self.loops.iter().rposition(|f| f.label.as_deref() != Some(DIM));
			self.loop_tail(at.expect("loop frame"), vt);
			self.b.ins().jump(top, &[]);
		}
		Ok((self.loops.pop().expect("loop frame"), flow))
	}

	// Loops collect the tails of their bodies.
	pub(super) fn collected<T>(
		&mut self,
		want: bool,
		hint: Option<&Typ>,
		f: impl FnOnce(&mut Self) -> Result<T, Diagnostic>,
	) -> Result<T, Diagnostic> {
		let hdr = (want && !hint.is_some_and(Typ::is_unit)).then(|| {
			let z = self.b.ins().iconst(self.int, 0);
			self.make_array(z, z, &Typ::Array(Box::new(Typ::unit())))
		});
		let outer = std::mem::replace(&mut self.collect, hdr);
		let out = self.looped(f);
		self.collect = outer;
		out
	}

	// Push a tail onto the loop's array, or release it.
	fn loop_tail(&mut self, at: usize, (v, t): TypedVal) {
		let frame = &self.loops[at];
		let valued = frame.result.as_ref().is_some_and(|(_, r)| !r.is_unit());
		match frame.collect {
			Some((hdr, _)) if !t.is_unit() && !valued => {
				let moved = self.handover(v, &t);
				let owned = self.copy_in(v, &t);
				if owned != v && !moved {
					self.release_value(v, &t);
				}
				self.push(hdr, owned, &t);
				self.loops[at].collect = Some((hdr, t));
			}
			_ => self.release_value(v, &t),
		}
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
				self.untemp(v);
				self.b.def_var(*var, v);
			}
			self.b.ins().jump(exit, &[]);
		}
		self.b.seal_block(exit);
		self.b.switch_to_block(exit);
		if let Some((hdr, elem)) = frame.collect {
			self.untemp(hdr);
			if !elem.is_unit() {
				let t = Typ::Array(Box::new(elem));
				self.temp(hdr, &t);
				return (hdr, t);
			}
			self.rt_call("array_release", &[hdr]);
		}
		match frame.result {
			Some((var, t)) => {
				let v = self.b.use_var(var);
				self.temp(v, &t);
				(v, t)
			}
			None => self.unit_value(),
		}
	}

	// Set up an Iterator, or a Keyed walked by its key.
	fn iter_loop<'e>(
		&mut self,
		(pat, key): Pats<'e>,
		(val, typ): TypedVal,
		span: Span,
	) -> Result<Emit<'e, 'a, M>, Diagnostic> {
		let keyed = self.claims(&typ, role::KEYED) && (key.is_some() || !self.claims(&typ, role::ITERATOR));
		if let Some(key) = key.filter(|_| !keyed) {
			return fail(format!("{typ} is not keyed"), key.1, "not keyed");
		}

		let mut method = |name: &str| {
			(self.find_fill(&format!("{}.{name}", typ.key()), 0, &typ)).or_else(|| self.resolve_method(&typ, name))
		};
		let step = if keyed { "pairs" } else { "next" };
		let (next, key_at) = (method(step), key.and_then(|_| method("key")));
		let item = next.as_ref().and_then(|n| self.types.option_inner(&n.ret));
		let (Some(next), Some(item)) = (next, item) else {
			let note = format!("no `{step}` on its claim");
			return fail(format!("cannot iterate over {typ}"), span, note);
		};

		// make the cursor
		let after = match &item {
			Typ::Tuple(fields) if keyed => {
				let opt_key = self.types.core_enum(role::OPTION, std::slice::from_ref(&fields[1].1));
				let none = self.make_option(&opt_key, None);
				self.untemp(none);
				let var = self.b.declare_var(self.int);
				self.b.def_var(var, none);
				self.own_local(var, &opt_key);
				Some((var, opt_key))
			}
			_ => None,
		};

		// `mut self` without inline slots
		let it = match (next.access[0] == Access::Mut, typ.slots().is_some()) {
			(true, false) => self.frame_slots(&[val]),
			(true, true) => {
				let copy = self.copy_bind(val, &typ);
				self.temp(copy, &typ);
				copy
			}
			_ => val,
		};

		Ok(Box::new(move |s: &mut Self| {
			// keeps a plain `?T` as its pack
			let outer = s.pack_call.replace(span);
			let mut args = vec![it];
			args.extend(after.as_ref().map(|(var, _)| s.b.use_var(*var)));
			let (yielded, opt) = s.emit_call_at(&next, &args, Some(span));
			s.pack_call = outer;

			let tag = s.enum_tag(&opt, yielded);
			let (body_block, done) = s.fork(tag);

			// each pass owns what gets yielded by `next`
			s.untemp(yielded);
			s.b.switch_to_block(done);
			s.release_value(yielded, &opt);
			s.leave()?;

			s.b.switch_to_block(body_block);
			s.temp(yielded, &opt);
			let v = s.take_payload(yielded, &opt, &item);
			let (Some((var, opt_key)), Typ::Tuple(fields)) = (&after, &item) else {
				return s.bind_pat(pat, v, &item, Some(false));
			};
			let (value, key_val) = (s.ld_typ(v, 0, &fields[0].1), s.ld_typ(v, 8, &fields[1].1));
			s.bind_pat(pat, value, &fields[0].1, Some(false))?;
			if let (Some(key), Some(key_at)) = (key, &key_at) {
				let (at, typ) = s.emit_call_at(key_at, &[it, key_val], Some(span));
				s.bind_pat(key, at, &typ, Some(false))?;
			}
			let last = s.b.use_var(*var);

			// a key owning nothing overwrites the last one in place
			if s.enum_box(opt_key) && !s.slot_owns(&fields[1].1) {
				let some = s.b.ins().iconst(s.int, 1);
				s.store_slots(last, &[some, key_val]);
				return Ok(());
			}

			s.release_value(last, opt_key);
			let last = s.make_option(opt_key, Some(key_val));
			s.untemp(last);
			s.b.def_var(*var, last);
			Ok(())
		}))
	}

	// The block a failed clause leaves a loop through, made on first use.
	fn ended(&mut self) -> Block {
		let frame = self.loops.last_mut().expect("loop frame");
		frame.exit.get_or_insert_with(|| self.b.create_block());
		*frame.fallthrough.get_or_insert_with(|| self.b.create_block())
	}

	// Leave a loop's round through `ended`, releasing what it bound so far.
	fn leave(&mut self) -> Result<(), Diagnostic> {
		let to = self.ended();
		self.release_scopes(self.loops.last().expect("loop frame").depth, None)?;
		self.b.ins().jump(to, &[]);
		Ok(())
	}

	// Set up a generator over its iterable.
	fn for_loop<'e>(
		&mut self,
		(pat, key): Pats<'e>,
		iter: &Spanned<Expr>,
		gens: &mut Vec<Cursor<'e, 'a, M>>,
	) -> Result<(), Diagnostic> {
		let (pat, access) = match &pat.0 {
			Expr::ArgMod(access, name) => (&**name, *access),
			_ => (pat, Access::Read),
		};
		if let Some((Expr::ArgMod(..), span)) = key {
			return fail("the key is not a place", *span, "`mut` goes on the item");
		}
		let (iter, consume) = match &iter.0 {
			Expr::ArgMod(Access::Move, x) => (&**x, true),
			_ => (iter, access == Access::Move),
		};
		let iter = self.header(iter)?;
		let (mut val, typ) = self.expr(iter)?;
		if access == Access::Mut {
			self.mut_place(Self::place_root(iter), "only a mutable binding can be looped `mut`")?;
		}

		// resources are owned. a full-width array or map slot is viewed in place, the rest is written back
		let given = match (access, typ.unkeyed()) {
			(Access::Move, Typ::Array(e)) if self.is_affine(e) => access,
			(Access::Mut, Typ::FixedArray(e, _)) if self.elem_stride(e) == elem_size(e) => access,
			(Access::Mut, Typ::Array(e)) if self.elem_stride(e) == elem_size(e) => {
				self.cow_array(val, e);
				access
			}
			(Access::Mut, Typ::Map(_, v)) if !v.is_unit() => {
				let (slot, _, lent) = self.lend_mut(iter)?;
				let map = self.ld_word(slot, 0);
				val = self.rt_call("map_cow", &[map]).unwrap();
				self.st(slot, 0, val);
				self.reload_lent(&[(slot, lent)])?;
				access
			}
			_ => Access::Read,
		};

		if consume {
			self.move_out(iter, &typ)?;
			self.untemp(val);
			if given != Access::Move {
				self.temp(val, &typ);
			}
		}

		gens.push(self.for_value((pat, key), (val, typ), given, iter.1)?);
		let (Access::Mut, Access::Read, Expr::Ident(name), Some(key)) = (access, given, &pat.0, key) else {
			return Ok(());
		};

		let index = Expr::Index {
			collection: Box::new(iter.clone()),
			index: Box::new(key.clone()),
		};
		let back = crate::ast::assign((index, iter.1), None, (Expr::Ident(name.clone()), pat.1), pat.1);
		let step = move |s: &mut Self| {
			let mut item = s.vars[name].clone();
			item.mutable = true;
			s.vars.insert(name.clone(), item);
			s.defer(&None, &back, When::Always, false);
			Ok(())
		};
		gens.push((Box::new(step), None));

		Ok(())
	}

	fn for_value<'e>(
		&mut self,
		(pat, key): Pats<'e>,
		(val, typ): TypedVal,
		access: Access,
		span: Span,
	) -> Result<Cursor<'e, 'a, M>, Diagnostic> {
		let owns = access == Access::Move;
		let direct = matches!(typ.unkeyed(), Typ::Array(_) | Typ::Map(..) | Typ::FixedArray(..));
		if self.claims(&typ, role::ITERATOR) || !direct && self.claims(&typ, role::KEYED) {
			return Ok((self.iter_loop((pat, key), (val, typ), span)?, None));
		}

		let zero = self.b.ins().iconst(self.int, 0);
		let (enum_key, typ) = (typ.keyed().map(|(k, _)| k), typ.unkeyed().clone());
		let hint = key.is_none() && (enum_key.is_some() || matches!(&typ, Typ::Map(_, v) if !v.is_unit()));
		let (limit, src, keys): (_, TypedVal, Option<TypedVal>) = match typ {
			// arrays
			Typ::Array(_) | Typ::FixedArray(..) | Typ::Str => {
				let (data, len) = self.array_parts(val, &typ);
				(len, (data, array_elem(&typ).clone()), None)
			}

			// sets
			Typ::Map(k, v) if v.is_unit() => {
				let keys = self.map_entries(val, true, &k);
				self.temp(keys, &Typ::Array(k.clone()));
				let (kdata, len) = self.array_parts(keys, &Typ::Array(k.clone()));
				(len, (kdata, (*k).clone()), Some((kdata, *k)))
			}

			// maps
			Typ::Map(k, v) => {
				let (keys, vals) = (self.map_entries(val, true, &k), self.map_entries(val, false, &v));
				let vals = self.owning(vals, &v);
				self.temp(keys, &Typ::Array(k.clone()));
				self.temp(vals, &Typ::Array(v.clone()));
				let (kdata, len) = self.array_parts(keys, &Typ::Array(k.clone()));
				let vdata = self.array_data(vals);
				(len, (vdata, *v), Some((kdata, *k)))
			}

			_ => {
				return fail(format!("cannot iterate over {typ}"), span, "not iterable");
			}
		};

		let counter = self.b.declare_var(self.b.func.dfg.value_type(zero));
		self.b.def_var(counter, zero);

		// release vars skipped by `break`
		let elem = src.1.clone();
		let drop = move |s: &mut Self| {
			let lo = s.b.use_var(counter);
			let n = s.b.ins().isub(limit, lo);
			s.elems_rc(val, lo, n, &elem, false);
			s.rt_call("array_release", &[val]);
			Ok(())
		};

		let step = move |s: &mut Self| {
			let iv = s.b.use_var(counter);
			let more = s.b.ins().icmp(IntCC::SignedLessThan, iv, limit);
			let (body_block, done) = s.fork(more);
			s.b.switch_to_block(done);
			s.leave()?;

			s.b.switch_to_block(body_block);
			let next = s.b.ins().iadd_imm(iv, 1);
			s.b.def_var(counter, next);
			let (data, elem) = &src;
			let n = s.scopes.last().map_or(0, Vec::len);

			// `mut` items view their slot
			if let (Access::Mut, Expr::Ident(name)) = (access, &pat.0) {
				let slot = match keys {
					Some(_) => s.rt_call("map_slot", &[val, iv]).unwrap(),
					None => s.nth_addr(*data, iv, elem),
				};
				let var = s.b.declare_var(s.int);
				s.b.def_var(var, slot);
				let mut view = Local::plain(var, elem.clone(), true);
				view.boxed = true;
				s.vars.insert(name.clone(), view);
			} else {
				let item = s.load_nth(*data, iv, elem);
				let item = if owns { s.unbox_ret(item, elem) } else { item };
				s.bind_pat(pat, item, elem, Some(false)).map_err(|d| match hint {
					true => d.with_note("the key is a second name, as in `loop v, k in m`"),
					false => d,
				})?;
			}

			if let Some(key) = key {
				let (key_val, key_typ) = match (&keys, &enum_key) {
					(Some((kdata, kt)), _) => (s.load_nth(*kdata, iv, kt), kt.clone()),
					(None, Some(variant)) => (s.enum_disc(variant, iv), variant.clone()),
					(None, None) => (iv, Typ::Int(64)),
				};
				s.bind_pat(key, key_val, &key_typ, Some(false))?;
			}
			if !owns {
				for (var, t) in s.scopes.last_mut().expect("scope").split_off(n) {
					if !s.is_affine(&t) {
						s.own_local(var, &t);
					}
				}
			}
			Ok(())
		};
		Ok((Box::new(step), owns.then(|| Box::new(drop) as Emit<'e, 'a, M>)))
	}

	// Bind or assign a pattern's names against a value.
	pub(super) fn bind_pat(
		&mut self,
		pat: &Spanned<Expr>,
		val: Value,
		typ: &Typ,
		mutable: Option<bool>,
	) -> Result<(), Diagnostic> {
		// two names on a propagator split the payload from its ok flag or error
		if let Expr::Tuple(names) = &pat.0
			&& let [(_, value_pat), (_, flag_pat)] = names.as_slice()
			&& let Some((inner, err)) = self.fallible_split(typ).filter(|_| self.trailing(typ).is_none())
		{
			let tag = self.enum_tag(typ, val);
			let payload = self.payload_or_zero(val, typ, pat.1)?;
			let (flag, flag_typ) = match err {
				None => (tag, Typ::Bool),
				Some(err) => {
					let flag_typ = self.types.core_enum(role::OPTION, std::slice::from_ref(&err));
					if self.types.opt_niche(&flag_typ).is_some() {
						let (e, none) = (self.ld_typ(val, 8, &err), self.make_option(&flag_typ, None));
						(self.b.ins().select(tag, e, none), flag_typ)
					} else {
						let flag = self.call_alloc(2);
						self.st(flag, 0, tag);
						self.when(tag, |s| {
							let e = s.ld_typ(val, 8, &err);
							let e = s.copy_in(e, &err);
							s.st(flag, 8, e)
						});
						self.temp(flag, &flag_typ);
						(flag, flag_typ)
					}
				}
			};
			self.bind_pat(value_pat, payload, &inner, mutable)?;
			return self.bind_pat(flag_pat, flag, &flag_typ, mutable);
		}
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
				let pname = if name.is_empty() {
					sname.into()
				} else {
					self.qualify(name)
				};
				struct_pattern(fdefs, &pname, sname, fields, pat.1)
			}

			(Expr::Array(elems), Typ::Array(elem) | Typ::FixedArray(elem, _)) => {
				field_binds(elems.iter().map(|e| (e, &**elem)), 0, 1)
			}
			_ => fail(
				format!("cannot destructure {typ} with this pattern"),
				pat.1,
				"wrong shape",
			)
			.map_err(|d| match pat.0 {
				Expr::Tuple(_) => d.with_note("to bind every name to one value, chain `a := b := ...`"),
				_ => d,
			}),
		}
	}

	// Load a name from `pat_binds` out of the subject.
	pub(super) fn load_bind(&mut self, subject: Value, typ: &Typ, field: &Typ, off: i32, span: Span) -> Value {
		match typ {
			Typ::Array(_) | Typ::FixedArray(..) => {
				let idx = self.b.ins().iconst(self.int, off as i64);
				self.load_index((subject, typ), idx, span)
			}
			_ => self.opt_payload(subject, typ, field, off),
		}
	}
}

type ArmBinds = (TypedVal, Vec<Bind>);
type Unwrapping = (String, Local, Typ);

// Field chain key.
pub(super) fn place_key(e: &Expr) -> Option<String> {
	match e {
		Expr::Ident(name) => Some(name.clone()),
		Expr::Field { tuple, field } => Some(format!("{}.{field}", place_key(&tuple.0)?)),
		_ => None,
	}
}

// Swaps a pattern's sub-patterns for hidden names, returning them in order.
fn split_subs(pat: &Spanned<Expr>) -> (Spanned<Expr>, Vec<Spanned<Expr>>) {
	let (mut pat, mut subs) = (pat.clone(), vec![]);
	let slots = match &mut pat.0 {
		Expr::Tuple(elems) => elems.iter_mut().map(|(_, e)| e).collect(),
		Expr::Array(elems) => elems.iter_mut().collect(),
		Expr::StructLit { fields, .. } => {
			fields.iter_mut().filter(|(name, _)| name.is_some()).map(|(_, e)| e).collect()
		}
		Expr::EnumShorthand { args, .. } => match &mut args[..] {
			[(Expr::Record(entries), _)] => entries.iter_mut().map(|(_, e)| e).collect(),
			args => args.iter_mut().collect(),
		},
		_ => vec![],
	};
	for slot in slots.into_iter().filter(|e| !matches!(e.0, Expr::Ident(_))) {
		let hidden = (Expr::Ident(format!("$sub{}", subs.len())), slot.1);
		subs.push(std::mem::replace(slot, hidden));
	}
	(pat, subs)
}

// Whether any sub-pattern can fail to match.
fn refutable_subs(pat: &Spanned<Expr>) -> bool {
	let destructures = |sub: &Spanned<Expr>| matches!(sub.0, Expr::Tuple(_) | Expr::StructLit { .. });
	split_subs(pat).1.iter().any(|sub| !destructures(sub) || refutable_subs(sub))
}

fn falls(arm: &MatchArm) -> bool {
	matches!(arm.body.last(), Some((Expr::Fallthrough, _)))
}
