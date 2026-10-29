use crate::compiler::role;

use super::*;

impl<'a, M: Module> Translator<'a, M> {
	pub fn expr(&mut self, expr: &Spanned<Expr>) -> Result<TypedVal, Diagnostic> {
		self.lower(expr, None)
	}

	// Get the array place for an append operation, if any.
	fn append_target(&self, l: &Spanned<Expr>) -> Option<(String, Option<String>)> {
		match &l.0 {
			Expr::Ident(n) => matches!(self.vars.get(n)?.typ, Typ::Array(_)).then(|| (n.clone(), None)),
			Expr::Field { tuple, field } => {
				let Expr::Ident(n) = &tuple.0 else { return None };
				let Typ::Struct(s, fs) = self.peeled(&self.vars.get(n)?.typ) else {
					return None;
				};
				let (.., t) = self.struct_field(&s, &fs, field, l.1).ok()?;
				matches!(t, Typ::Array(_)).then(|| (n.clone(), Some(field.clone())))
			}
			Expr::Binary(BinOp::Shl, l, _) => self.append_target(l),
			_ => None,
		}
	}

	// The local under a `<<` overload that appends rather than shifts.
	fn grow_target(&self, l: &Spanned<Expr>) -> Option<String> {
		match &l.0 {
			Expr::Ident(n) => {
				let sig = self.sole_fill(&format!("{}.shl", self.vars.get(n)?.typ.nominal()?))?;
				(!matches!(sig.params[1].typ, Typ::Int(_))).then(|| n.clone())
			}
			Expr::Binary(BinOp::Shl, l, _) => self.grow_target(l),
			_ => None,
		}
	}

	// `recv[index]` through its `Index` claim.
	fn index_call(&mut self, (ptr, typ): TypedVal, index: &Spanned<Expr>, span: Span) -> Result<TypedVal, Diagnostic> {
		let recv = format!("$recv{}", self.vars.len());
		self.hidden_local(recv.clone(), ptr, typ);
		let recv = Box::new((Expr::Ident(recv), span));
		let call = Expr::MethodCall {
			recv,
			method: "index".into(),
			type_args: vec![],
			args: vec![index.clone()],
		};
		self.expr(&(call, span))
	}

