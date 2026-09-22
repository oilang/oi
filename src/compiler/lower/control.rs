use crate::compiler::expand;

use super::*;

impl<'a, M: Module> Translator<'a, M> {
	// Lower branching control flow constructs.
	pub(super) fn branching(
		&mut self,
		expr: &Spanned<Expr>,
		hint: Option<&Typ>,
	) -> Result<Option<TypedVal>, Diagnostic> {
		match &expr.0 {
			Expr::If { cond, then, els } => self.conditional(cond, then, els.as_deref(), hint, expr.1),
			Expr::Match {
				subject,
				arms,
				else_body,
			} => self.match_expr(subject, arms, else_body.as_deref(), hint, expr.1),
			Expr::Loop { cond, body } => self.loop_expr(cond.as_deref(), body),
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
	) -> Result<Option<TypedVal>, Diagnostic> {
		let (cv, ct) = self.expr(cond)?;
		if ct != Typ::Bool {
			return Err(
				Diagnostic::new(format!("`if` condition must be Bool, got {ct}"), cond.1.into_range())
					.with_label("not a Bool"),
			);
		}

		let then_block = self.b.create_block();
		let else_block = self.b.create_block();
		self.b.ins().brif(cv, then_block, &[], else_block, &[]);
		self.b.seal_block(then_block);
		self.b.seal_block(else_block);

		let merge = self.b.create_block();
		let mut result: Option<(Variable, Typ)> = None;

		self.b.switch_to_block(then_block);
		let then_flow = self.scoped(|s| s.block_tail(then, target))?;
		if let Some(vt) = then_flow {
			self.contribute("if", vt, &mut result, merge, span)?;
		}

		self.b.switch_to_block(else_block);
		let else_flow = self.else_default(els, &result, target, span)?;
		if let Some(vt) = else_flow {
			self.contribute("if", vt, &mut result, merge, span)?;
		}

		Ok(self.finish_merge(merge, result))
	}

	// The else arm.
	fn else_default(
		&mut self,
		els: Option<&[Spanned<Expr>]>,
		result: &Option<(Variable, Typ)>,
		target: Option<&Typ>,
		span: Span,
	) -> Result<Option<TypedVal>, Diagnostic> {
		let Some(els) = els else {
			let t = result
				.as_ref()
				.map(|(_, t)| t.clone())
				.or_else(|| target.cloned())
				.unwrap_or(Typ::unit());
			let ok = self.types.fallible(&t);
			return self.scoped(|s| {
				let v = if ok { s.zero(&t) } else { s.zero_or_err(&t, span)? };
				Ok(Some((v, t.clone())))
			});
		};
		self.scoped(|s| s.block_tail(els, target))
	}

	// Evaluate `f` in a child scope.
	pub(super) fn scoped(
		&mut self,
		f: impl FnOnce(&mut Self) -> Result<Option<TypedVal>, Diagnostic>,
	) -> Result<Option<TypedVal>, Diagnostic> {
		let saved = self.vars.clone();
		self.scopes.push(vec![]);
		self.defers.push(vec![]);
		let flow = f(self);
		self.vars = saved;
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
			None => Err(Diagnostic::new("this block never produces a value", span.into_range())
				.with_label("every path returns, but a value is needed here")),
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

	// `match`
	// first arm wins.
	pub(super) fn match_expr(
		&mut self,
		subject: &Spanned<Expr>,
		arms: &[MatchArm],
		else_body: Option<&[Spanned<Expr>]>,
		target: Option<&Typ>,
		span: Span,
	) -> Result<Option<TypedVal>, Diagnostic> {
		let (sv, st) = self.expr(subject)?;
		let sv_var = self.b.declare_var(cl_type(&st, self.int));
		self.b.def_var(sv_var, sv);

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
					let msg = format!("non-exhaustive match, missing: {}", missing.join(", "));
					return Err(
						Diagnostic::new(msg, span.into_range()).with_label("cover these variants or add `else`")
					);
				}
			}
		}

