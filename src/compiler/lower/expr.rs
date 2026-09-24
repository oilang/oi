use crate::compiler::role;

use super::*;

impl<'a, M: Module> Translator<'a, M> {
	pub fn expr(&mut self, expr: &Spanned<Expr>) -> Result<TypedVal, Diagnostic> {
		self.lower(expr, None)
	}

	// Get the array name for an append operation, if any.
	fn append_target(&self, l: &Spanned<Expr>) -> Option<String> {
		match &l.0 {
			Expr::Ident(n) => matches!(self.vars.get(n)?.typ, Typ::Array(_)).then(|| n.clone()),
			Expr::Binary(BinOp::Shl, l, _) => self.append_target(l),
			_ => None,
		}
	}

	pub(super) fn lower(&mut self, expr: &Spanned<Expr>, hint: Option<&Typ>) -> Result<TypedVal, Diagnostic> {
		if let Some(t) = hint
			&& let Some(v) = self.coerce_lit(expr, t)?
		{
			return Ok((v, t.clone()));
		}
		match &expr.0 {
			Expr::Int(n) => Ok((self.b.ins().iconst(types::I64, *n), Typ::Int(64))),
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

			Expr::ArgMod(a, _) => Err(Diagnostic::new(
				format!("`{a}` is only allowed on call arguments"),
				expr.1.into_range(),
			)
			.with_label("not a call argument")),

			Expr::Foreign => Err(Diagnostic::new(
				"foreign is only allowed as a module-level binding",
				expr.1.into_range(),
			)
			.with_label("not a module-level binding")),

			Expr::Ident(name) => match self.local(name, expr.1.into_range()) {
				Ok(local) => {
					let val = self.read_local(&local);
					Ok((val, local.typ))
				}
				Err(e) => match self.funcs.get(self.qualify(name).as_ref()).cloned() {
					Some(sig) => {
						if sig.unsafe_call {
							self.require_unsafe(name, expr.1)?;
						}
						let obj = self.fn_object(sig.id);
						Ok((obj, Typ::Fn(sig.value_params(), Box::new(sig.ret))))
					}
					None => {
						let key = self.qualify(name);
						let visible = key.contains("::") || !self.is_main;
						match visible.then(|| self.types.consts.map.get(key.as_ref()).cloned()).flatten() {
							Some(c) => self.expr(&c),
							None => Err(e),
						}
					}
				},
			},

			Expr::Dollar => Ok(self.dollar()),

			Expr::Is {
				subject,
				trait_name,
				negated,
			} => {
				let Expr::Ident(name) = &subject.0 else {
					return Err(Diagnostic::new(
						"`is` takes a type name on the left",
						subject.1.into_range(),
					));
				};
				let typ = self.types().resolve(&TypeExpr::Name(name.clone()), subject.1)?;
				let tn = self.types.scope.env.get(trait_name).unwrap_or(trait_name);
				let holds = self.claims(&typ, tn) ^ negated;
				Ok((self.b.ins().iconst(self.int, holds as i64), Typ::Bool))
			}

			Expr::Negative(e) => {
				let (v, typ) = self.expr(e)?;
				let out = match typ {
					Typ::Int(_) => self.b.ins().ineg(v),
					Typ::Float(_) => self.b.ins().fneg(v),
					Typ::Struct(ref name, _) | Typ::Enum(ref name) => match self.fill(name, role::NEG, "neg", 1) {
						Some(sig) => return Ok(self.emit_call(&sig, &[v])),
						None => {
							return Err(Diagnostic::new(format!("cannot negate {typ}"), expr.1.into_range())
								.with_label(format!("claim `Neg` for `{name}`")));
						}
					},
					_ => {
						return Err(Diagnostic::new(format!("cannot negate {typ}"), expr.1.into_range())
							.with_label(format!("this is {typ}")));
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
					Some(name) => {
						self.expr(l)?;
						let value = r.clone();
						self.lower(&(Expr::Append { name, value }, expr.1), hint)
					}
					None => self.binop(*op, l, r, expr.1),
				},
				_ => self.binop(*op, l, r, expr.1),
			},
			Expr::Not(e) => {
				let (v, typ) = self.expr(e)?;
				let out = match &typ {
					Typ::Bool => self.b.ins().bxor_imm(v, 1),
					Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize => {
						let v = self.b.ins().bnot(v);
						self.narrow(v, &typ)
					}
					Typ::Struct(name, _) | Typ::Enum(name) => match self.fill(name, role::NOT, "not", 1) {
						Some(sig) => return Ok(self.emit_call(&sig, &[v])),
						None => {
							return Err(
								Diagnostic::new(format!("cannot apply `!` to {typ}"), expr.1.into_range())
									.with_label(format!("claim `Not` for `{name}`")),
							);
						}
					},
					_ => {
						return Err(Diagnostic::new(
							format!("expected Bool or an integer, got {typ}"),
							expr.1.into_range(),
						)
						.with_label("`!` needs a Bool or integer operand"));
					}
				};
				Ok((out, typ))
			}

			Expr::Call { name, type_args, args } => {
				let qn = self.qualify(name).to_string();
				self.check_type_args(name, &qn, type_args, expr.1)?;
				if let Some(local) = self.vars.get(name).cloned() {
					let callee = self.read_local(&local);
					return self.call_value(name, Callee::Object(callee), &local.typ, args, None, expr.1);
				}
				match self.builtin_call(name, args, expr.1)? {
					Some(result) => Ok(result),
					None => match self.funcs.get(&qn).cloned() {
						Some(sig) => self.call_sig(name, sig, None, None, args, expr.1),
						None => match self.generic_fns.get(&qn).cloned() {
							Some(def) => self.call_generic(&qn, &def, type_args, args, None, expr.1),
							None if matches!(self.types.aliases.get(&qn), Some(TypeExpr::TupleStruct(..))) => {
								self.construct_tuple_struct(&qn, args, expr.1)
							}
							None if matches!(
								self.types.aliases.get(&qn),
								Some(TypeExpr::Fn(..) | TypeExpr::Annotated(..))
							) =>
							{
								self.cast_fn_ptr(&qn, args, expr.1)
							}
							None if matches!(name.as_str(), "assert" | "panic") => {
								Err(Diagnostic::new(format!("`{name}` is a macro"), expr.1.into_range())
									.with_label(format!("write `{name}!(...)`")))
							}
							None => Err(
								Diagnostic::new(format!("undefined function `{name}`"), expr.1.into_range())
									.with_label("not defined"),
							),
						},
					},
				}
			}

			Expr::Cast { target, args } => {
				if let TypeExpr::Name(path) = &target.0
					&& let Some((name, variant)) = self.variant_path(path, target.1)
				{
					return self.construct_variant(&name, &variant, args, expr.1);
				}
				let typ = self.types().resolve(&target.0, target.1)?;
				self.cast_to(&typ, args, expr.1)
			}

			Expr::Apply { callee, args } => {
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
				if let Expr::Ident(m) = &recv.0
					&& !self.vars.contains_key(m)
					&& let Some(vis) = self.types.scope.visible.get(m)
				{
					// narrowed imports
					let target = match &vis.only {
						None => method,
						Some(only) => only.get(method).ok_or_else(|| {
							Diagnostic::new(format!("`{method}` is not part of `{m}`"), expr.1.into_range())
								.with_label("not in this import")
						})?,
					};
					let (module, target) = (vis.module.clone(), target.clone());
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
						return Err(Diagnostic::new(msg, expr.1.into_range())
							.with_label(format!("write `{name}.{method}.( … )` or `.{{ … }}`")));
					}
				}

				// generic enum variants
				if let Some(instance) = self.enum_instance(recv)
					&& !self.has_fill(&instance, method)
				{
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
					&& let Some(typ) = self.types().named(name, recv.1).ok().filter(|t| {
						matches!(
							t,
							Typ::Int(_)
								| Typ::UInt(_) | Typ::Float(_)
								| Typ::Bool | Typ::ISize | Typ::USize
								| Typ::Str | Typ::Struct(..)
								| Typ::TupleStruct(..) | Typ::Enum(_)
								| Typ::Array(_) | Typ::Map(..)
						)
					}) {
					match &typ {
						Typ::Enum(n) => (n.clone(), None),
						Typ::Array(_) => ("array".into(), None),
						Typ::Map(..) => ("map".into(), None),
						_ => (typ.to_string(), None),
					}
				} else {
					let (recv_val, recv_typ) = self.expr(recv)?;
					let recv_typ = self.peeled(&recv_typ);
					if recv_typ == Typ::Ast && method == "int" && args.is_empty() {
						let raw = self.ast_method(recv_val, method, None);
						return Ok((self.intcast(raw, types::I64, true), Typ::Int(64)));
					}
					let has_str_impl = matches!(
						recv_typ,
						Typ::Struct(..) | Typ::TupleStruct(..) | Typ::Enum(..) | Typ::Sum(..) | Typ::CStr
					);
					if method == "str" && args.is_empty() && !has_str_impl {
						return Ok((self.derived_str(recv_val, &recv_typ), Typ::Str));
					}
					if let Typ::Trait(tn) = &recv_typ {
						return self.dyn_call(recv_val, tn, method, args, expr.1);
					}

					// `Error` trait
					if recv_typ == Typ::Error {
						return self.dyn_call(recv_val, role::ERROR, method, args, expr.1);
					}
					match &recv_typ {
						Typ::Struct(name, _) | Typ::TupleStruct(name, _) | Typ::Enum(name) | Typ::Sum(name, _) => {
							(name.clone(), Some((recv_val, recv_typ)))
						}
						Typ::Str | Typ::CStr => (recv_typ.to_string(), Some((recv_val, recv_typ))),
						Typ::Int(_) | Typ::UInt(_) | Typ::Float(_) | Typ::Bool | Typ::ISize | Typ::USize => {
							(recv_typ.to_string(), Some((recv_val, recv_typ)))
						}
						Typ::Array(_) => ("array".into(), Some((recv_val, recv_typ))),
						Typ::Map(..) => ("map".into(), Some((recv_val, recv_typ))),
						_ => {
							return Err(
								Diagnostic::new(format!("`{recv_typ}` has no methods"), recv.1.into_range())
									.with_label("methods are only defined on structs and primitives"),
							);
						}
					}
				};
				if sname == role::PTR {
					let recv = bound.as_ref().map(|(v, _)| *v);
					match method.as_str() {
						"array" => return self.ptr_array(recv, type_args, args, expr.1),
						m @ ("read" | "write") => return self.ptr_copy(m == "read", recv, type_args, args, expr.1),
						_ => {}
					}
				}
				let recv_expr = bound.is_some().then_some(recv);
				self.check_member(&sname, method, expr.1)?;
				let key = format!("{sname}.{method}");
				let gkey = format!("{}.{method}", rc::base_name(&sname));
				self.check_type_args(method, &gkey, type_args, expr.1)?;
				if let Some(sig) = self.funcs.get(&key).cloned() {
					return self.call_sig(&key, sig, bound.map(|(v, _)| v), recv_expr, args, expr.1);
				}
				if let Some(def) = self.generic_fns.get(&gkey).cloned() {
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
				if method == "str"
					&& args.is_empty()
					&& let Some((v, t)) = &bound
				{
					return Ok((self.derived_str(*v, t), Typ::Str));
				}

				// make fn fields callable
				if let Some((recv, Typ::Struct(_, sfields))) = &bound
					&& let Some(i) = sfields.iter().position(|f| f.name == *method)
				{
					let ft = sfields[i].typ.clone();
					let v = self
						.b
						.ins()
						.load(cl_type(&ft, self.int), MemFlags::new(), *recv, (i * 8) as i32);
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
				Err(
					Diagnostic::new(format!("`{sname}` has no method `{method}`"), expr.1.into_range())
						.with_label("no such method"),
				)
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
				let ptr = self.call_alloc(elems.len());
				let mut fields = Vec::with_capacity(elems.len());
				for (i, (name, value)) in elems.iter().enumerate() {
					let (val, typ) = match want {
						Some(fs) => self.check_expr(value, &fs[i].1)?,
						None => self.expr(value)?,
					};
					self.move_resource(value, &typ)?;
					let val = self.copy_in(val, &typ);
					self.b.ins().store(MemFlags::new(), val, ptr, (i * 8) as i32);
					fields.push((name.clone(), typ));
				}
				let typ = Typ::Tuple(fields);
				self.temp(ptr, &typ);
				Ok((ptr, typ))
			}

			Expr::Field { tuple, field } => {
				let dotted = self.dotted_type(tuple);
				let tuple = dotted.as_ref().unwrap_or(tuple);

				// access an imported module's items
				if let Expr::Ident(m) = &tuple.0
					&& !self.vars.contains_key(m)
					&& let Some(vis) = self.types.scope.visible.get(m)
				{
					let target = match &vis.only {
						None => field,
						Some(only) => only.get(field).ok_or_else(|| {
							Diagnostic::new(format!("`{field}` is not part of `{m}`"), expr.1.into_range())
								.with_label("not in this import")
						})?,
					};
					let module = vis.module.clone();
					let key = format!("{module}::{target}");
					let key = self.reexports.get(&key).cloned().unwrap_or(key);
					if let Some(l) = self.vars.get(&key).cloned().filter(|l| l.stat) {
						if !self.publics.contains(&key) {
							let msg = format!("`{field}` is private to module `{module}`");
							return Err(Diagnostic::new(msg, expr.1.into_range()).with_label("not public"));
						}
						self.require_pure(field, expr.1)?;
						let val = self.read_local(&l);
						return Ok((val, l.typ));
					}
					let (msg, label) = match self.types.consts.map.get(&key).cloned() {
						Some(c) if self.publics.contains(&key) => return self.expr(&c),
						Some(_) => (format!("`{field}` is private to module `{module}`"), "not public"),
						None if self.funcs.contains_key(&key) || self.generic_fns.contains_key(&key) => {
							(format!("`{field}` is a function, call it"), "add `()`")
						}
						None => (format!("module `{module}` has no const `{field}`"), "no such const"),
					};
					return Err(Diagnostic::new(msg, expr.1.into_range()).with_label(label));
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

				// associated consts
				if let Expr::Ident(name) = &tuple.0
					&& !self.vars.contains_key(name)
					&& let Ok(t) = self.types().named(name, tuple.1)
				{
					if let Some(c) = self.types.consts.map.get(&format!("{t}::{field}")).cloned() {
						return self.check_expr(&c, &t);
					}
					if let Some(v) = self.numeric_bound(&t, field) {
						return Ok((v, t));
					}
					if field == "size"
						&& let Some((n, _)) = t.c_size_align(&|n: &str| is_c_struct(self.types.consts.anns, n))
					{
						return Ok((self.b.ins().iconst(self.int, n as i64), Typ::USize));
					}
				}

				let (ptr, typ) = self.expr(tuple)?;
				let typ = self.peeled(&typ);

				// expose fields
				if typ == Typ::Ast {
					let ret = match field.as_str() {
						"name" | "typ" => Some(Typ::Ast),
						"items" | "notes" | "fills" => Some(Typ::Array(Box::new(Typ::Ast))),
						_ => None,
					};
					if let Some(ret) = ret {
						return Ok((self.ast_method(ptr, field, None), ret));
					}
				}

				if let Typ::Trait(tn) = &typ {
					return self.trait_field(ptr, tn, field, expr.1);
				}

				// expose array/string fields
				if let Typ::Array(_) | Typ::FixedArray(..) | Typ::Str = &typ {
					let elem = array_elem(&typ).clone();
					let (data, len) = self.array_parts(ptr, &typ);
					if field == "len" {
						return Ok((self.intcast(len, types::I64, true), Typ::Int(64)));
					}
					if field == "ptr" {
						let typ = self.types().resolve(&TypeExpr::Name(role::PTR.into()), expr.1)?;
						return Ok((data, typ));
					}
					return match field.parse::<i64>() {
						Ok(n) => {
							let idx = self.b.ins().iconst(self.int, n);
							Ok((self.load_index(data, len, &elem, idx), elem))
						}
						Err(_) => Err(Diagnostic::new(
							format!("{typ} has no field `{field}`"),
							expr.1.into_range(),
						)),
					};
				}

				// expose map fields
				if let Typ::Map(k, v) = &typ {
					if field == "len" {
						let len = self.rt_call("map_len", &[ptr]).unwrap();
						return Ok((self.intcast(len, types::I64, true), Typ::Int(64)));
					}
					if field == "keys" || field == "values" {
						let elem = if field == "keys" { k } else { v };
						let typ = Typ::Array(elem.clone());
						let header = self.map_entries(ptr, field == "keys", elem);
						self.temp(header, &typ);
						return Ok((header, typ));
					}
				}

				if let Typ::Struct(sname, sfields) = &typ {
					self.check_member(sname, field, expr.1)?;
					// promote fields from embedded structs if applicable
					if !sfields.iter().any(|f| f.name == *field)
						&& let Some((path, inner, ftyp)) = self.promoted(sfields, field, expr.1)?
					{
						let embed = self.follow(ptr, &path);
						let v = self
							.b
							.ins()
							.load(cl_type(&ftyp, self.int), MemFlags::new(), embed, (inner * 8) as i32);
						return Ok((v, ftyp));
					}
					// a trait const or default settles fields that aren't stored
					if !sfields.iter().any(|f| f.name == *field)
						&& let Some(c) = self.types.consts.map.get(&format!("{sname}::{field}")).cloned()
					{
						return self.check_expr(&c, &typ);
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
						return Err(
							Diagnostic::new(format!("cannot access a field of {typ}"), tuple.1.into_range())
								.with_label("not a tuple"),
						);
					}
				};
				let idx = match field.parse::<usize>() {
					Ok(i) if i < fields.len() => i,
					Ok(i) => {
						return Err(Diagnostic::new(
							format!("tuple index {i} out of range (len {})", fields.len()),
							expr.1.into_range(),
						)
						.with_label("no such field"));
					}
					Err(_) => fields
						.iter()
						.position(|(name, _)| name.as_deref() == Some(field.as_str()))
						.ok_or_else(|| {
							Diagnostic::new(format!("tuple has no field `{field}`"), expr.1.into_range())
								.with_label("no such field")
						})?,
				};
				let field_typ = fields[idx].1.clone();
				if transparent {
					return Ok((ptr, field_typ));
				}
				let cl = cl_type(&field_typ, self.int);
				let v = self.b.ins().load(cl, MemFlags::new(), ptr, (idx * 8) as i32);
				Ok((v, field_typ))
			}

			Expr::Array(elems) => match self.through_sum(hint, |t| matches!(t, Typ::Array(_))).as_ref() {
				Some(Typ::Array(elem)) => self.array_lit(elems, Some(elem), expr.1),
				Some(t @ Typ::Map(..)) if elems.is_empty() => self.map_lit(&[], expr.1, Some(t)),
				_ => self.array_lit(elems, None, expr.1),
			},

			Expr::DotArray(None, elems) => match hint {
				Some(Typ::Array(elem)) => self.array_lit(elems, Some(elem), expr.1),
				Some(Typ::FixedArray(elem, n)) => self.fixed_lit(elems, elem, *n, expr.1),
				_ => self.fixed_infer(elems, expr.1),
			},

			Expr::DotTuple(args) => match hint {
				Some(Typ::TupleStruct(name, _)) => self.construct_tuple_struct(name, args, expr.1),
				_ => Err(
					Diagnostic::new("cannot infer the tuple struct here", expr.1.into_range())
						.with_label("annotate the binding, or construct with `Name( ... )`"),
				),
			},

			Expr::DotArray(Some((te, span)), elems) => {
				let typ = self.types().resolve(te, *span)?;
				match &typ {
					Typ::Array(elem) => self.array_lit(elems, Some(elem), expr.1),
					Typ::FixedArray(elem, n) => self.fixed_lit(elems, elem, *n, expr.1),
					_ if elems.is_empty() => Err(Diagnostic::new(
						"an exact array literal needs elements",
						expr.1.into_range(),
					)
					.with_label(format!("write `[]{typ}.[]` for an empty dynamic array"))),
					_ => self.fixed_lit(elems, &typ, elems.len(), expr.1),
				}
			}

			Expr::Index { collection, index } => {
				let (ptr, typ) = self.expr(collection)?;
				match &typ {
					Typ::Map(k, v) => {
						let (k, v) = (*k.clone(), *v.clone());
						let (tag, bits) = self.map_key(index, &k)?;
						let raw = self.call_map_get(ptr, tag, bits);
						Ok((self.unmap_bits(raw, &v), v))
					}
					Typ::Array(_) | Typ::FixedArray(..) | Typ::Str => {
						let (idx, ityp) = self.expr(index)?;
						if is_range(&ityp) {
							return self.range_slice((ptr, typ), idx, collection.1);
						}
						let Typ::Int(_) = ityp else {
							return Err(Diagnostic::new(
								format!("index must be Int, got {ityp}"),
								index.1.into_range(),
							)
							.with_label("not an Int"));
						};
						let elem = array_elem(&typ).clone();
						let idx = self.intcast(idx, self.int, true);
						let (data, len) = self.array_parts(ptr, &typ);
						Ok((self.load_index(data, len, &elem, idx), elem))
					}
					_ => Err(
						Diagnostic::new(format!("cannot index {typ}"), collection.1.into_range())
							.with_label("not indexable"),
					),
				}
			}

			Expr::Slice { collection, range } => {
				let (ptr, typ) = self.expr(collection)?;
				let range = range.as_deref();
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

			Expr::If { .. } | Expr::Match { .. } | Expr::Loop { .. } => match self.branching(expr, hint)? {
				Some(vt) => Ok(vt),
				None => {
					let (kw, why) = match &expr.0 {
						Expr::If { .. } => ("if", "every branch returns, but a value is needed here"),
						Expr::Match { .. } => ("match", "every arm returns, but a value is needed here"),
						_ => ("loop", "an infinite loop with no `break` yields nothing"),
					};
					Err(
						Diagnostic::new(format!("this `{kw}` never produces a value"), expr.1.into_range())
							.with_label(why),
					)
				}
			},

			Expr::Pipe { value, step } => self.pipe(value, step, expr.1),

			Expr::OrElse { value, body } => self.or_else(value, body, expr.1),
			Expr::Propagate(value) => self.propagate(value, expr.1),

			Expr::For { pat, iter, body } => self.for_loop(pat, iter, body),

			Expr::Block(body) => match hint {
				// bare blocks are treated as fn literals when they match an expected/inferred fn type
				Some(t @ Typ::Fn(..)) => {
					self.declare_anon_fn(&None, &[], true, AnonSig::Inferred(t.clone()), body, expr.1)
				}
				_ => self.block_expr(body, expr.1),
			},

			Expr::StructLit {
				name,
				type_args,
				fields,
			} => self.struct_lit(name, type_args, fields, expr.1, hint),

			Expr::Ref(inner) => {
				let (ptr, typ) = self.expr(inner)?;
				let Typ::Struct(name, fields) = &typ else {
					return Err(
						Diagnostic::new("only a struct can be boxed into a reference", inner.1.into_range())
							.with_label(format!("this is {typ}")),
					);
				};
				// move the literal's slots into a shared box
				let n = fields.len();
				let base = self.call_alloc_bytes((n * 8) as i64 + 16);
				let descv = self.trace_desc(name, fields);
				self.b.ins().store(MemFlags::new(), descv, base, 0);
				let one = self.b.ins().iconst(self.int, 1);
				self.b.ins().store(MemFlags::new(), one, base, 8);
				let boxp = self.b.ins().iadd_imm(base, 16);
				for i in 0..n {
					let v = self.b.ins().load(self.int, MemFlags::new(), ptr, (i * 8) as i32);
					self.b.ins().store(MemFlags::new(), v, boxp, (i * 8) as i32);
				}
				let typ = Typ::Ref(Box::new(typ.clone()));
				self.temp(boxp, &typ);
				Ok((boxp, typ))
			}

			Expr::Record(entries) => match hint {
				Some(Typ::Struct(name, _)) => {
					let fields = entries
						.iter()
						.map(|(k, v)| match &k.0 {
							Expr::Ident(n) => Ok((Some(n.clone()), v.clone())),
							_ => Err(
								Diagnostic::new(format!("`{name}` fields are named by idents"), k.1.into_range())
									.with_label("not a field name"),
							),
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
					return Err(
						Diagnostic::new(format!("cannot spread {typ} here"), expr.1.into_range())
							.with_label("`..` spreads only inside a literal or a call"),
					);
				};
				let (zero, one) = (self.b.ins().iconst(types::I64, 0), self.b.ins().iconst(types::I64, 1));
				self.make_range(zero, Some(val), one, expr.1)
			}

			Expr::Range { start, end, inclusive } => self.range_value(start, end.as_deref(), *inclusive, expr.1),

			Expr::AnonFn {
				captures,
				params,
				params_tuple,
				ret,
				body,
			} => {
				let sig = match (ret, hint) {
					(Some(ret), _) => AnonSig::Explicit(ret),
					(None, Some(t @ Typ::Fn(..))) => AnonSig::Inferred(t.clone()),
					(None, _) => {
						return Err(Diagnostic::new(
							"anonymous functions need an explicit return type",
							expr.1.into_range(),
						)
						.with_label("add a return type, e.g. `fn [] () int { ... }`"));
					}
				};
				self.declare_anon_fn(captures, params, *params_tuple, sig, body, expr.1)
			}

			Expr::Annotated(anns, inner) => {
				let (names, (val, typ)) = (ann_names(self.types.scope, anns), self.expr(inner)?);
				check_ann_typ(&names, &typ, expr.1)?;
				let addr = self.b.ins().load(self.int, MemFlags::new(), val, 0);
				Ok((addr, Typ::Annotated(names, Box::new(typ))))
			}

			Expr::Bind { .. }
			| Expr::Assign { .. }
			| Expr::PatBind { .. }
			| Expr::IndexAssign { .. }
			| Expr::FieldAssign { .. }
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
			Expr::TypePat(te) => Err(Diagnostic::new(
				format!("`{}` is a type, not a value", self.types().resolve(te, expr.1)?),
				expr.1.into_range(),
			)),
			Expr::Return(..) => unreachable!("return in expression position"),
			Expr::Break(_) | Expr::Continue => Err(Diagnostic::new(
				"`break` and `continue` never produce a value",
				expr.1.into_range(),
			)
			.with_label("this diverges")),
			Expr::Defer { .. } => Err(
				Diagnostic::new("`defer` is only allowed as a statement", expr.1.into_range())
					.with_label("not a value"),
			),
			Expr::Doc(_) | Expr::Module(_) | Expr::Use { .. } | Expr::Pub(_) => {
				unreachable!("not an expression")
			}
			Expr::MacroDef { .. } => unreachable!("removed by macro expansion"),
			Expr::Quote(stmts) => self.quote(stmts, expr.1),
			Expr::Arm(_) => Err(Diagnostic::new("a match arm only fits in a match", expr.1.into_range())),
			Expr::Unquote(_) | Expr::UnquoteExpr(_) | Expr::UnquoteSplat(_) | Expr::UnquoteBind(..) => Err(
				Diagnostic::new("unquotes only make sense inside a quote", expr.1.into_range())
					.with_label("stray unquote"),
			),

			Expr::Comp(_) => Err(Diagnostic::new("`comp` isn't supported here", expr.1.into_range())
				.with_label("can't run at compile time")),

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
		let ins = self.b.ins();
		Some(match t {
			Typ::Int(w) => ins.iconst(cl_int_for_width(*w), if hi { int_max(*w) } else { int_min(*w) }),
			Typ::UInt(w) => ins.iconst(cl_int_for_width(*w), if hi { uint_max(*w) } else { 0 }),
			Typ::ISize => ins.iconst(self.int, if hi { int_max(64) } else { int_min(64) }),
			Typ::USize => ins.iconst(self.int, if hi { uint_max(64) } else { 0 }),
			Typ::Float(64) => ins.f64const(if hi { f64::MAX } else { f64::MIN }),
			Typ::Float(32) => ins.f32const(if hi { f32::MAX } else { f32::MIN }),
			_ => return None,
		})
	}

	pub(super) fn result_init(&mut self, ok_typ: Typ, err_typ: Typ, arg: &Spanned<Expr>) -> Result<TypedVal, Diagnostic> {
		let typ = self.types.core_enum(role::RESULT, &[ok_typ.clone(), err_typ.clone()]);
		let variants = self.variants_of(&typ);
		let (fv, at) = self.check_expr(arg, &ok_typ)?;
		let disc = if at == ok_typ {
			0
		} else if at == err_typ {
			1
		} else {
			return Err(
				Diagnostic::new(format!("expected {ok_typ} or {err_typ}, got {at}"), arg.1.into_range())
					.with_label("type mismatch"),
			);
		};
		let val = self.make_enum(&variants, disc, &[fv]);
		Ok((val, typ))
	}
}