	pub(super) fn lower(&mut self, expr: &Spanned<Expr>, hint: Option<&Typ>) -> Result<TypedVal, Diagnostic> {
		if let Some(t) = hint
			&& let Some(v) = self.coerce_lit(expr, t)?
		{
			return Ok((v, t.clone()));
		}
		match &expr.0 {
			Expr::Int(n) => match int_word(*n) {
				Some(bits) => Ok((self.b.ins().iconst(types::I64, bits), Typ::Int(64))),
				None => fail(format!("{n} is out of range for int"), expr.1, "needs a wider type"),
			},
			Expr::Rune(n) => Ok((self.b.ins().iconst(types::I32, *n), Typ::Rune)),
			Expr::Bool(v) => Ok((self.b.ins().iconst(self.int, *v as i64), Typ::Bool)),
			Expr::Float(x) => Ok((self.b.ins().f64const(*x), Typ::Float(64))),
			Expr::String(s) => Ok((self.str_const(s), Typ::Str)),
			Expr::Atom(name) => Ok((self.atom_const(name), Typ::Atom)),

			Expr::EnumShorthand { variant, .. } => Err(Diagnostic::new(
				format!("cannot infer the enum type of `.{variant}` here"),
				expr.1.into_range(),
			)
			.with_label("no enum type is expected in this position")
			.with_note(format!("qualify it, e.g. `Color.{variant}`"))),

			Expr::ArgMod(a, _) => fail(
				format!("`{a}` is only allowed on call arguments"),
				expr.1,
				"not a call argument",
			),

			Expr::Foreign => fail(
				"foreign is only allowed as a module-level binding",
				expr.1,
				"not a module-level binding",
			),

			Expr::Ident(name) => match self.local(name, expr.1.into_range()) {
				Ok(local) => {
					let val = self.read_local(&local);
					Ok((val, local.typ))
				}
				Err(e) if self.vars.contains_key(name) => Err(e),
				Err(e) => match self.funcs.get(self.qualify(name).as_ref()).cloned() {
					Some(sig) => {
						if sig.unsafe_call {
							self.require_unsafe(name, expr.1)?;
						}
						let obj = self.fn_object(sig.id);
						Ok((obj, sig.value_typ()))
					}
					None => {
						let key = self.qualify(name);
						let visible = key.contains("::") || !self.script;
						match visible.then(|| self.types.consts.map.get(key.as_ref()).cloned()).flatten() {
							Some(c) => self.expr(&c),
							None => match self.types.type_params.get(name) {
								Some(Typ::Const(lit)) => self.lower(&((**lit).clone(), expr.1), hint),
								_ => match self.through_with(expr)? {
									Some(m) => self.lower(&m, hint),
									None => self
										.lower(&(Expr::TypePat(TypeExpr::Name(name.clone())), expr.1), hint)
										.map_err(|_| e),
								},
							},
						}
					}
				},
			},

			Expr::Dollar => Ok(self.dollar()),

			Expr::Is { subject, typ, negated } => {
				let tn = match typ {
					TypeExpr::Name(n) => self.types.scope.env.get(n).unwrap_or(n).clone(),
					_ => String::new(),
				};
				if let Expr::Call { name, args, .. } = &subject.0
					&& let ("type", [x]) = (name.as_str(), args.as_slice())
				{
					let typ = self.type_of(x)?;
					let holds = self.claims(&typ, &tn) ^ negated;
					return Ok((self.b.ins().iconst(self.int, holds as i64), Typ::Bool));
				}
				if let Some(te) = TypeExpr::from_expr(&subject.0)
					&& let TypeExpr::Name(name) | TypeExpr::Generic(name, _) = &te
					&& !self.vars.contains_key(name)
				{
					let typ = self.types.resolve(&te, subject.1)?;
					let holds = self.claims(&typ, &tn) ^ negated;
					return Ok((self.b.ins().iconst(self.int, holds as i64), Typ::Bool));
				}
				let (obj, vt) = self.expr(subject)?;
				let cc = match negated {
					true => IntCC::NotEqual,
					false => IntCC::Equal,
				};
				let (sv, st) = self.deref(obj, &vt);
				if st.is_enumish() {
					let (disc, binds) = self.enum_pattern(expr, &st)?;
					if !binds.is_empty() {
						return fail(format!("`{st}` is checked by its variants"), expr.1, "ex: `.some`");
					}
					let tag = self.enum_tag(&st, sv);
					let hit = self.b.ins().icmp_imm(cc, tag, disc);
					return Ok((self.b.ins().uextend(self.int, hit), Typ::Bool));
				}
				let tn = match &vt {
					Typ::Trait(tn) => tn.clone(),
					Typ::Error => role::ERROR.to_string(),
					_ => {
						return fail(
							"`is` takes a type name or a trait object on the left",
							subject.1,
							format!("this is {vt}"),
						);
					}
				};
				let typ = self.types.resolve(typ, expr.1)?;
				if !self.world.trait_impls.contains(&(typ.key(), tn.clone())) {
					return Ok((self.b.ins().iconst(self.int, *negated as i64), Typ::Bool));
				}
				let want = self.data_addr(&oi_symbol(&format!("vtable_{}_{tn}", typ.key())));
				let got = self.ld_word(obj, 0);
				let hit = self.b.ins().icmp(cc, got, want);
				Ok((self.b.ins().uextend(self.int, hit), Typ::Bool))
			}

			Expr::Negative(e) => {
				let (v, typ) = self.expr(e)?;
				let out = match self.own_field(&typ, role::NEG) {
					Typ::Int(_) => self.b.ins().ineg(v),
					Typ::Float(16) => {
						let wide = self.widen_f16(v, &typ);
						let neg = self.b.ins().fneg(wide);
						self.numcast(neg, &Typ::Float(32), &typ)
					}
					Typ::Float(_) => self.b.ins().fneg(v),
					t if t.nominal().is_some() => match self.fill(t, role::NEG, "neg", 1) {
						Some(sig) => return Ok(self.emit_call(&sig, &[v])),
						None => {
							return fail(format!("cannot negate {typ}"), expr.1, format!("claim `Neg` for `{t}`"));
						}
					},
					_ => {
						return fail(format!("cannot negate {typ}"), expr.1, format!("this is {typ}"));
					}
				};
				Ok((out, typ))
			}

			Expr::Binary(op, l, r) => match op {
				BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
					let (icc, fcc) = cmp_cc(*op);
					self.cmp(icc, fcc, l, r, expr.1)
				}
				BinOp::And => self.logical(true, l, r),
				BinOp::Or => self.logical(false, l, r),
				BinOp::In => self.in_op(l, r),
				BinOp::Shl => match self.append_target(l) {
					// arrays append, ints shift
					Some((name, field)) => {
						self.expr(l)?;
						let value = r.clone();
						self.lower(&(Expr::Append { name, field, value }, expr.1), hint)
					}
					// overloaded appending
					None if let Some(name) = self.grow_target(l) => {
						self.expr(l)?;
						let (recv, args) = (Box::new((Expr::Ident(name.clone()), l.1)), vec![(**r).clone()]);
						let call = Expr::MethodCall {
							recv,
							method: "shl".into(),
							type_args: vec![],
							args,
						};
						let value = Box::new((call, expr.1));
						self.lower(&(Expr::Assign { name, value }, expr.1), hint)
					}
					None => self.binop(*op, l, r, expr.1),
				},
				_ => self.binop(*op, l, r, expr.1),
			},
			Expr::Not(e) => {
				let (v, typ) = self.expr(e)?;
				let out = match self.own_field(&typ, role::NOT) {
					t if *t == Typ::Bool => self.b.ins().bxor_imm(v, 1),
					t @ (Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize) => {
						let v = self.b.ins().bnot(v);
						self.narrow(v, t)
					}
					t if t.nominal().is_some() => match self.fill(t, role::NOT, "not", 1) {
						Some(sig) => return Ok(self.emit_call(&sig, &[v])),
						None => {
							return fail(
								format!("cannot apply `!` to {typ}"),
								expr.1,
								format!("claim `Not` for `{t}`"),
							);
						}
					},
					_ => {
						return fail(
							format!("expected Bool or an integer, got {typ}"),
							expr.1,
							"`!` needs a Bool or integer operand",
						);
					}
				};
				Ok((out, typ))
			}
			Expr::Recv(e) => {
				let (v, typ) = self.expr(e)?;
				let sig = (typ.nominal()).and_then(|n| {
					self.fill(&typ, role::RECV, "recv", 1)
						.or_else(|| self.sole_fill(&format!("{n}.recv")))
				});
				match sig {
					Some(sig) => Ok(self.emit_call(&sig, &[v])),
					None => fail(
						format!("cannot receive from {typ}"),
						expr.1,
						"claim `Recv` to overload `<-`",
					),
				}
			}

			Expr::Call { name, type_args, args } => {
				let qn = self.qualify(name).to_string();
				self.check_type_args(name, &qn, type_args, expr.1)?;
				if let Some(local) = self.vars.get(name).cloned() {
					let callee = self.read_local(&local);
					return self.call_value(name, Callee::Object(callee), &local.typ, args, None, expr.1);
				}
				if let Some(out) = self.comp_call(&qn, expr, hint)? {
					return Ok(out);
				}
				match self.builtin_call(name, args, expr.1)? {
					Some(result) => Ok(result),
					None => match self.funcs.get(&qn).cloned() {
						Some(sig) => self.call_sig(name, sig, None, None, args, expr.1),
						None => match self.world.generic_fns.get(&qn).cloned() {
							Some(def) => self.call_generic(&qn, &def, type_args, args, None, expr.1),
							None if matches!(
								self.types.aliases.get(&qn),
								Some(TypeExpr::Fn(..) | TypeExpr::Annotated(..))
							) =>
							{
								self.cast_fn_ptr(&qn, args, expr.1)
							}
							None if matches!(name.as_str(), "assert" | "panic") => {
								fail(format!("`{name}` is a macro"), expr.1, format!("write `{name}!(...)`"))
							}
							None => match self.through_with(expr)? {
								Some(member) => self.lower(&member, hint),
								None => fail(format!("undefined function `{name}`"), expr.1, "not defined"),
							},
						},
					},
				}
			}

			Expr::Cast { target, args } => {
				if let (TypeExpr::Name(local), [(Expr::Ident(field), _)]) = (&target.0, &args[..])
					&& self.vars.contains_key(local)
				{
					let (tuple, field) = (Box::new((Expr::Ident(local.clone()), target.1)), field.clone());
					return self.lower(&(Expr::Field { tuple, field }, expr.1), hint);
				}
				if let TypeExpr::Name(path) = &target.0
					&& let Some((name, variant)) = self.variant_path(path, target.1)
				{
					return self.construct_variant(&name, &variant, args, expr.1);
				}
				let typ = self.types.resolve(&target.0, target.1)?;
				self.cast_to(&typ, args, expr.1)
			}

			Expr::Apply { callee, args } => {
				// a lone value arg parsed as an index, because only the resolver knows what a name is
				if let Expr::Index { collection, index } = &callee.0
					&& let Expr::Ident(n) = &collection.0
					&& !self.vars.contains_key(n)
					&& let Some(def) = self.world.generic_fns.get(self.qualify(n).as_ref()).cloned()
				{
					let (key, arg) = (self.qualify(n).to_string(), [(TypeExpr::Value(index.clone()), index.1)]);
					return self.call_generic(&key, &def, &arg, args, None, expr.1);
				}
				let (val, typ) = self.expr(callee)?;
				self.call_value(&typ.to_string(), Callee::Object(val), &typ, args, None, expr.1)
			}

			Expr::MacroCall { name, args } => self.macro_call(name, args, expr.1),

			Expr::MethodCall {
				recv,
				method,
				type_args,
				args,
			} => {
				let dotted = self.dotted_type(recv);
				let recv = dotted.as_ref().unwrap_or(recv);

				// access to an imported module's function
				if let Some((module, target)) = self.import_item(&recv.0, method, expr.1)? {
					let Expr::Ident(m) = &recv.0 else {
						unreachable!("an import is a name")
					};
					let call = Expr::Call {
						name: format!("{m}.{method}"),
						type_args: type_args.clone(),
						args: args.clone(),
					};
					if let Some(out) = self.comp_call(&format!("{module}::{target}"), &(call, expr.1), hint)? {
						return Ok(out);
					}
					return self.module_call(&module, &target, type_args, args, expr.1);
				}

				// enum payload
				if let Expr::Ident(name) = &recv.0
					&& !self.vars.contains_key(name)
					&& self.types.enums.borrow().contains_key(self.qualify(name).as_ref())
				{
					let name = self.qualify(name).to_string();
					if method == "from" {
						return self.enum_from(&name, args, expr.1);
					}
					if !self.funcs.contains_key(&format!("{name}.{method}"))
						&& self.enum_variants(&name).iter().any(|v| v.name == *method)
					{
						let msg = format!("`{name}.{method}` is a variant, not a method");
						return fail(msg, expr.1, format!("write `{name}.{method}.( … )` or `.{{ … }}`"));
					}
				}

				// generic enum variants
				if let Some(instance) = self.enum_instance(recv)
					&& !self.has_fill(&instance, method)
				{
					let typ = Typ::Enum(instance.clone());
					if method == "from_index" && args.len() == 1 && self.plain_enum(&typ) {
						return self.enum_from_index(typ, args);
					}
					return self.construct_variant(&instance, method, args, expr.1);
				}
				if let Some((n, d)) = self.generic_variant(recv, method) {
					return self.infer_variant(&n, &d, method, args, hint, expr.1);
				}

				// method call is static when `recv` names a type
				let (sname, bound) = if let Expr::Ident(name) = &recv.0
					&& !self.vars.contains_key(name)
					&& (self.types.structs.contains_key(self.qualify(name).as_ref())
						|| self.types.enums.borrow().contains_key(self.qualify(name).as_ref())
						|| self.types.generics.structs.contains_key(self.qualify(name).as_ref())
						|| self.types.generics.enums.contains_key(self.qualify(name).as_ref())
						|| matches!(
							self.types.aliases.get(self.qualify(name).as_ref()),
							Some(TypeExpr::TupleStruct(..))
						)) {
					(self.qualify(name).to_string(), None)
				} else if let Some(instance) = self.enum_instance(recv) {
					(instance, None)
				} else if let Expr::Ident(name) = &recv.0
					&& !self.vars.contains_key(name)
					&& let Some(owner) = self.types.named(name, recv.1).ok().as_ref().and_then(method_owner)
				{
					(owner, None)
				} else {
					let (recv_val, recv_typ) = self.expr(recv)?;
					let (recv_val, recv_typ) = self.deref(recv_val, &recv_typ);
					let has_str_impl = matches!(
						recv_typ,
						Typ::Struct(..) | Typ::TupleStruct(..) | Typ::Enum(..) | Typ::Sum(..) | Typ::CStr | Typ::Rune
					);
					if matches!(method.as_str(), "str" | "repr") && args.is_empty() && !has_str_impl {
						return Ok((self.derived_str(recv_val, &recv_typ, method == "repr"), Typ::Str));
					}
					if method == "len" && args.is_empty() {
						let len = match &recv_typ {
							Typ::Array(_) | Typ::FixedArray(..) | Typ::Str => {
								Some(self.array_parts(recv_val, &recv_typ).1)
							}
							Typ::Map(..) => self.rt_call("map_len", &[recv_val]),
							t if let Some((_, arr)) = t.keyed() => Some(self.array_parts(recv_val, arr).1),
							_ => None,
						};
						if let Some(len) = len {
							return Ok((self.intcast(len, types::I64, true), Typ::Int(64)));
						}
					}
					if matches!(method.as_str(), "index" | "ord")
						&& args.is_empty() && self.plain_enum(&recv_typ)
						&& method_owner(&recv_typ).is_none_or(|o| !self.funcs.contains_key(&format!("{o}.{method}")))
					{
						let tag = self.enum_tag(&recv_typ, recv_val);
						let mut tag = self.intcast(tag, types::I64, true);
						if method == "index" {
							tag = self.enum_index(&recv_typ, tag);
						}
						return Ok((tag, Typ::Int(64)));
					}
					if let Typ::Trait(tn) = &recv_typ {
						return self.dyn_call(recv_val, tn, method, args, expr.1);
					}

					// Error trait
					if recv_typ == Typ::Error {
						return self.dyn_call(recv_val, role::ERROR, method, args, expr.1);
					}
					let Some(owner) = method_owner(&recv_typ) else {
						let msg = format!("`{recv_typ}` has no methods");
						return fail(msg, recv.1, "methods are only defined on structs and primitives");
					};
					(owner, Some((recv_val, recv_typ)))
				};

				// interpolate type names
				if bound.is_none()
					&& method == "str"
					&& args.is_empty()
					&& self.funcs.get(&format!("{sname}.str")).is_none_or(|sig| !sig.params.is_empty())
				{
					let (val, typ) = self.expr(recv)?;
					return Ok((self.derived_str(val, &typ, false), Typ::Str));
				}

				if sname == role::PTR {
					let recv = bound.as_ref().map(|(v, _)| *v);
					match method.as_str() {
						"array" => return self.ptr_array(recv, type_args, args, expr.1),
						m @ ("read" | "write") => return self.ptr_copy(m == "read", recv, type_args, args, expr.1),
						_ => {}
					}
				}

				// treat `T.m(mut x)` like calling a struct method
				if bound.is_none()
					&& let Some((Expr::ArgMod(Access::Mut, x), _)) = args.first()
					&& !self.has_fill(&sname, method)
				{
					let (recv, args) = (x.clone(), args[1..].to_vec());
					let (method, type_args) = (method.clone(), type_args.clone());
					return self.expr(&(
						Expr::MethodCall {
							recv,
							method,
							type_args,
							args,
						},
						expr.1,
					));
				}

				let recv_expr = bound.is_some().then_some(recv);
				self.check_member(&sname, method, expr.1)?;
				let base = rc::base_name(&sname);
				let (key, gkey) = match self.extension(base, method, expr.1)? {
					Some(ext) => (ext.clone(), ext),
					None => (format!("{sname}.{method}"), format!("{base}.{method}")),
				};
				self.check_type_args(method, &gkey, type_args, expr.1)?;

				if let Some(sig) = self.funcs.get(&key).cloned() {
					return self.call_sig(&key, sig, bound.map(|(v, _)| v), recv_expr, args, expr.1);
				}
				if let Some(def) = self.world.generic_fns.get(&gkey).cloned() {
					return self.call_generic(&gkey, &def, type_args, args, bound.zip(recv_expr), expr.1);
				}
				if let Some((sig, args)) = self.pick_fill(&key, bound.is_some() as usize, args)? {
					return self.call_sig(&key, sig, bound.map(|(v, _)| v), recv_expr, &args, expr.1);
				}
				if let Some((_, rt)) = &bound
					&& args.is_empty()
					&& let Some(sig) = self.find_fill(&key, 0, rt)
				{
					return self.call_sig(&key, sig, bound.map(|(v, _)| v), recv_expr, args, expr.1);
				}

				// blanket claims
				if let Some(rt) =
					(bound.as_ref().map(|(_, t)| t.clone())).or_else(|| self.types.named(&sname, expr.1).ok())
					&& let Some((k, def)) = (self.world.blankets.iter().filter(|(tn, _)| self.claims(&rt, tn)))
						.find_map(|(_, p)| self.world.generic_fns.get_key_value(&format!("[{}].{method}", p.name)))
				{
					let (k, def) = (k.clone(), def.clone());
					let targs = match bound {
						Some(_) => type_args.to_vec(),
						None => [vec![(TypeExpr::Name(sname.clone()), expr.1)], type_args.to_vec()].concat(),
					};
					return self.call_generic(&k, &def, &targs, args, bound.zip(recv_expr), expr.1);
				}

				// Display and Debug defaults
				if let Some((v, t)) = &bound
					&& matches!(method.as_str(), "str" | "repr")
					&& args.is_empty()
				{
					return Ok((self.derived_str(*v, t, method == "repr"), Typ::Str));
				}

				// make fn fields callable
				if let Some((recv, Typ::Struct(_, sfields))) = &bound
					&& let Some(i) = sfields.iter().position(|f| f.name == *method)
				{
					let ft = sfields[i].typ.clone();
					let v = self.ld_typ(*recv, (i * 8) as i32, &ft);
					return self.call_value(method, Callee::Object(v), &ft, args, None, expr.1);
				}

				// instance calls pierce embedded structs
				if let Some((recv_val, Typ::Struct(_, sfields))) = &bound {
					let owner = |sn: &str, _: &[FieldDef]| {
						let key = format!("{sn}.{method}");
						self.funcs.get(&key).map(|sig| (key, sig.clone()))
					};
					if let Some((path, (key, sig))) = self.pierce(sfields, method, expr.1, owner)? {
						let embed = self.follow(*recv_val, &path);
						return self.call_sig(&key, sig, Some(embed), recv_expr, args, expr.1);
					}
				}
				Err(unknown_member(format!("`{sname}`"), "method", method, expr.1))
			}

			// tuples
			Expr::Tuple(elems) => {
				if elems.is_empty() {
					return Ok(self.unit_value());
				}
				let want = match hint {
					Some(Typ::Tuple(fs)) if fs.len() == elems.len() => Some(fs),
					_ => None,
				};
				let ptr = self.stack_slot((elems.len() * 8) as u32);
				let mut fields = Vec::with_capacity(elems.len());
				for (i, (name, value)) in elems.iter().enumerate() {
					let (val, typ) = match want {
						Some(fs) => self.check_expr(value, &fs[i].1)?,
						None => self.expr(value)?,
					};
					let typ = word_slot(typ, value.1)?;
					self.move_resource(value, &typ)?;
					let val = self.copy_in(val, &typ);
					self.st(ptr, (i * 8) as i32, val);
					fields.push((name.clone(), typ));
				}
				let typ = Typ::Tuple(fields);
				self.temp(ptr, &typ);
				Ok((ptr, typ))
			}

			Expr::Field { tuple, field } => {
				if let Some(view) = control::place_key(&expr.0).and_then(|key| self.vars.get(&key).cloned()) {
					return Ok((self.read_local(&view), view.typ));
				}
				let dotted = self.dotted_type(tuple);
				let tuple = dotted.as_ref().unwrap_or(tuple);

				// access an imported module's items
				if let Some((module, target)) = self.import_item(&tuple.0, field, expr.1)? {
					let key = format!("{module}::{target}");
					let key = self.world.reexports.get(&key).cloned().unwrap_or(key);
					if let Some(l) = self.vars.get(&key).cloned().filter(|l| l.stat) {
						if !self.world.publics.is_visible(&key, &self.types.scope.module) {
							let msg = format!("`{field}` is private to module `{module}`");
							return fail(msg, expr.1, "not public");
						}
						self.require_pure(field, expr.1)?;
						let val = self.read_local(&l);
						return Ok((val, l.typ));
					}
					let (msg, label) = match self.types.consts.map.get(&key).cloned() {
						Some(c) if self.world.publics.is_visible(&key, &self.types.scope.module) => {
							return self.expr(&c);
						}
						Some(_) => (format!("`{field}` is private to module `{module}`"), "not public"),
						None if self.funcs.contains_key(&key) || self.world.generic_fns.contains_key(&key) => {
							(format!("`{field}` is a function, call it"), "add `()`")
						}
						None => (format!("module `{module}` has no const `{field}`"), "no such const"),
					};
					let d = Diagnostic::new(msg, expr.1.into_range()).with_label(label);
					return Err(crate::loader::shadow_note(d, &module, &self.world.core_origin));
				}

				// associated consts
				if let Expr::Ident(name) = &tuple.0
					&& !self.vars.contains_key(name)
					&& let Ok(t) = self.types.named(name, tuple.1)
				{
					// trait consts
					if let Some(c) = self.types.consts.map.get(&format!("{t}::{field}")).cloned() {
						let declared = (self.types.traits.iter())
							.filter(|(tn, _)| self.claims(&t, tn))
							.find_map(|(_, tr)| Some((tr.2.iter().find(|f| f.name == *field)?, tr.4)))
							.and_then(|(f, home)| self.types.with_scope(home).resolve(&f.typ, f.span).ok());
						return self.check_expr(&c, &declared.unwrap_or(t));
					}

					if field == "count"
						&& let Some(n) = self.types.assoc_int(name, field)
					{
						return Ok((self.b.ins().iconst(self.int, n), Typ::Int(64)));
					}

					if let Some(v) = self.numeric_bound(&t, field) {
						return Ok((v, t));
					}

					if field == "size"
						&& let Some((n, _)) = t.c_size_align(&self.types)
					{
						return Ok((self.b.ins().iconst(self.int, n as i64), Typ::USize));
					}
				}

				// enum variants
				if let Expr::Ident(name) = &tuple.0
					&& !self.vars.contains_key(name)
					&& self.types.enums.borrow().contains_key(self.qualify(name).as_ref())
				{
					let name = self.qualify(name).to_string();
					return self.construct_variant(&name, field, &[], expr.1);
				}
				if let Some(instance) = self.enum_instance(tuple) {
					return self.construct_variant(&instance, field, &[], expr.1);
				}
				if let Some((n, d)) = self.generic_variant(tuple, field) {
					return self.infer_variant(&n, &d, field, &[], hint, expr.1);
				}

				let (ptr, typ) = self.expr(tuple)?;

				// a refined variant reads its payload
				if let Some(fields) = self.variant_fields(&typ) {
					let idx = tuple_index(&fields, field, expr.1)?;
					return Ok((
						self.ld_typ(ptr, 8 + idx as i32 * 8, &fields[idx].1),
						fields[idx].1.clone(),
					));
				}
				let (ptr, typ) = self.deref(ptr, &typ);

				// expose fields
				if typ == Typ::Ast {
					let ret = match field.as_str() {
						"name" | "typ" | "kind" | "len" => Some(Typ::Ast),
						"items" | "notes" | "fills" => Some(Typ::Array(Box::new(Typ::Ast))),
						_ => None,
					};
					if let Some(ret) = ret {
						let val = self.ast_method(ptr, field, None);
						if let Typ::Array(_) = ret {
							self.temp(val, &ret);
						}
						return Ok((val, ret));
					}
				}

				if let Typ::Trait(tn) = &typ {
					return self.trait_field(ptr, tn, field, expr.1);
				}

				if let Some((key, _)) = typ.keyed()
					&& field == "keys"
				{
					let discs = self
						.variants_of(&key)
						.iter()
						.map(|v| self.b.ins().iconst(self.int, v.disc))
						.collect();
					let (data, len) = self.heap_alloc(discs, &key);
					let typ = Typ::Array(Box::new(key));
					return Ok((self.make_array(data, len, &typ), typ));
				}

				// expose array/string fields
				if let Typ::Array(_) | Typ::FixedArray(..) | Typ::Str = &typ {
					let elem = array_elem(&typ).clone();
					let (data, _) = self.array_parts(ptr, &typ);
					if field == "ptr" {
						let typ = self.types.resolve(&TypeExpr::Name(role::PTR.into()), expr.1)?;
						return Ok((data, typ));
					}
					return match field.parse::<i64>() {
						Ok(n) => {
							let idx = self.b.ins().iconst(self.int, n);
							Ok((self.load_index((ptr, &typ), idx, expr.1), elem))
						}
						Err(_) => Err(Diagnostic::new(
							format!("{typ} has no field `{field}`"),
							expr.1.into_range(),
						)),
					};
				}

				// expose map fields
				if let Typ::Map(k, v) = &typ
					&& (field == "keys" || field == "values")
				{
					let elem = if field == "keys" { k } else { v };
					let typ = Typ::Array(elem.clone());
					let header = self.map_entries(ptr, field == "keys", elem);
					let header = self.owning(header, elem);
					self.temp(header, &typ);
					return Ok((header, typ));
				}

				if let Typ::Struct(sname, sfields) = &typ {
					self.check_member(sname, field, expr.1)?;
					// promote fields from embedded structs if applicable
					if !sfields.iter().any(|f| f.name == *field)
						&& let Some((path, inner, ftyp)) = self.promoted(sfields, field, expr.1)?
					{
						let embed = self.follow(ptr, &path);
						let v = self.ld_typ(embed, (inner * 8) as i32, &ftyp);
						return Ok((v, ftyp));
					}
					// a trait const or default settles fields that aren't stored
					if !sfields.iter().any(|f| f.name == *field)
						&& let Some(c) = self.types.consts.map.get(&format!("{sname}::{field}")).cloned()
					{
						return self.check_expr(&c, &typ);
					}
					// an uncalled method is a closure over its receiver
					if !sfields.iter().any(|f| f.name == *field)
						&& let Some(sig) = self.funcs.get(&format!("{sname}.{field}")).cloned()
						&& sig.params.first().is_some_and(|p| p.typ == typ)
					{
						let (span, rest) = (expr.1, sig.value_params()[1..].to_vec());
						let recv = format!("$recv{}", self.vars.len());
						self.hidden_local(recv.clone(), ptr, typ.clone());
						let args = (0..rest.len()).map(|i| (Expr::Ident(format!("${i}")), span)).collect();
						let body = [(
							Expr::MethodCall {
								recv: Box::new((Expr::Ident(recv), span)),
								method: field.clone(),
								type_args: vec![],
								args,
							},
							span,
						)];
						let fsig = AnonSig::Inferred(Typ::Fn(rest, Box::new(sig.ret)));
						return self.declare_anon_fn(&None, &[], false, fsig, &body, span);
					}
				}

				// a newtype has no heap block
				let transparent = typ.newtype().is_some();
				// structs are just fully-named tuples at the codegen level
				let typ = match typ {
					Typ::Struct(_, fields) => Typ::Tuple(fields.into_iter().map(|f| (Some(f.name), f.typ)).collect()),
					Typ::TupleStruct(_, fields) => Typ::Tuple(fields),
					other => other,
				};

				let fields = match &typ {
					Typ::Tuple(fields) => fields,
					_ => {
						return fail(format!("cannot access a field of {typ}"), tuple.1, "not a tuple");
					}
				};
				let idx = tuple_index(fields, field, expr.1)?;
				let field_typ = fields[idx].1.clone();
				if transparent {
					return Ok((ptr, field_typ));
				}
				let v = self.ld_typ(ptr, (idx * 8) as i32, &field_typ);
				Ok((v, field_typ))
			}

			Expr::Array(elems) => match self.through_sum(hint, |t| matches!(t, Typ::Array(_))).as_ref() {
				Some(Typ::Array(elem)) => self.array_lit(elems, Some(elem), expr.1),
				Some(t @ Typ::Map(..)) if elems.is_empty() => self.map_lit(&[], expr.1, Some(t)),
				_ => self.array_lit(elems, None, expr.1),
			},

			Expr::DotArray(None, elems) => match hint {
				Some(Typ::Array(elem)) => self.array_lit(elems, Some(elem), expr.1),
				Some(t @ Typ::FixedArray(..)) if elems.is_empty() => Ok((self.zero_or_err(t, expr.1)?, t.clone())),
				Some(Typ::FixedArray(elem, n)) => self.fixed_lit(elems, Some((elem, *n)), expr.1),
				Some(t @ Typ::Map(..)) if elems.is_empty() => self.map_lit(&[], expr.1, Some(t)),
				Some(t) if elems.is_empty() && t.keyed().is_some() => self.map_lit(&[], expr.1, Some(t)),
				Some(t) if let Some((_, Typ::FixedArray(elem, n))) = t.keyed() => {
					Ok((self.fixed_lit(elems, Some((elem, *n)), expr.1)?.0, t.clone()))
				}
				// built through `From[[]T]` fills
				Some(t)
					if let Some(sig) = self.pick_from(t, |p| matches!(p, Typ::Array(_)), expr.1)?
						&& let Typ::Array(elem) = access_peel(&sig.params[0].typ) =>
				{
					let (v, _) = self.array_lit(elems, Some(elem), expr.1)?;
					Ok((self.emit_call(&sig, &[v]).0, t.clone()))
				}
				_ => self.fixed_lit(elems, None, expr.1),
			},

			Expr::DotTuple(args) => match hint {
				Some(t) => self.cast_to(t, args, expr.1),
				None => fail(
					"cannot infer the type of `.()` here",
					expr.1,
					"annotate the binding, or cast with `T.( ... )`",
				),
			},

			Expr::DotArray(Some((te, span)), elems) => {
				let typ = self.types.resolve(te, *span)?;
				match &typ {
					Typ::Array(elem) => self.array_lit(elems, Some(elem), expr.1),
					Typ::FixedArray(elem, n) => self.fixed_lit(elems, Some((elem, *n)), expr.1),
					Typ::Map(..) if elems.is_empty() => self.map_lit(&[], expr.1, Some(&typ)),
					_ if elems.is_empty() => fail(
						"an exact array literal needs elements",
						expr.1,
						format!("write `[]{typ}.[]` for an empty dynamic array"),
					),
					_ => self.fixed_lit(elems, Some((&typ, elems.len())), expr.1),
				}
			}

			Expr::Index { collection, index } => {
				let (ptr, typ) = self.expr(collection)?;
				let (ptr, typ) = self.deref(ptr, &typ);
				match &typ {
					Typ::Map(k, v) => {
						let (k, v) = (*k.clone(), *v.clone());
						let (tag, bits) = self.map_key(index, &k)?;
						let raw = self.map_rt("get", ptr, tag, bits, &[]);
						Ok((self.unmap_bits(raw, &v), v))
					}
					t if let Some((key, arr)) = t.keyed() => {
						let idx = self.key_slot(index, &key)?;
						Ok((self.load_index((ptr, arr), idx, collection.1), array_elem(arr).clone()))
					}
					Typ::Array(_) | Typ::FixedArray(..) | Typ::Str => {
						let (idx, ityp) = self.expr(index)?;
						if is_range(&ityp) {
							return self.range_slice((ptr, typ), idx, collection.1);
						}
						let Typ::Int(_) = ityp else {
							return fail(format!("index must be Int, got {ityp}"), index.1, "not an Int");
						};
						let elem = array_elem(&typ).clone();
						let idx = self.intcast(idx, self.int, true);
						Ok((self.load_index((ptr, &typ), idx, collection.1), elem))
					}
					t if self.claims(t, role::INDEX) => self.index_call((ptr, typ), index, expr.1),
					_ => fail(
						format!("cannot index {typ}"),
						collection.1,
						format!("implement `{}` for `{typ}` to index it", role::INDEX),
					),
				}
			}

			Expr::Slice { collection, range } => {
				let (ptr, typ) = self.expr(collection)?;
				let (ptr, typ) = self.deref(ptr, &typ);
				let range = range.as_deref();
				if let Some(r) = range
					&& self.claims(&typ, role::INDEX)
				{
					return self.index_call((ptr, typ), r, expr.1);
				}
				if typ == Typ::Str {
					let len = self.array_len(ptr);
					let (lo, hi) = self.slice_bounds(range, len)?;
					return Ok((self.rt_call("str_slice", &[ptr, lo, hi]).unwrap(), Typ::Str));
				}
				let (out, _, elem) = self.slice_copy((ptr, typ), collection.1, range)?;
				let typ = Typ::Array(Box::new(elem));
				self.temp(out, &typ);
				Ok((out, typ))
			}

			Expr::If { .. } | Expr::Match { .. } | Expr::Loop { .. } => match self.branching(expr, hint, true)? {
				Some(vt) => Ok(vt),
				None => {
					let (kw, why) = match &expr.0 {
						Expr::If { .. } => ("if", "every branch returns, but a value is needed here"),
						Expr::Match { .. } => ("match", "every arm returns, but a value is needed here"),
						_ => ("loop", "an infinite loop with no `break` yields nothing"),
					};
					fail(format!("this `{kw}` never produces a value"), expr.1, why)
				}
			},
			Expr::Labeled { .. } => match self.branching(expr, hint, true)? {
				Some(vt) => Ok(vt),
				None => fail("this block never produces a value", expr.1, "every path returns"),
			},

			Expr::Pipe { value, step } => self.pipe(value, step, expr.1),

			Expr::OrElse { value, body } => self.or_else(value, body, expr.1, true),
			Expr::AndThen { value, body } => self.and_then(value, body, expr.1),
			Expr::Propagate(value) => self.propagate(value, expr.1),

			Expr::For { label, pat, iter, body } => {
				self.collected(true, hint, |s| s.for_loop(label.as_deref(), pat, iter, body))
			}

			Expr::Block(body) => match hint {
				// bare blocks are treated as fn literals when they match an expected/inferred fn type
				Some(t @ Typ::Fn(..)) => {
					self.declare_anon_fn(&None, &[], true, AnonSig::Inferred(t.clone()), body, expr.1)
				}
				Some(Typ::Annotated(anns, t)) if is_isolated(anns) && matches!(**t, Typ::Fn(..)) => {
					self.isolated = true;
					self.lower(expr, Some(t))
				}
				_ => self.block_expr(body, expr.1),
			},

			Expr::StructLit {
				name,
				type_args,
				fields,
			} => self.struct_lit(name, type_args, fields, expr.1, hint),

			Expr::Ref(inner) => {
				let span = expr.1.into_range();
				match &inner.0 {
					Expr::Field { .. } | Expr::Index { .. } => {
						return Err(Diagnostic::new("cannot take the address of a field or element", span));
					}
					Expr::Ident(n) if let Some(local) = self.vars.get(n).cloned() => {
						if !self.aliases.contains(&local.var) {
							return Err(Diagnostic::new(format!("cannot take the address of `{n}`"), span)
								.with_label("not a mutable local"));
						}
						let typ = Typ::Ref(Box::new(local.typ));
						let handle = self.b.use_var(local.var);
						let handle = self.copy_in(handle, &typ);
						self.temp(handle, &typ);
						return Ok((handle, typ));
					}
					_ => {}
				}
				let pointee = hint.and_then(|t| self.pointee(t, expr.1).ok());
				let (ptr, typ) = self.lower(inner, pointee.as_ref())?;
				// move the literal's slots into a shared box
				let ptr = match inner.0 {
					Expr::StructLit { .. } => ptr,
					_ => {
						self.move_resource(inner, &typ)?;
						self.copy_bind(ptr, &typ)
					}
				};
				let boxp = self.box_value(ptr, &typ);
				self.untemp(ptr);
				let typ = Typ::Ref(Box::new(typ.clone()));
				self.temp(boxp, &typ);
				Ok((boxp, typ))
			}

			Expr::Deref(inner) => {
				let (val, typ) = self.expr(inner)?;
				self.pointee(&typ, inner.1)?;
				Ok(self.deref(val, &typ))
			}

			Expr::Record(entries) => match hint {
				Some(Typ::Struct(name, _)) => {
					let fields = entries
						.iter()
						.map(|(k, v)| match &k.0 {
							Expr::Ident(n) => Ok((Some(n.clone()), v.clone())),
							_ => fail(format!("`{name}` fields are named by idents"), k.1, "not a field name"),
						})
						.collect::<Result<Vec<_>, _>>()?;
					self.struct_lit(name, &[], &fields, expr.1, hint)
				}
				_ => self.record_lit(entries, expr.1, hint),
			},

			Expr::Map(entries) => self.map_lit(
				entries,
				expr.1,
				self.through_sum(hint, |t| matches!(t, Typ::Map(..))).as_ref(),
			),

			Expr::Spread(inner) => {
				let (val, typ) = self.expr(inner)?;
				let Typ::Int(_) = typ else {
					return fail(
						format!("cannot spread {typ} here"),
						expr.1,
						"`..` spreads only inside a literal or a call",
					);
				};
				self.upto(val, expr.1)
			}

			Expr::Range { start, end, inclusive } => self.range_value(start, end.as_deref(), *inclusive, expr.1),

			Expr::AnonFn {
				captures,
				params,
				params_tuple,
				ret,
				body,
			} => {
				let hint = match hint {
					// `@ctx` pins the literal's ctx
					Some(Typ::Annotated(anns, t)) if let Some(c) = ctx_mark(anns) => {
						self.anon_ctx.get_or_insert_with(|| c.into());
						Some(&**t)
					}
					// `@isolated` isolates a literal
					Some(Typ::Annotated(anns, t)) if is_isolated(anns) => {
						self.isolated = true;
						return self.lower(expr, Some(t));
					}
					h => h,
				};
				let sig = match (ret, hint) {
					(Some(ret), _) => AnonSig::Explicit(ret),
					(None, Some(t @ Typ::Fn(..))) => AnonSig::Inferred(t.clone()),
					(None, _) => {
						return fail(
							"anonymous functions need an explicit return type",
							expr.1,
							"add a return type, e.g. `fn [] () int { ... }`",
						);
					}
				};
				self.declare_anon_fn(captures, params, *params_tuple, sig, body, expr.1)
			}

			Expr::Annotated(anns, inner) => {
				let names = ann_names(self.types.scope, anns);
				if matches!(inner.0, Expr::AnonFn { .. }) && is_pure(&names) {
					let was = std::mem::replace(&mut self.pure, true);
					let out = self.expr(inner);
					self.pure = was;
					return out;
				}
				if matches!(inner.0, Expr::AnonFn { .. }) && is_isolated(&names) {
					self.isolated = true;
					return self.lower(inner, hint);
				}
				if matches!(inner.0, Expr::AnonFn { .. })
					&& let Some(t) = ctx_mark(&names)
				{
					self.anon_ctx = Some(t.into());
					return self.lower(inner, hint);
				}
				let (val, typ) = self.expr(inner)?;
				check_ann_typ(self.types, &names, &typ, expr.1)?;
				let addr = self.ld_word(val, 0);
				Ok((addr, Typ::Annotated(names, Box::new(typ))))
			}

			Expr::Bind { .. }
			| Expr::Assign { .. }
			| Expr::PatBind { .. }
			| Expr::IndexAssign { .. }
			| Expr::FieldAssign { .. }
			| Expr::DerefAssign { .. }
			| Expr::Append { .. }
			| Expr::MapDelete { .. } => {
				// a place in expression position is a one-statement block
				Ok(self
					.block_tail(std::slice::from_ref(expr), hint)?
					.expect("a place never diverges"))
			}
			Expr::Fn { .. }
			| Expr::StructDef { .. }
			| Expr::EnumDef { .. }
			| Expr::TraitDef { .. }
			| Expr::TypeAlias { .. } => Err(Diagnostic::new(
				"definitions are only allowed at the top level",
				expr.1.into_range(),
			)),
			Expr::Claim { .. } => unreachable!("claim in expression position"),
			Expr::TypePat(te) => Ok((self.type_val(te, expr.1)?, Typ::TypeId)),
			Expr::Return(_) | Expr::Break(..) | Expr::Continue(_) => fail(
				"`return`, `break`, and `continue` never produce a value",
				expr.1,
				"this diverges",
			),

			Expr::Fallthrough => fail(
				"`fallthrough` must be the last statement of a match arm",
				expr.1,
				"not at the end of an arm",
			),

			Expr::Defer { label, body, when } => Ok(self.defer(label, body, *when, false)),
			Expr::Cancel(label) => self.cancel(label, expr.1),

			Expr::Discard(e) => {
				let (v, t) = self.expr(e)?;
				if self.is_linear(&t) {
					if self.self_type.as_deref() != t.nominal() {
						return fail(
							format!("only `{t}`'s own methods can discard it"),
							expr.1,
							"still linear",
						);
					}
					self.move_resource(e, &t)?;
					self.untemp(v);
					self.release_value(v, &t);
				}
				Ok(self.unit_value())
			}

			Expr::Doc(_) | Expr::Module(_) | Expr::Use { .. } | Expr::Pub(..) => {
				unreachable!("not an expression")
			}

			Expr::MacroDef { .. } => unreachable!("removed by macro expansion"),
			Expr::Quote(stmts) => self.quote(stmts, expr.1),
			Expr::Arm(_) => Err(Diagnostic::new("a match arm only fits in a match", expr.1.into_range())),
			Expr::Unquote(_) | Expr::UnquoteExpr(_) | Expr::UnquoteSplat(_) | Expr::UnquoteBind(..) => {
				fail("unquotes only make sense inside a quote", expr.1, "stray unquote")
			}

			Expr::Comp(_) => fail("`comp` isn't supported here", expr.1, "can't run at compile time"),

			Expr::With(subjects) => self.open_with(subjects),

			Expr::Unsafe(inner) => {
				self.unsafely += 1;
				let v = self.expr(inner);
				self.unsafely -= 1;
				v
			}
		}
	}

	// Get arch bounds of numeric types.
	fn numeric_bound(&mut self, t: &Typ, field: &str) -> Option<Value> {
		let hi = match field {
			"max" => true,
			"min" => false,
			_ => return None,
		};
		if let Some((min, max)) = t.int_bounds() {
			return Some(self.int_const(cl_type(t, self.int), if hi { max } else { min }));
		}
		let ins = self.b.ins();
		Some(match t {
			Typ::Float(64) => ins.f64const(if hi { f64::MAX } else { f64::MIN }),
			Typ::Float(32) => ins.f32const(if hi { f32::MAX } else { f32::MIN }),
			Typ::Float(16) => ins.f16const(Ieee16::with_bits(if hi { 0x7bff } else { 0xfbff })),
			_ => return None,
		})
	}
}

// The name a type's methods are keyed under.
fn method_owner(typ: &Typ) -> Option<String> {
	Some(match typ {
		Typ::Struct(name, _) | Typ::TupleStruct(name, _) | Typ::Enum(name) | Typ::Sum(name, _) => name.clone(),
		Typ::Array(_) => "array".into(),
		Typ::Map(..) => "map".into(),
		Typ::Int(_) | Typ::UInt(_) | Typ::Float(_) | Typ::Bool | Typ::ISize | Typ::USize => typ.to_string(),
		Typ::Rune | Typ::Str | Typ::CStr | Typ::Ast | Typ::Atom | Typ::Tuple(_) => typ.to_string(),
		_ => return None,
	})
}
