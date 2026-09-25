use crate::compiler::{comp, expand};

use super::*;

impl<'a, M: Module> Translator<'a, M> {
	// Lower a quote. Register its template and build the Ast it produces at runtime.
	pub(super) fn quote(&mut self, stmts: &[Spanned<Expr>], span: Span) -> Result<TypedVal, Diagnostic> {
		let (tpl, slots) = expand::register(stmts, span)?;
		let mut ptrs = Vec::with_capacity(slots.len());
		for slot in &slots {
			let ptr = match slot {
				expand::Slot::Name(name) => {
					let Some(local) = self.vars.get(name).cloned() else {
						return Err(Diagnostic::new(
							format!("`%{name}` refers to no binding in scope"),
							span.into_range(),
						)
						.with_label("not found in scope"));
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
						return Err(Diagnostic::new(
							format!("can't spread a `{typ}`, expected `[]Ast`"),
							e.1.into_range(),
						)
						.with_label("not []Ast"));
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
			for (i, v) in ptrs.iter().enumerate() {
				self.b.ins().store(MemFlags::new(), *v, slot, (i * 8) as i32);
			}
			slot
		};
		let len = self.b.ins().iconst(self.int, ptrs.len() as i64);
		let idxv = self.b.ins().iconst(self.int, tpl as i64);
		let func = self.import_fn(expand::RT_QUOTE, &[self.int; 3], Some(self.int));
		let call = self.b.ins().call(func, &[idxv, slot, len]);
		Ok((self.b.inst_results(call)[0], Typ::Ast))
	}

	// Call rt_ast_method on an Ast.
	pub(super) fn ast_method(&mut self, ast: Value, m: &str, arg: Option<Value>) -> Value {
		let m = self.str_const(m);
		let arg = arg.unwrap_or_else(|| self.b.ins().iconst(self.int, 0));
		let func = self.import_fn(expand::RT_AST_METHOD, &[self.int; 3], Some(self.int));
		let call = self.b.ins().call(func, &[ast, m, arg]);
		self.b.inst_results(call)[0]
	}

	// Lift an unquoted value into a Ast literal pointer, ready to splice into a template.
	fn lift_unquote(&mut self, val: Value, typ: &Typ, span: Span) -> Result<Value, Diagnostic> {
		let (tag, bits) = match typ {
			Typ::Ast => return Ok(val),
			Typ::Int(_) | Typ::ISize => (comp::TAG_INT, self.intcast(val, types::I64, true)),
			Typ::UInt(_) | Typ::USize => (comp::TAG_INT, self.intcast(val, types::I64, false)),
			Typ::Bool => (comp::TAG_BOOL, val),
			Typ::Str => (comp::TAG_STR, val),
			Typ::Float(32) => {
				let f64v = self.b.ins().fpromote(types::F64, val);
				(comp::TAG_FLOAT, self.b.ins().bitcast(self.int, MemFlags::new(), f64v))
			}
			Typ::Float(_) => (comp::TAG_FLOAT, self.b.ins().bitcast(self.int, MemFlags::new(), val)),
			other => {
				return Err(
					Diagnostic::new(format!("can't unquote a `{other}` yet"), span.into_range())
						.with_label("unsupported unquote type"),
				);
			}
		};
		let tagv = self.b.ins().iconst(self.int, tag);
		let func = self.import_fn(expand::RT_AST_LIT, &[self.int; 2], Some(self.int));
		let call = self.b.ins().call(func, &[tagv, bits]);
		Ok(self.b.inst_results(call)[0])
	}

	// Call the runtime panic path with `msg` and mark the current block unreachable.
	fn abort(&mut self, msg: Value) -> TypedVal {
		self.rt_call("panic", &[msg]);
		self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);

		// unreachable paths
		let dead = self.b.create_block();
		self.b.seal_block(dead);
		self.b.switch_to_block(dead);
		self.unit_value()
	}

	// The optional message argument for the aborting macros.
	fn msg_arg(&mut self, name: &str, arg: Option<&Spanned<Expr>>, default: &str) -> Result<Value, Diagnostic> {
		let Some(arg) = arg else {
			return Ok(self.str_const(default));
		};
		match self.expr(arg)? {
			(val, Typ::Str) => Ok(val),
			(_, typ) => Err(
				Diagnostic::new(format!("`{name}!` message must be Str, got {typ}"), arg.1.into_range())
					.with_label("not a Str"),
			),
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
				return Err(
					Diagnostic::new(format!("no macro named `{name}!`"), span.into_range()).with_label("unknown macro")
				);
			}
		};
		if !(min..=max).contains(&args.len()) {
			let want = match (min, max) {
				(1, 1) => "1 argument".into(),
				(a, b) if a == b => format!("{a} arguments"),
				(a, b) => format!("{a} or {b} arguments"),
			};
			return Err(
				Diagnostic::new(format!("`{name}!` takes {want}, got {}", args.len()), span.into_range())
					.with_label("wrong number of arguments"),
			);
		}

		match name {
			"dbg" => {
				let (val, typ) = self.expr(&args[0])?;
				let (file, line, snippet) = self.map.locate_span(args[0].1.into_range());
				self.write_lit(&format!("[{file}:{line}] {snippet} = "), runtime::Sink::Err);
				self.emit_print(val, &typ, false, runtime::Sink::Err);
				self.write_lit("\n", runtime::Sink::Err);
				Ok((val, typ))
			}

			"src" => self.src_lit(span),

			"assert" => {
				let (cond, typ) = self.expr(&args[0])?;
				if typ != Typ::Bool {
					return Err(Diagnostic::new(
						format!("`assert!` condition must be Bool, got {typ}"),
						args[0].1.into_range(),
					)
					.with_label("not a Bool"));
				}
				// the failure message defaults to the condition's source
				let snippet = self.map.locate_span(args[0].1.into_range()).2;
				let msg = self.msg_arg(name, args.get(1), snippet)?;

				let fail_block = self.b.create_block();
				let ok_block = self.b.create_block();
				self.b.ins().brif(cond, ok_block, &[], fail_block, &[]);
				self.b.seal_block(fail_block);
				self.b.seal_block(ok_block);

				self.b.switch_to_block(fail_block);
				self.rt_call("assert_fail", &[msg]);
				self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);

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
				Ok(self.abort(msg))
			}
		}
	}
}
