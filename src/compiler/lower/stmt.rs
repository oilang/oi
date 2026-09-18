use super::*;

impl<'a, M: Module> Translator<'a, M> {
	// Evaluate a block of statements, returning the final value.
	// Returns None if the block diverged (every path returned/broke).
	pub fn block(&mut self, stmts: &[Spanned<Expr>]) -> Result<Option<TypedVal>, Diagnostic> {
		self.block_tail(stmts, None)
	}

	// Coerces a bare trailing expression against `tail`.
	pub fn block_tail(&mut self, stmts: &[Spanned<Expr>], tail: Option<&Typ>) -> Result<Option<TypedVal>, Diagnostic> {
		let mut last = self.unit_value();
		for (i, stmt) in stmts.iter().enumerate() {
			let stmt_target = if i + 1 == stmts.len() { tail } else { None };
			let zeroed;
			let stmt = match &stmt.0 {
				Expr::Claim { typ, traits, .. } => {
					let annot = Some((TypeExpr::Name(traits[0].0.clone()), stmt.1));
					zeroed = (
						Expr::Bind {
							mutable: true,
							name: typ.clone(),
							typ: annot,
							value: None,
						},
						stmt.1,
					);
					&zeroed
				}
				_ => stmt,
			};
			match &stmt.0 {
				Expr::Bind {
					mutable,
					name,
					typ,
					value,
				} => {
					// a bind whose name resolves as a type is an alias
					if !*mutable
						&& typ.is_none() && value.as_ref().is_some_and(|v| TypeExpr::from_expr(&v.0).is_some())
						&& self.types().resolve(&TypeExpr::Name(name.clone()), stmt.1).is_ok()
					{
						continue;
					}
					let annot = typ.as_ref().map(|(t, span)| self.types().resolve(t, *span)).transpose()?;
					if !*mutable && matches!(value.as_deref(), Some((Expr::AnonFn { .. }, _))) {
						self.self_name = Some(name.clone());
					}
					let (val, typ) = match (value, annot) {
						(Some(value), Some(target)) => {
							let val = self.check_typed(value, &target, "does not match the declared type")?;
							(val, target)
						}
						(Some(value), None) => self.expr(value)?,
						(None, Some(target)) => {
							if matches!(target, Typ::Ref(_)) {
								let msg = "a reference must be initialized (`?&T` for an optional one)";
								return Err(
									Diagnostic::new(msg, stmt.1.into_range()).with_label("no zero value for `&T`")
								);
							}
							(self.zero(&target), target)
						}
						(None, None) => unreachable!("binding has neither a type nor a value"),
					};
					self.self_name = None;
					if let Some(v) = value {
						self.move_resource(v, &typ)?;
					}
					let final_val = match &typ {
						Typ::FixedArray(elem, n) => self.fixed_copy(val, elem, *n),
						_ => self.copy_bind(val, &typ),
					};
					// `:=` always declares a fresh binding, shadowing any earlier ones
					self.bind_local(name, final_val, typ, *mutable);
				}

				// pattern bindings
				Expr::PatBind { pat, value, mutable } => {
					let (ptr, typ) = self.expr(value)?;
					if i + 1 == stmts.len() {
						last = (ptr, typ.clone());
					}
					self.bind_pat(pat, ptr, &typ, *mutable)?;
				}

				Expr::Assign { name, value } => {
					let local = self.mutable_local(name, stmt.1.into_range(), Mutation::Assign)?;
					let (val, typ) = self.check_expr(value, &local.typ)?;
					if typ != local.typ {
						return Err(Diagnostic::new(
							format!("cannot assign {typ} to `{name}`, which is {}", local.typ),
							value.1.into_range(),
						)
						.with_label("type mismatch"));
					}
					self.move_resource(value, &typ)?;
					if let Typ::Struct(_, ref fields) = typ {
						let fields = fields.clone();
						let dst = self.read_local(&local);
						if self.is_resource(&typ) {
							self.release_value(dst, &typ);
						}
						self.assign_fields(val, dst, &fields, true);
						self.settle(val, dst, &typ);
					} else {
						let val = self.copy_in(val, &typ);
						let old = self.read_local(&local);
						self.write_local(&local, val);
						self.release_value(old, &typ);
					}
				}

				Expr::IndexAssign { name, index, value } => {
					let local = self.mutable_local(name, stmt.1.into_range(), Mutation::IndexAssign)?;
					if let Typ::Map(k, v) = local.typ.clone() {
						let (k, v) = (*k, *v);
						let (tag, key_bits) = self.map_key(index, &k)?;
						let (val, vtyp) = self.check_expr(value, &v)?;
						if vtyp != v {
							return Err(Diagnostic::new(
								format!("cannot assign {vtyp} to {v} value of map"),
								value.1.into_range(),
							)
							.with_label("type mismatch"));
						}
						closure_escape(&v, value.1.into_range(), "stored in a map")?;
						self.move_resource(value, &v)?;
						let val = self.copy_in(val, &v);
						let val_bits = self.map_bits(val);
						let ptr = self.read_local(&local);
						let ptr = self.call_map_set(ptr, tag, key_bits, val_bits);
						self.write_local(&local, ptr);
						continue;
					}
					let elem = match &local.typ {
						Typ::Array(e) | Typ::FixedArray(e, _) => (**e).clone(),
						Typ::Str => {
							return Err(Diagnostic::new(
								format!("cannot assign into `{name}`, strings are immutable"),
								stmt.1.into_range(),
							)
							.with_label("cannot assign into a string"));
						}
						_ => {
							return Err(
								Diagnostic::new(format!("`{name}` is not an array"), stmt.1.into_range())
									.with_label("not an array"),
							);
						}
					};
					let ptr = self.read_local(&local);
					// a shared buffer clones before the element write
					if matches!(local.typ, Typ::Array(_)) {
						self.cow_array(ptr, &elem);
					}
					let idx = self.int_value(index, "array index")?;
					let idx = self.intcast(idx, self.int, true);
					let (val, vtyp) = self.check_expr(value, &elem)?;
					if vtyp != elem {
						return Err(Diagnostic::new(
							format!("cannot assign {vtyp} to element of {elem} array"),
							value.1.into_range(),
						)
						.with_label("type mismatch"));
					}
					closure_escape(&vtyp, value.1.into_range(), "stored in an array")?;
					let val = self.copy_in(val, &vtyp);
					let (data, len) = self.array_parts(ptr, &local.typ);
					self.store_index(data, len, &elem, idx, val);
				}

				Expr::MapDelete { name, key } => {
					let local = self.mutable_local(name, stmt.1.into_range(), Mutation::IndexAssign)?;
					let Typ::Map(k, _) = local.typ.clone() else {
						return Err(Diagnostic::new(format!("`{name}` is not a map"), stmt.1.into_range())
							.with_label("not a map"));
					};
					let (tag, key_bits) = self.map_key(key, &k)?;
					let ptr = self.read_local(&local);
					let ptr = self.call_map_delete(ptr, tag, key_bits);
					self.write_local(&local, ptr);
				}

				Expr::Append { name, value } => {
					let local = self.mutable_local(name, stmt.1.into_range(), Mutation::Append)?;
					let elem = match &local.typ {
						Typ::Array(e) => (**e).clone(),
						_ => {
							return Err(
								Diagnostic::new(format!("`{name}` is not an array"), stmt.1.into_range())
									.with_label("not an array"),
							);
						}
					};
					let (val, vtyp) = self.check_expr(value, &elem)?;
					let stride = self.elem_stride(&elem);
					let size = self.b.ins().iconst(self.int, stride);
					let ptr = self.read_local(&local);
					// a shared buffer clones before it grows
					self.cow_array(ptr, &elem);

					if vtyp == elem {
						closure_escape(&vtyp, value.1.into_range(), "stored in an array")?;
						let val = self.copy_in(val, &elem);
						// grow if full, then write the new element and bump len
						let len = self.array_len(ptr);
						let cap = self.array_cap(ptr);
						let full = self.b.ins().icmp(IntCC::Equal, len, cap);
						let grow_block = self.b.create_block();
						let ok_block = self.b.create_block();
						self.b.ins().brif(full, grow_block, &[], ok_block, &[]);
						self.b.seal_block(grow_block);

						self.b.switch_to_block(grow_block);
						let min_cap = self.b.ins().iadd_imm(len, 1);
						self.rt_call("array_reserve", &[ptr, min_cap, size]);
						self.b.ins().jump(ok_block, &[]);
						self.b.seal_block(ok_block);

						self.b.switch_to_block(ok_block);
						let len = self.array_len(ptr);
						let data = self.array_data(ptr);
						let off = self.b.ins().imul_imm(len, stride);
						let addr = self.b.ins().iadd(data, off);
						self.store_elem(addr, 0, &elem, val);
						let new_len = self.b.ins().iadd_imm(len, 1);
						self.b.ins().store(MemFlags::new(), new_len, ptr, 8);
					} else if vtyp == Typ::Array(Box::new(elem.clone())) {
						self.rt_call("array_extend", &[ptr, val, size]);
					} else {
						return Err(Diagnostic::new(
							format!("cannot append {vtyp} to {elem} array"),
							value.1.into_range(),
						)
						.with_label("type mismatch"));
					}
				}

				Expr::Return(value) => {
					let (val, typ) = match value {
						Some(e) => match self.ret.clone() {
							Some((target, _)) => self.check_expr(e, &target)?,
							None => self.expr(e)?,
						},
						None => {
							let typ = self.ret.as_ref().map_or(Typ::unit(), |(t, _)| t.clone());
							(self.zero(&typ), typ)
						}
					};
					if let Some(e) = value {
						self.move_resource(e, &typ)?;
					}
					self.emit_return(val, typ, stmt.1)?;
					return Ok(None);
				}

				Expr::If { cond, then, els } => {
					match self.conditional(cond, then, els.as_deref(), stmt_target, stmt.1)? {
						Some((v, t)) => last = (v, t),
						None => return Ok(None),
					}
				}

				Expr::Match {
					subject,
					arms,
					else_body,
				} => match self.match_expr(subject, arms, else_body.as_deref(), stmt_target, stmt.1)? {
					Some((v, t)) => last = (v, t),
					None => return Ok(None),
				},

				Expr::Loop { cond, body } => match self.loop_expr(cond.as_deref(), body)? {
					Some((v, t)) => last = (v, t),
					None => return Ok(None),
				},

				// TODO: revisit after adding the Iterator trait
				Expr::For { pat, iter, body } => last = self.for_loop(pat, iter, body)?,

				Expr::FieldAssign { name, field, value } => {
					let local = self.mutable_local(name, stmt.1.into_range(), Mutation::FieldAssign)?;
					let fields = match self.peeled(&local.typ) {
						Typ::Struct(sname, fields) => {
							self.check_member(&sname, field, stmt.1)?;
							fields
						}
						_ => {
							return Err(
								Diagnostic::new(format!("`{name}` is not a struct"), stmt.1.into_range())
									.with_label("not a struct"),
							);
						}
					};
					// writes fall through embeds
					let (path, idx, ftyp) = match fields.iter().position(|f| &f.name == field) {
						Some(i) => (Vec::new(), i, fields[i].typ.clone()),
						None => match self.promoted(&fields, field, stmt.1)? {
							Some((p, i, t)) => (p, i, t),
							None => {
								return Err(Diagnostic::new(
									format!("struct has no field `{field}`"),
									stmt.1.into_range(),
								)
								.with_label("no such field"));
							}
						},
					};
					let (val, vtyp) = self.check_expr(value, &ftyp)?;
					if vtyp != ftyp {
						return Err(Diagnostic::new(
							format!("cannot assign {vtyp} to field `{field}` of type {ftyp}"),
							value.1.into_range(),
						)
						.with_label("type mismatch"));
					}
					closure_escape(&vtyp, value.1.into_range(), "stored in a field")?;
					let val = self.copy_in(val, &vtyp);
					let base = self.read_local(&local);
					let ptr = self.follow(base, &path);
					if rc::releasable(&vtyp) {
						let cl = self.b.func.dfg.value_type(val);
						let old = self.b.ins().load(cl, MemFlags::new(), ptr, (idx * 8) as i32);
						self.release_value(old, &vtyp);
					}
					self.b.ins().store(MemFlags::new(), val, ptr, (idx * 8) as i32);
				}

				Expr::Break(payload) => {
					let Some(&LoopFrame {
						depth,
						exit,
						fallthrough,
						..
					}) = self.loops.last()
					else {
						return Err(Diagnostic::new("`break` outside of a loop", stmt.1.into_range())
							.with_label("not inside a loop"));
					};
					// the first `break` creates the exit block
					let exit = exit.unwrap_or_else(|| {
						let exit = self.b.create_block();
						self.loops.last_mut().unwrap().exit = Some(exit);
						exit
					});
					// a bare `break` yields unit, so mixing it with a break-with-value is a type mismatch
					let (v, t) = match payload {
						Some(e) => {
							let (v, t) = self.expr(e)?;
							match fallthrough {
								// a loop that can end without breaking yields an Option
								Some(_) => (self.make_option(&t, Some(v)), Typ::Option(Box::new(t))),
								None => (self.copy_in(v, &t), t),
							}
						}
						None => self.unit_value(),
					};
					self.release_scopes(depth);
					let mut result = self.loops.last_mut().unwrap().result.take();
					self.contribute("break", (v, t), &mut result, exit, stmt.1)?;
					self.loops.last_mut().unwrap().result = result;
					return Ok(None);
				}

				Expr::Continue => {
					let (top, depth) = match self.loops.last() {
						Some(frame) => (frame.top, frame.depth),
						None => {
							return Err(Diagnostic::new("`continue` outside of a loop", stmt.1.into_range())
								.with_label("not inside a loop"));
						}
					};
					self.release_scopes(depth);
					self.b.ins().jump(top, &[]);
					return Ok(None);
				}

				Expr::Doc(_) => {}

				_ => {
					last = match stmt_target {
						Some(t) => self.check_expr(stmt, t)?,
						None => self.expr(stmt)?,
					}
				}
			}
		}

		// trailing places
		if let Some(read) = stmts.last().and_then(place_read) {
			last = match tail {
				Some(t) if t.is_unit() || fallible(t) => self.unit_value(),
				Some(t) => self.check_expr(&read, t)?,
				None => self.expr(&read)?,
			};
		}

		Ok(Some(last))
	}

