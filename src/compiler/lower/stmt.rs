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
			let want = i + 1 == stmts.len();
			let stmt_target = if want { tail } else { None };
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
					if *mutable {
						check_reserved(name, stmt.1)?;
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
								let msg = "a reference must be initialized (`?^T` for an optional one)";
								return Err(
									Diagnostic::new(msg, stmt.1.into_range()).with_label("no zero value for `^T`")
								);
							}
							// a nozero binding starts unassigned, and stays unreadable until assigned
							if self.nozero(&target).is_some() {
								self.slots.push(name.clone());
							}
							(self.zero(&target), target)
						}
						(None, None) => unreachable!("binding has neither a type nor a value"),
					};
					if name == CTX && typ.ctx_path(crate::compiler::CONTEXT).is_none() {
						return Err(Diagnostic::new(format!("`ctx` can't be {typ}"), stmt.1.into_range())
							.with_label("needs `Context` or a struct embedding one"));
					}
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

				Expr::Assign { name, value } | Expr::DerefAssign { name, value } => {
					let deref = matches!(stmt.0, Expr::DerefAssign { .. });
					let mutation = if deref { Mutation::DerefAssign } else { Mutation::Assign };
					let mut local = self.mutable_local(name, stmt.1.into_range(), mutation)?;
					if deref {
						let typ = self.pointee(&local.typ, stmt.1)?;
						let var = self.b.declare_var(self.int);
						let handle = self.read_local(&local);
						self.b.def_var(var, handle);
						local = Local {
							boxed: !matches!(typ, Typ::Struct(..)),
							..Local::plain(var, typ, true)
						};
					}
					let (val, typ) = self.check_expr(value, &local.typ)?;
					if typ != local.typ {
						return Err(Diagnostic::new(
							format!("cannot assign {typ} to `{name}`, which is {}", local.typ),
							value.1.into_range(),
						)
						.with_label("type mismatch"));
					}
					self.move_resource(value, &typ)?;
					self.slots.retain(|s| s != name);
					if let Typ::Struct(_, ref fields) = typ {
						let fields = fields.clone();
						let dst = self.read_local(&local);
						let resource = self.is_resource(&typ);
						if resource {
							self.release_value(dst, &typ);
						}
						self.assign_fields(val, dst, &fields, !resource);
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
						let val = self.stored(value, &v, &format!("{v} value of map"), "a map")?;
						self.move_resource(value, &v)?;
						let val = self.copy_in(val, &v);
						let val_bits = self.map_bits(val);
						let ptr = self.read_local(&local);
						let ptr = self.map_rt("set", ptr, tag, key_bits, &[val_bits]);
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
						t if self.claims(t, role::INDEX_ASSIGN) => {
							let call = Expr::MethodCall {
								recv: Box::new((Expr::Ident(name.clone()), stmt.1)),
								method: "index_assign".into(),
								type_args: vec![],
								args: vec![(**index).clone(), (**value).clone()],
							};
							self.expr(&(call, stmt.1))?;
							continue;
						}
						_ => {
							return Err(
								Diagnostic::new(format!("`{name}` is not an array"), stmt.1.into_range())
									.with_label(format!("implement `{}` to assign into it", role::INDEX_ASSIGN)),
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
					let val = self.stored(value, &elem, &format!("element of {elem} array"), "an array")?;
					let val = self.copy_in(val, &elem);
					let (data, len) = self.array_parts(ptr, &local.typ);
					self.store_index(data, len, &elem, idx, val, stmt.1);
				}

				Expr::MapDelete { name, key } => {
					let local = self.mutable_local(name, stmt.1.into_range(), Mutation::IndexAssign)?;
					let Typ::Map(k, _) = local.typ.clone() else {
						return Err(Diagnostic::new(format!("`{name}` is not a map"), stmt.1.into_range())
							.with_label("not a map"));
					};
					let (tag, key_bits) = self.map_key(key, &k)?;
					let ptr = self.read_local(&local);
					let ptr = self.map_rt("delete", ptr, tag, key_bits, &[]);
					self.write_local(&local, ptr);
				}

				Expr::Append { name, value, .. } => {
					self.mutable_local(name, stmt.1.into_range(), Mutation::Append)?;
					let (ptr, typ) = self.expr(&place_read(stmt).unwrap())?;
					let elem = match &typ {
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
						self.st(ptr, 8, new_len);
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
							(self.zero_or_err(&typ, stmt.1)?, typ)
						}
					};
					if let Some(e) = value {
						self.move_resource(e, &typ)?;
					}
					self.emit_return(val, typ, stmt.1)?;
					return Ok(None);
				}

				Expr::If { .. } | Expr::Match { .. } | Expr::Loop { .. } => {
					match self.branching(stmt, stmt_target, want)? {
						Some((v, t)) => last = (v, t),
						None => return Ok(None),
					}
				}

				Expr::Block(body) if stmt_target.is_none() => match self.scoped(|s| s.block_tail(body, None))? {
					Some((v, t)) => last = (v, t),
					None => return Ok(None),
				},

				Expr::MacroCall { name, .. } if matches!(name.as_str(), "panic" | "todo" | "unreachable") => {
					self.expr(stmt)?;
					self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);
					return Ok(None);
				}

				// TODO: revisit after adding the Iterator trait
				Expr::For { pat, iter, body } => last = self.looped(|s| s.for_loop(pat, iter, body))?,

				Expr::FieldAssign { name, field, value } => {
					self.check_static_write(name, field, stmt.1)?;
					let local = self.mutable_local(name, stmt.1.into_range(), Mutation::FieldAssign)?;
					let (path, idx, ftyp) = match self.peeled(&local.typ) {
						Typ::Tuple(elems) => {
							let i = tuple_index(&elems, field, stmt.1)?;
							(Vec::new(), i, elems[i].1.clone())
						}
						// writes fall through embeds
						Typ::Struct(sname, fields) => self.struct_field(&sname, &fields, field, stmt.1)?,
						_ => {
							return Err(
								Diagnostic::new(format!("`{name}` is not a struct"), stmt.1.into_range())
									.with_label("not a struct"),
							);
						}
					};
					let val = self.stored(value, &ftyp, &format!("field `{field}` of type {ftyp}"), "a field")?;
					let val = self.copy_in(val, &ftyp);
					let base = self.read_local(&local);
					let ptr = self.follow(base, &path);
					if rc::owns(&ftyp) {
						let cl = self.b.func.dfg.value_type(val);
						let old = self.b.ins().load(cl, MemFlags::new(), ptr, (idx * 8) as i32);
						self.release_field(old, &ftyp);
					}
					self.st(ptr, (idx * 8) as i32, val);
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
								Some(_) => {
									let ot = self.types.core_enum(role::OPTION, &[t]);
									(self.make_option(&ot, Some(v)), ot)
								}
								None => (self.copy_in(v, &t), t),
							}
						}
						None => self.unit_value(),
					};
					self.release_scopes(depth, None)?;
					let mut join = control::Join::new("break", stmt.1, self.loops.last_mut().unwrap().result.take());
					self.contribute((v, t), &mut join, exit)?;
					self.loops.last_mut().unwrap().result = join.result;
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
					self.release_scopes(depth, None)?;
					self.b.ins().jump(top, &[]);
					return Ok(None);
				}

				Expr::Doc(_) => {}

				Expr::Defer { body, on_err } => self.defers.last_mut().expect("scope").push(rc::Defer {
					body: (**body).clone(),
					vars: self.vars.clone(),
					on_err: *on_err,
				}),

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
				Some(t) if t.is_unit() || self.types.fallible(t) => self.unit_value(),
				Some(t) => self.check_expr(&read, t)?,
				None => self.expr(&read)?,
			};
		}

		Ok(Some(last))
	}

	// Lower a value for a store into the desired slot.
	fn stored(&mut self, value: &Spanned<Expr>, want: &Typ, what: &str, place: &str) -> Result<Value, Diagnostic> {
		let (val, vtyp) = self.check_expr(value, want)?;
		if &vtyp != want {
			return Err(
				Diagnostic::new(format!("cannot assign {vtyp} to {what}"), value.1.into_range())
					.with_label("type mismatch"),
			);
		}
		closure_escape(want, value.1.into_range(), &format!("stored in {place}"))?;
		Ok(val)
	}

	// Autowrap return types.
	// idk whether it'll be more general in the future, but for now this is for the propagators (Option/Result).
	fn autowrap_return(&mut self, val: Value, typ: Typ, span: Span) -> Result<TypedVal, Diagnostic> {
		let Some(ret) = self.ret.as_ref().map(|(t, _)| t.clone()) else {
			return Ok((val, typ));
		};
		if let Some(some) = self.types.option_inner(&ret) {
			let (val, typ) = self.coerce(val, &typ, &some, span)?;
			return Ok(match typ == some {
				true => (self.make_option(&ret, Some(val)), ret),
				false => (val, typ),
			});
		}
		let Some((ok, err)) = self.types.result_parts(&ret) else {
			return Ok((val, typ));
		};
		let (val, typ) = self.coerce(val, &typ, &ok, span)?;
		let (val, typ) = match typ == ok {
			true => (val, typ),
			false => self.coerce(val, &typ, &err, span)?,
		};
		let variants = self.variants_of(&ret);
		Ok(if typ == ok {
			(self.make_enum(&variants, 0, &[val]), ret)
		} else if typ == err {
			(self.make_enum(&variants, 1, &[val]), ret)
		} else {
			(val, typ)
		})
	}

	// The first return fixes the fn's type, and later returns must agree.
	pub fn emit_return(&mut self, val: Value, typ: Typ, span: Span) -> Result<(), Diagnostic> {
		if self.deferring {
			return Err(Diagnostic::new("cannot return from a defer body", span.into_range())
				.with_label("every exit path already runs this deferred body"));
		}
		let (val, typ) = self.autowrap_return(val, typ, span)?;
		if self.is_main && !typ.is_unit() && !self.types.fallible(&typ) {
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
			self.release_scopes(0, Some((val, typ.clone())))?;
			self.b.ins().return_(&[]);
			if self.ret.is_none() {
				self.ret = Some((typ, span));
			}
			return Ok(());
		}
		let final_val = self.copy_in(val, &typ);
		self.release_scopes(0, Some((final_val, typ.clone())))?;
		// take return type from the first return
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
		| Expr::Append { name, field: None, .. }
		| Expr::MapDelete { name, .. } => Expr::Ident(name.clone()),
		Expr::IndexAssign { name: n, index, .. } => Expr::Index {
			collection: name(n),
			index: index.clone(),
		},
		Expr::FieldAssign { name: n, field, .. }
		| Expr::Append {
			name: n,
			field: Some(field),
			..
		} => Expr::Field {
			tuple: name(n),
			field: field.clone(),
		},
		Expr::DerefAssign { name: n, .. } => Expr::Deref(name(n)),
		_ => return None,
	};
	Some((read, stmt.1))
}
