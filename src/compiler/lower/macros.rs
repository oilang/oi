use crate::compiler::{comp, expand};

use super::*;

impl<'a, M: Module> Translator<'a, M> {
	// Lower a quote. Register its template and build the Ast it produces at runtime.
	pub(super) fn quote(&mut self, stmts: &[Spanned<Expr>], span: Span) -> Result<TypedVal, Diagnostic> {
		if !self.comptime {
			return fail("quotes only exist at comptime", span, "runtime quote");
		}
		let (tpl, slots) = expand::register(stmts, span)?;
		let mut ptrs = Vec::with_capacity(slots.len());
		for slot in &slots {
			let ptr = match slot {
				expand::Slot::Name(name) => {
					let Some(local) = self.vars.get(name).cloned() else {
						return fail(
							format!("`%{name}` refers to no binding in scope"),
							span,
							"not found in scope",
						);
					};
					let val = self.read_local(&local);
					self.lift_unquote(val, &local.typ, span)?
				}
				expand::Slot::Expr(e) => {
					let (val, typ) = self.expr(e)?;
					self.lift_unquote(val, &typ, span)?
				}
				expand::Slot::Splat(e) => {
					let (val, typ) = self.expr(e)?;
					if !matches!(&typ, Typ::Array(inner) if **inner == Typ::Ast) {
						return fail(format!("can't spread a `{typ}`, expected `[]Ast`"), e.1, "not []Ast");
					}
					// the header pointer itself: rt_quote reads the elements
					val
				}
			};
			ptrs.push(ptr);
		}
		let slot = if ptrs.is_empty() {
			self.b.ins().iconst(self.int, 0)
		} else {
			let slot = self.stack_slot((ptrs.len() * 8) as u32);
			self.store_slots(slot, &ptrs);
			slot
		};
		let len = self.b.ins().iconst(self.int, ptrs.len() as i64);
		let idxv = self.b.ins().iconst(self.int, tpl as i64);
		let ast = self.extern_call(expand::RT_QUOTE, &[idxv, slot, len], Some(self.int));
		Ok((ast.unwrap(), Typ::Ast))
	}

	// Call rt_ast_method on an Ast.
	pub(super) fn ast_method(&mut self, ast: Value, m: &str, arg: Option<Value>) -> Value {
		let m = self.str_const(m);
		let arg = arg.unwrap_or_else(|| self.b.ins().iconst(self.int, 0));
		self.extern_call(expand::RT_AST_METHOD, &[ast, m, arg], Some(self.int)).unwrap()
	}

	// Lift an unquoted value into a Ast literal pointer, ready to splice into a template.
	fn lift_unquote(&mut self, val: Value, typ: &Typ, span: Span) -> Result<Value, Diagnostic> {
		let (tag, bits) = match typ {
			Typ::Ast => return Ok(val),
			Typ::Int(_) | Typ::ISize | Typ::UInt(_) | Typ::USize => (comp::TAG_INT, self.scalar_bits(val, typ)),
			Typ::Bool => (comp::TAG_BOOL, val),
			Typ::Str => (comp::TAG_STR, val),
			Typ::Float(_) => (comp::TAG_FLOAT, self.scalar_bits(val, typ)),
			other => {
				return fail(
					format!("can't unquote a `{other}` yet"),
					span,
					"unsupported unquote type",
				);
			}
		};
		let tagv = self.b.ins().iconst(self.int, tag);
		Ok(self.extern_call(expand::RT_AST_LIT, &[tagv, bits], Some(self.int)).unwrap())
	}

	// Abort via the specified `rt`, passing the context it routes `ctx.panic` from, then trap.
	pub(super) fn ctx_panic(&mut self, rt: &str, msg: Value, span: Span) -> Result<(), Diagnostic> {
		if self.comptime {
			let [assert, at, end] = [(rt == "assert_fail") as usize, span.start, span.end];
			let [assert, at, end] = [assert, at, end].map(|n| self.b.ins().iconst(self.int, n as i64));
			self.extern_call(comp::RT_COMP_PANIC, &[assert, msg, at, end], None);
		} else {
			let (at, _) = self.src_lit(span)?;
			let ctx = self.ctx_value(crate::compiler::CONTEXT);
			self.rt_call(rt, &[ctx, msg, at]);
		}
		self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);
		Ok(())
	}

	// The optional message argument for the aborting macros.
	fn msg_arg(&mut self, name: &str, arg: Option<&Spanned<Expr>>, default: &str) -> Result<Value, Diagnostic> {
		let Some(arg) = arg else {
			return Ok(self.str_const(default));
		};
		match self.expr(arg)? {
			(val, Typ::Str) => Ok(val),
			(_, typ) => fail(format!("`{name}!` message must be Str, got {typ}"), arg.1, "not a Str"),
		}
	}

	pub(super) fn macro_call(
		&mut self,
		name: &str,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let (min, max) = match name {
			"dbg" => (1, 1),
			"assert" => (1, 2),
			"panic" | "unreachable" => (0, 1),
			"todo" | "src" => (0, 0),
			_ => {
				return fail(format!("no macro named `{name}!`"), span, "unknown macro");
			}
		};
		if !(min..=max).contains(&args.len()) {
			let want = match (min, max) {
				(1, 1) => "1 argument".into(),
				(a, b) if a == b => format!("{a} arguments"),
				(a, b) => format!("{a} or {b} arguments"),
			};
			return fail(
				format!("`{name}!` takes {want}, got {}", args.len()),
				span,
				"wrong number of arguments",
			);
		}

		match name {
			"dbg" => {
				let (val, typ) = self.expr(&args[0])?;
				let (file, line, col, _) = self.world.map.locate_span(span.into_range());
				let snippet = self.world.map.locate_span(args[0].1.into_range()).3;
				self.write_lit(&format!("[{file}:{line}:{col}] {snippet} = "), runtime::Sink::Err);
				let recv = format!("$dbg{}", self.vars.len());
				self.hidden_local(recv.clone(), val, typ.clone());
				let repr = Expr::MethodCall {
					recv: Box::new((Expr::Ident(recv), span)),
					method: "repr".into(),
					type_args: vec![],
					args: vec![],
				};
				let (s, _) = self.expr(&(repr, span))?;
				self.emit_print(s, &Typ::Str, false, runtime::Sink::Err);
				self.write_lit("\n", runtime::Sink::Err);
				Ok((val, typ))
			}

			"src" => self.src_lit(span),

			"assert" => {
				let cond = self.bool_value(&args[0], "`assert!` condition")?;
				// the failure message defaults to the condition's source
				let snippet = self.world.map.locate_span(args[0].1.into_range()).3;
				let msg = self.msg_arg(name, args.get(1), snippet)?;

				let (ok_block, fail_block) = self.fork(cond);

				self.b.switch_to_block(fail_block);
				self.ctx_panic("assert_fail", msg, span)?;

				self.b.switch_to_block(ok_block);
				Ok(self.unit_value())
			}

			_ => {
				let default = match name {
					"panic" => "panicked",
					"todo" => "not yet implemented",
					_ => "entered unreachable code",
				};
				let msg = self.msg_arg(name, args.first(), default)?;
				self.ctx_panic("panic", msg, span)?;
				// code after the panic lands in a dead block
				let dead = self.b.create_block();
				self.b.seal_block(dead);
				self.b.switch_to_block(dead);
				Ok(self.unit_value())
			}
		}
	}
}