		let merge = self.b.create_block();
		let mut result: Option<(Variable, Typ)> = None;

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
					let disc = self.b.ins().iconst(self.int, disc);
					self.b.ins().icmp(IntCC::Equal, tag, disc)
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
					let fv = s.load_bind(sv, &st, typ, *off);
					let fv = s.copy_bind(fv, typ);
					s.bind_local(name, fv, typ.clone(), false);
				}
				if let Some((outs, names)) = &quote_binds {
					for (i, name) in names.iter().enumerate() {
						let ptr = s.b.ins().load(s.int, MemFlags::new(), *outs, (i * 8) as i32);
						s.bind_local(name, ptr, Typ::Ast, false);
					}
				}
				s.block_tail(&arm.body, target)
			})?;
			if let Some(vt) = flow {
				self.contribute("match", vt, &mut result, merge, span)?;
			}
		}

		self.b.switch_to_block(else_blk);
		self.b.seal_block(else_blk);
		let else_flow = self.else_default(else_body, &result, target, span)?;
		if let Some(vt) = else_flow {
			self.contribute("match", vt, &mut result, merge, span)?;
		}

		Ok(self.finish_merge(merge, result))
	}

	// Write (v, t) into the shared result variable and jump to `merge`.
	// All branches must agree on type. The first one declares the variable.
	pub(super) fn contribute(
		&mut self,
		kw: &str,
		(v, t): TypedVal,
		result: &mut Option<(Variable, Typ)>,
		merge: Block,
		span: Span,
	) -> Result<(), Diagnostic> {
		match result {
			Some((_, rt)) if rt != &t => Err(Diagnostic::new(
				format!("`{kw}` branches have mismatched types: {rt} and {t}"),
				span.into_range(),
			)
			.with_label("must yield the same type")),
			Some((var, _)) => {
				self.b.def_var(*var, v);
				self.b.ins().jump(merge, &[]);
				Ok(())
			}
			None => {
				let var = self.b.declare_var(cl_type(&t, self.int));
				self.b.def_var(var, v);
				self.b.ins().jump(merge, &[]);
				*result = Some((var, t));
				Ok(())
			}
		}
	}

	// `or` blocks, for unwrapping Options and Results.
	// The happy branch yields the inner value, the sad branch executes a block and yields its value.
	pub(super) fn or_else(
		&mut self,
		value: &Spanned<Expr>,
		body: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let (val, typ) = self.expr(value)?;
		let (inner, happy, err) = match (self.types.result_parts(&typ), self.types.option_inner(&typ)) {
			(Some((ok, err)), _) => (ok, 0, Some(err)),
			(None, Some(inner)) => (inner, 1, None),
			_ => {
				return Err(
					Diagnostic::new(format!("`or` needs a `?T`/`!T` value, got {typ}"), value.1.into_range())
						.with_label("not an Option or Result"),
				);
			}
		};

		let tag = self.enum_tag(&typ, val);
		let happy_disc = self.b.ins().iconst(self.int, happy);
		let is_happy = self.b.ins().icmp(IntCC::Equal, tag, happy_disc);

		let happy_block = self.b.create_block();
		let fallback_block = self.b.create_block();
		self.b.ins().brif(is_happy, happy_block, &[], fallback_block, &[]);
		self.b.seal_block(happy_block);
		self.b.seal_block(fallback_block);
		let merge = self.b.create_block();
		let mut result = None;

		self.b.switch_to_block(happy_block);
		let payload = self.opt_payload(val, &typ, &inner, 8);
		let payload = self.copy_bind(payload, &inner);
		self.contribute("or", (payload, inner.clone()), &mut result, merge, span)?;

		self.b.switch_to_block(fallback_block);
		let saved_dollar = self.dollar.take();
		self.dollar = Some(match err {
			Some(err) => (self.b.ins().load(cl_type(&err, self.int), MemFlags::new(), val, 8), err),
			None => self.unit_value(),
		});
		let flow = self.scoped(|s| s.block_tail(body, Some(&inner)))?;
		self.dollar = saved_dollar;
		if let Some(vt) = flow {
			self.contribute("or", vt, &mut result, merge, span)?;
		}

		Ok(self.finish_merge(merge, result).expect("`or` always yields"))
	}

	// Unwraps `?T`/`!T`.
	// Returns `none`/error from the enclosing fn on the sad path.
	// Panics when called in `main`.
	pub(super) fn propagate(&mut self, value: &Spanned<Expr>, span: Span) -> Result<TypedVal, Diagnostic> {
		let (val, typ) = self.expr(value)?;
		let (is_result, inner, err_typ) = match (self.types.result_parts(&typ), self.types.option_inner(&typ)) {
			(Some((ok, err)), _) => (true, ok, err),
			(None, Some(inner)) => (false, inner, Typ::Error),
			_ => {
				let msg = format!("`?` needs a `?T` or `!T` value, got {typ}");
				return Err(Diagnostic::new(msg, value.1.into_range()).with_label("not a `?T` or `!T` value"));
			}
		};
		let shape = if is_result { "!T" } else { "?T" };
		let panic_in_main = self.ret.is_none() && self.is_main;
		let mut target_err = err_typ.clone();
		let declared = self.ret.as_ref().map(|(t, _)| t.clone());
		let target = match &declared {
			Some(d) if is_result && let Some((t, e)) = self.types.result_parts(d) => {
				if e != err_typ && !(e == Typ::Error && self.open_error(&err_typ)) {
					let msg = format!("cannot propagate `{err_typ}` into a fn returning {d}");
					let label = match e == Typ::Error {
						true => format!("`{err_typ}` does not claim Error"),
						false => "mismatched error type".to_string(),
					};
					return Err(Diagnostic::new(msg, span.into_range()).with_label(label));
				}
				target_err = e;
				t
			}
			Some(d) if !is_result && let Some(t) = self.types.option_inner(d) => t,
			Some(other) => {
				let msg = format!("`?` needs an enclosing fn returning `{shape}`, found {other}");
				return Err(Diagnostic::new(msg, span.into_range()).with_label(format!("not a `{shape}` fn")));
			}
			None => inner.clone(),
		};
		let target_typ = match is_result {
			true => self.types.core_enum(role::RESULT, &[target.clone(), target_err.clone()]),
			false => self.types.core_enum(role::OPTION, std::slice::from_ref(&target)),
		};

		let tag = self.enum_tag(&typ, val);
		let happy: i64 = if is_result { 0 } else { 1 };
		let happy_disc = self.b.ins().iconst(self.int, happy);
		let is_happy = self.b.ins().icmp(IntCC::Equal, tag, happy_disc);

		let happy_block = self.b.create_block();
		let sad_block = self.b.create_block();
		self.b.ins().brif(is_happy, happy_block, &[], sad_block, &[]);
		self.b.seal_block(happy_block);
		self.b.seal_block(sad_block);

		self.b.switch_to_block(sad_block);
		if panic_in_main {
			let msg = if is_result {
				let e = self.b.ins().load(self.int, MemFlags::new(), val, 8);
				self.derived_str(e, &err_typ)
			} else {
				self.str_const("unwrapped `none`")
			};
			self.rt_call("panic", &[msg]);
			self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);
		} else {
			let sad_val = if is_result {
				let e = self.b.ins().load(self.int, MemFlags::new(), val, 8);
				let e = if err_typ == target_err {
					e
				} else {
					self.box_error(e, &err_typ)
				};
				let variants = self.variants_of(&target_typ);
				self.make_enum(&variants, 1, &[e])
			} else {
				self.make_option(&target_typ, None)
			};
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
		let (sad, done) = (self.b.create_block(), self.b.create_block());
		self.b.ins().brif(tag, sad, &[], done, &[]);
		self.b.seal_block(sad);
		self.b.seal_block(done);
		self.b.switch_to_block(sad);
		let e = self.b.ins().load(self.int, MemFlags::new(), val, 8);
		let msg = self.derived_str(e, &err);
		self.rt_call("fail", &[msg]);
		self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);
		self.b.switch_to_block(done);
	}

	// `Enum.from(v)`.
	pub(super) fn enum_from(&mut self, name: &str, args: &[Spanned<Expr>], span: Span) -> Result<TypedVal, Diagnostic> {
		if args.len() != 1 {
			let msg = format!("`{name}.from` takes 1 argument, got {}", args.len());
			return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of arguments"));
		}
		let (av, at) = self.expr(&args[0])?;
		if !matches!(
			at,
			Typ::Str | Typ::Atom | Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize
		) {
			let msg = format!("`{name}.from` needs an int, string, or atom. Got {at}");
			return Err(Diagnostic::new(msg, args[0].1.into_range()).with_label("not an int, string, or atom"));
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
			Some(cond) => {
				let (cv, ct) = self.expr(cond)?;
				if ct != Typ::Bool {
					return Err(Diagnostic::new(
						format!("`loop` condition must be Bool, got {ct}"),
						cond.1.into_range(),
					)
					.with_label("not a Bool"));
				}
				let body_block = self.b.create_block();
				let (exit, fallthrough) = (self.b.create_block(), self.b.create_block());
				self.b.ins().brif(cv, body_block, &[], fallthrough, &[]);
				self.b.seal_block(body_block);
				self.b.seal_block(fallthrough);
				self.b.switch_to_block(body_block);
				(Some(exit), Some(fallthrough))
			}
			None => (None, None),
		};

		let (frame, flow) = self.in_loop(top, exit, fallthrough, |s| s.block(body))?;

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
			return Err(
				Diagnostic::new(format!("cannot iterate over {it_typ}"), span.into_range())
					.with_label("its `Iterator` claim has no `next(mut self) ?T`"),
			);
		};
		let opt = next.ret.clone();

		let (header, body_block, fallthrough, exit) = (
			self.b.create_block(),
			self.b.create_block(),
			self.b.create_block(),
			self.b.create_block(),
		);
		self.b.ins().jump(header, &[]);

		self.b.switch_to_block(header);
		let (yielded, _) = self.emit_call(&next, &[it]);
		let tag = self.enum_tag(&opt, yielded);
		self.b.ins().brif(tag, body_block, &[], fallthrough, &[]);
		self.b.seal_block(body_block);
		self.b.seal_block(fallthrough);

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
		let (val, typ) = self.expr(iter)?;
		if self.claims(&typ, role::ITERABLE) || self.claims(&typ, role::ITERATOR) {
			return self.iter_loop(pat, body, (val, typ), iter.1);
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
				return Err(
					Diagnostic::new(format!("cannot iterate over {typ}"), iter.1.into_range())
						.with_label("not iterable"),
				);
			}
		};
		let counter = self.b.declare_var(self.b.func.dfg.value_type(zero));
		self.b.def_var(counter, zero);

		let (header, body_block, latch, fallthrough, exit) = (
			self.b.create_block(),
			self.b.create_block(),
			self.b.create_block(),
			self.b.create_block(),
			self.b.create_block(),
		);
		self.b.ins().jump(header, &[]);

		self.b.switch_to_block(header);
		let iv = self.b.use_var(counter);
		let more = self.b.ins().icmp(IntCC::SignedLessThan, iv, limit);
		self.b.ins().brif(more, body_block, &[], fallthrough, &[]);
		self.b.seal_block(body_block);
		self.b.seal_block(fallthrough);

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
					return Err(
						Diagnostic::new("destructure map entries with `(k, v)`", pat.1.into_range())
							.with_label("expected `(k, v)`"),
					);
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
				self.load_bind(val, typ, &ftyp, off)
			} else {
				val
			};
			let v = self.copy_bind(v, &ftyp);
			let Some(mutable) = mutable else {
				let local = self.mutable_local(&name, pat.1.into_range(), Mutation::Assign)?;
				if local.typ != ftyp {
					let msg = format!("cannot assign {ftyp} to `{name}`, which is {}", local.typ);
					return Err(Diagnostic::new(msg, pat.1.into_range()).with_label("type mismatch"));
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
					return Err(Diagnostic::new(msg, pat.1.into_range()).with_label("wrong number of fields"));
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
			_ => Err(Diagnostic::new(
				format!("cannot destructure {typ} with this pattern"),
				pat.1.into_range(),
			)
			.with_label("wrong shape")),
		}
	}

	// Load a name from `pat_binds` out of the subject.
	pub(super) fn load_bind(&mut self, subject: Value, typ: &Typ, field: &Typ, off: i32) -> Value {
		match typ {
			Typ::Array(_) | Typ::FixedArray(..) => {
				let (data, len) = self.array_parts(subject, typ);
				let idx = self.b.ins().iconst(self.int, off as i64);
				self.load_index(data, len, field, idx)
			}
			_ => self.opt_payload(subject, typ, field, off),
		}
	}
}