	// Autowrap return types.
	// idk whether it'll be more general in the future, but for now this is for `Option` and `Result`.
	fn autowrap_return(&mut self, val: Value, typ: Typ, span: Span) -> Result<TypedVal, Diagnostic> {
		let ret = self.ret.as_ref().map(|(t, _)| t.clone());
		let (val, typ) = match &ret {
			Some(Typ::Option(inner) | Typ::Result(inner, _)) => self.coerce(val, &typ, inner, span)?,
			_ => (val, typ),
		};
		Ok(match ret {
			Some(Typ::Option(inner)) if typ == *inner => {
				let v = self.make_option(&inner, Some(val));
				(v, Typ::Option(inner))
			}
			Some(Typ::Result(ok, err)) if typ == *ok => {
				let v = self.make_enum(&result_variants(&ok, &err), 0, &[val]);
				(v, Typ::Result(ok, err))
			}
			Some(Typ::Result(ok, err)) if typ == *err => {
				let v = self.make_enum(&result_variants(&ok, &err), 1, &[val]);
				(v, Typ::Result(ok, err))
			}
			Some(Typ::Result(ok, err)) if *err == Typ::Error && self.open_error(&typ) => {
				let boxed = self.box_error(val, &typ);
				let v = self.make_enum(&result_variants(&ok, &err), 1, &[boxed]);
				(v, Typ::Result(ok, err))
			}
			_ => (val, typ),
		})
	}

	// The first return fixes the fn's type, and later returns must agree.
	pub fn emit_return(&mut self, val: Value, typ: Typ, span: Span) -> Result<(), Diagnostic> {
		let (val, typ) = self.autowrap_return(val, typ, span)?;
		if self.is_main && !typ.is_unit() && !fallible(&typ) {
			if self.script {
				self.emit_print(val, &typ, false, runtime::Sink::Out);
				self.write_lit("\n", runtime::Sink::Out);
			}
			let (val, typ) = self.unit_value();
			return self.emit_return(val, typ, span);
		}
		closure_escape(&typ, span.into_range(), "returned")?;
		if let Some((declared, _)) = &self.ret
			&& &typ != declared
		{
			return Err(Diagnostic::new(
				format!("expected {declared} return value, got {typ}"),
				span.into_range(),
			)
			.with_label("wrong return type"));
		}
		if typ.is_unit() {
			self.release_scopes(0);
			self.b.ins().return_(&[]);
			if self.ret.is_none() {
				self.ret = Some((typ, span));
			}
			return Ok(());
		}
		let val = self.copy_in(val, &typ);
		// `copy_in` already heap-copied structs, but fixed arrays still need to escape the frame
		let final_val = match &typ {
			Typ::FixedArray(elem, n) => {
				let (elem, n) = ((**elem).clone(), *n);
				let stride = self.elem_stride(&elem);
				let heap = self.call_alloc_bytes(n as i64 * stride);
				for i in 0..n {
					let off = (i as i64 * stride) as i32;
					let v = self.load_elem(val, off, &elem);
					self.store_elem(heap, off, &elem, v);
				}
				heap
			}
			_ => val,
		};
		// the bumped return value survives the walk over everything this fn owned
		self.release_scopes(0);
		// the cranelift signature takes its return type from the first return
		if self.b.func.signature.returns.is_empty() {
			self.b.func.signature.returns.push(AbiParam::new(cl_type(&typ, self.int)));
		}
		self.b.ins().return_(&[final_val]);
		if self.ret.is_none() {
			self.ret = Some((typ, span));
		}
		Ok(())
	}
}

// Expression left behind by a place statement.
fn place_read(stmt: &Spanned<Expr>) -> Option<Spanned<Expr>> {
	let name = |n: &String| Box::new((Expr::Ident(n.clone()), stmt.1));
	let read = match &stmt.0 {
		Expr::Bind { name, .. }
		| Expr::Assign { name, .. }
		| Expr::Append { name, .. }
		| Expr::MapDelete { name, .. } => Expr::Ident(name.clone()),
		Expr::IndexAssign { name: n, index, .. } => Expr::Index {
			collection: name(n),
			index: index.clone(),
		},
		Expr::FieldAssign { name: n, field, .. } => Expr::Field {
			tuple: name(n),
			field: field.clone(),
		},
		_ => return None,
	};
	Some((read, stmt.1))
}
