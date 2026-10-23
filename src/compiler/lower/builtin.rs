use crate::ast::{EnumVariant, Param};
use crate::compiler::{comp, role};

use super::*;

impl<'a, M: Module> Translator<'a, M> {
	// Dispatch a call to a compiler builtin.
	pub(super) fn builtin_call(
		&mut self,
		name: &str,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<Option<TypedVal>, Diagnostic> {
		// user fns shadow builtins
		let qualified = self.qualify(name);
		if self.funcs.contains_key(&*qualified) || self.world.generic_fns.contains_key(&*qualified) {
			return Ok(None);
		}

		match name {
			"error" => {
				if args.len() != 1 {
					return fail(
						format!("`error` takes 1 argument, got {}", args.len()),
						span,
						"wrong number of arguments",
					);
				}
				let ret = self.ret.as_ref().map(|(t, _)| t.clone());
				let err = ret.as_ref().and_then(|t| self.types.result_parts(t)).map(|(_, e)| e);
				let (av, at) = match &err {
					// resolve enum shorthands
					Some(err) if *err != Typ::Error => self.check_expr(&args[0], err)?,
					_ => self.expr(&args[0])?,
				};
				let bare = at == Typ::Str && err.as_ref().is_none_or(|e| *e == Typ::Error);
				match (ret, err) {
					(Some(ret), Some(err)) if at == err => {
						let variants = self.variants_of(&ret);
						let v = self.make_enum(&variants, 1, &[av]);
						Ok(Some((v, ret)))
					}
					// stamp error with call site
					_ if bare => {
						let (src, _) = self.src_lit(span)?;
						let sig = self.funcs[role::RAISE].clone();
						let (v, t) = self.emit_call(&sig, &[av, src]);
						Ok(Some((self.box_error(v, &t), Typ::Error)))
					}
					_ if self.open_error(&at) => Ok(Some((self.box_error(av, &at), Typ::Error))),
					_ => {
						let msg = format!("`{at}` doesn't claim Error, and no enclosing fn returns Result[_, {at}]");
						fail(msg, args[0].1, "not usable as an error")
					}
				}
			}

			"type" => {
				let [x] = args else {
					return fail("`type` takes 1 argument", span, "wrong number of arguments");
				};
				let t = self.type_of(x)?;
				Ok(Some((self.typeid_of(&t), Typ::TypeId)))
			}

			"type_info" => {
				let [(Expr::Ident(name), at)] = args else {
					return fail("`type_info` takes one type name", span, "expected a struct or enum");
				};
				let def = self.type_def(name, *at)?;
				Ok(Some(self.quote(&[(def, *at)], *at)?))
			}

			// an owned copy with no shared buffers
			"__unshare" => {
				let (val, typ) = self.expr(&args[0])?;
				if self.is_affine(&typ) {
					self.move_out(&args[0], &typ)?;
				}
				let val = self.copy_bind(val, &typ);
				let val = self.unshare(val, &typ);
				self.temp(val, &typ);
				Ok(Some((val, typ)))
			}

			// `fmt` and `debug` defaults
			"__render" | "__repr" => {
				let (val, typ) = self.expr(&args[0])?;
				Ok(Some((self.derived_str(val, &typ, name == "__repr"), Typ::Str)))
			}

			// hands a `comp` site's value back to the host, tagged so it can be reified as a literal
			"__comp_yield" => {
				let (val, typ) = self.expr(&args[0])?;
				self.comp_yield(val, &typ, args[0].1)?;
				Ok(Some(self.unit_value()))
			}

			_ => Ok(None),
		}
	}

	// The call site as a `Src`.
	pub(super) fn src_lit(&mut self, span: Span) -> Result<TypedVal, Diagnostic> {
		let (file, line, ..) = self.world.map.locate_span(span.into_range());
		let field = |n: &str, e| (Some(n.to_string()), (e, span));
		let fields = [
			field("file", Expr::String(file.to_string())),
			field("line", Expr::Int(line as i128)),
		];
		self.struct_lit(role::SRC, &[], &fields, span, None)
	}

	// Rebuild a named type's definition as an Ast.
	fn type_def(&self, name: &str, span: Span) -> Result<Expr, Diagnostic> {
		let field = |f: &FieldDef| Param {
			default: f.default.clone(),
			annotations: f.annotations.clone(),
			..Param::new(
				f.name.clone(),
				type_expr(&f.typ).unwrap_or_else(|| TypeExpr::Name(f.typ.to_string())),
				span,
			)
		};
		Ok(match self.peeled(&self.types.named(name, span)?) {
			Typ::Struct(n, fields) => Expr::StructDef {
				name: display_name(&n).into(),
				type_params: vec![],
				fields: fields.iter().map(field).collect(),
				fills: vec![],
			},
			// variants reflect without payloads for now
			Typ::Enum(n) => Expr::EnumDef {
				name: display_name(&n).into(),
				backing: None,
				type_params: vec![],
				variants: (self.enum_variants(&n).iter())
					.map(|v| EnumVariant {
						name: v.name.clone(),
						..Default::default()
					})
					.collect(),
				fills: vec![],
			},
			t => {
				return fail(
					format!("`{t}` has no definition to reflect"),
					span,
					"not a struct or enum",
				);
			}
		})
	}

	// Yield a value to the `comp` host (recursive).
	fn comp_yield(&mut self, val: Value, typ: &Typ, span: Span) -> Result<(), Diagnostic> {
		let (tag, bits) = match typ {
			Typ::Struct(name, fields) => {
				for (i, f) in fields.iter().enumerate() {
					let fv = self.ld_typ(val, (i * 8) as i32, &f.typ);
					self.comp_yield(fv, &f.typ, span)?;
				}
				let name = self.str_const(name);
				let nfields = self.b.ins().iconst(self.int, fields.len() as i64);
				self.extern_call(comp::RT_COMP_STRUCT, &[name, nfields], None);
				return Ok(());
			}
			Typ::Array(elem) | Typ::FixedArray(elem, _) => {
				let (elem, mut err) = ((**elem).clone(), None);
				self.each_elem(val, typ, |s, _, ev| {
					err = err.take().or(s.comp_yield(ev, &elem, span).err())
				});
				err.map_or(Ok(()), Err)?;
				let tag = if elem == Typ::Ast {
					comp::TAG_AST_SEQ
				} else {
					comp::TAG_ARRAY
				};
				(tag, self.array_parts(val, typ).1)
			}
			Typ::Ast => (comp::TAG_AST, val),
			Typ::Bool => (comp::TAG_BOOL, val),
			Typ::Str => (comp::TAG_STR, val),
			Typ::Atom => (comp::TAG_ATOM, val),
			Typ::Int(_) | Typ::ISize | Typ::UInt(_) | Typ::USize => (comp::TAG_INT, self.scalar_bits(val, typ)),
			Typ::Float(32 | 64) => (comp::TAG_FLOAT, self.scalar_bits(val, typ)),
			t if t.is_unit() => (comp::TAG_UNIT, self.b.ins().iconst(self.int, 0)),
			_ => {
				return fail(comp::UNREIFIABLE, span, "not comptime-reifiable");
			}
		};
		let tag_v = self.b.ins().iconst(self.int, tag);
		self.extern_call(comp::RT_COMP_YIELD, &[tag_v, bits], None);
		Ok(())
	}

	// Casts and numeric conversions.
	pub(super) fn cast_to(&mut self, target: &Typ, args: &[Spanned<Expr>], span: Span) -> Result<TypedVal, Diagnostic> {
		let [value] = args else {
			let fields: Vec<_> = args.iter().map(|a| (None, a.clone())).collect();
			return match target {
				Typ::TupleStruct(..) => self.construct_tuple_struct(target.clone(), args, span),
				Typ::Struct(name, _) => self.struct_lit(name, &[], &fields, span, Some(target)),
				Typ::Tuple(_) => self.check_expr(&(Expr::Tuple(fields), span), target),
				_ => fail(
					format!("`{target}` casts a single value"),
					span,
					"wrong number of arguments",
				),
			};
		};
		if *target == Typ::TypeId {
			let (val, typ) = self.lower(value, Some(&Typ::TypeId))?;
			let id = match typ {
				Typ::Any | Typ::TypeId => self.enum_tag(&typ, val),
				t => self.typeid_of(&t),
			};
			return Ok((id, Typ::TypeId));
		}
		if let Some(out) = self.cast_prim(target, value, span)? {
			return Ok(out);
		}
		let hint = match target {
			Typ::TupleStruct(_, fields) if fields.len() == 1 => &fields[0].1,
			t => t,
		};
		let (val, typ) = self.check_expr(value, hint)?;
		if typ == *target {
			return Ok((val, target.clone()));
		}
		if let Some(out) = self.assert_cast(val, &typ, target, span)? {
			return Ok(out);
		}
		if let (Typ::Ref(_), Typ::TupleStruct(p, _)) = (target, &typ)
			&& p == role::PTR
		{
			self.require_unsafe(&format!("{target} cast"), span)?;
			return Ok((
				val,
				Typ::Annotated(vec![role::UNOWNED.into()], Box::new(target.clone())),
			));
		}
		if let (Typ::Struct(name, _), Typ::TupleStruct(p, _)) = (target, &typ)
			&& p == role::PTR
		{
			// a raw address is a place
			self.require_unsafe(&format!("{target} cast"), span)?;
			if is_c_struct(self.types.consts.anns, name) {
				let msg = format!("`{target}` is a `@c` struct, C layout behind a `ptr`");
				return fail(msg, span, "copy it with `p.read[T]()`");
			}
			return Ok((val, target.clone()));
		}
		if let (Typ::CStr, Typ::TupleStruct(p, _)) = (target, &typ)
			&& p == role::PTR
		{
			self.require_unsafe("cstr cast", span)?;
			return Ok((val, Typ::CStr));
		}
		if let (Typ::TupleStruct(p, _), Typ::Fn(params, ret)) = (target, &typ)
			&& p == role::PTR
		{
			check_c_sig(self.types, "@c fn", params, ret, value.1)?;
			return Ok((self.ld_word(val, 0), target.clone()));
		}
		if let (Typ::Array(e), Typ::Str) = (target, &typ)
			&& **e == Typ::UInt(8)
		{
			let (data, len) = self.array_parts(val, &typ);
			let data = self.rt_call("ptr_buffer", &[data, len]).unwrap();
			return Ok((self.make_array(data, len, target), target.clone()));
		}
		if let (Typ::Struct(_, fs), Typ::Tuple(ts)) = (target, &typ)
			&& fs.iter().map(|f| &f.typ).eq(ts.iter().map(|(_, t)| t))
		{
			let ptr = self.stack_slot((fs.len() * 8) as u32);
			self.assign_slots(val, ptr, &target.slots().unwrap_or_default(), false);
			return Ok((ptr, target.clone()));
		}
		if let Typ::TupleStruct(_, fields) = target
			&& let [(_, ft)] = &fields[..]
		{
			let (val, got) = self.coerce(val, &typ, ft, value.1)?;
			if got == *ft {
				return Ok((val, target.clone()));
			}
		}
		// support userland From claims
		if self.claims(target, "core::From")
			&& let Some(sig) = self.find_fill(&format!("{target}.from"), 0, &typ)
		{
			let (out, _) = self.emit_call(&sig, &[val]);
			return Ok((out, target.clone()));
		}
		fail(format!("cannot cast {typ} to {target}"), value.1, "no conversion")
	}

	// Assertion casts.
	fn assert_cast(&mut self, obj: Value, typ: &Typ, target: &Typ, span: Span) -> Result<Option<TypedVal>, Diagnostic> {
		if !matches!(typ, Typ::Any | Typ::Trait(_) | Typ::Error) {
			return Ok(None);
		}
		let opt = self.types.core_enum(role::OPTION, std::slice::from_ref(target));
		let want = if *typ == Typ::Any {
			// `any` is tagged by typeid
			self.b.ins().iconst(self.int, typeid(target))
		} else {
			// trait objects tagged by their vtable
			let tn = if let Typ::Trait(tn) = typ { tn } else { role::ERROR };
			if !self.world.trait_impls.contains(&(target.key(), tn.to_string())) {
				return Ok(Some((self.make_option(&opt, None), opt)));
			}
			self.data_addr(&oi_symbol(&format!("vtable_{}_{tn}", target.key())))
		};

		let got = self.ld_word(obj, 0);
		let same = self.b.ins().icmp(IntCC::Equal, got, want);
		let (yes, no) = self.fork(same);
		let (merge, mut join) = (self.b.create_block(), control::Join::new("cast", span, None));
		self.b.switch_to_block(no);

		let none = self.make_option(&opt, None);
		self.contribute((none, opt.clone()), &mut join, merge)?;
		self.b.switch_to_block(yes);

		let mut data = self.load_bind(obj, typ, target, 8, span);
		if *typ == Typ::Any && self.is_affine(target) {
			let zero = self.b.ins().iconst(self.int, 0);
			self.st(obj, -16, zero);
			data = self.unbox_ret(data, target);
		} else if *typ != Typ::Any && target.slots().is_none() {
			data = self.ld_typ(data, 0, target);
		}

		let some = self.make_option(&opt, Some(data));
		self.contribute((some, opt), &mut join, merge)?;
		Ok(self.finish_merge(merge, join.result))
	}

	// Numeric and string casts.
	fn cast_prim(&mut self, target: &Typ, value: &Spanned<Expr>, span: Span) -> Result<Option<TypedVal>, Diagnostic> {
		use Typ::{Array, Float, ISize, Int, Rune, Str, UInt, USize};
		if !matches!(target, Int(_) | UInt(_) | ISize | USize | Float(_) | Rune | Str) {
			return Ok(None);
		}
		if let Float(w) = target
			&& !matches!(w, 16 | 32 | 64)
		{
			return fail(
				format!("f{w} casts are not yet supported by the JIT backend"),
				span,
				"not yet implemented",
			);
		}
		let (val, typ) = self.expr(value)?;
		if typ == Typ::Any {
			return self.assert_cast(val, &typ, target, span);
		}
		let (val, typ) = self.enum_as_backing(val, typ, value.1)?;
		if *target == Str && !matches!(typ, Str | Array(_)) {
			return fail(
				format!("cannot cast {typ} to {target}"),
				value.1,
				"`.str()` formats a value",
			);
		}
		if typ == *target {
			return Ok(Some((val, typ)));
		}
		let signed = matches!(typ, Int(_) | ISize);
		let out = match (target, &typ) {
			(Str, Array(e)) if **e == UInt(8) => self.rt_call("str_from_bytes", &[val]).unwrap(),
			(Float(_), Float(_) | Int(_) | UInt(_) | ISize | USize) => self.numcast(val, &typ, target),
			(_, Float(_)) => {
				let val = self.widen_f16(val, &typ);
				let to_signed = !matches!(target, UInt(_) | USize);
				let wide = if matches!(target, Int(128) | UInt(128)) {
					let val = self.numcast(val, &typ, &Float(64));
					let signed = self.b.ins().iconst(self.int, to_signed as i64);
					self.rt_call("float_to_wide", &[val, signed]).unwrap()
				} else if to_signed {
					self.b.ins().fcvt_to_sint_sat(self.int, val)
				} else {
					self.b.ins().fcvt_to_uint_sat(self.int, val)
				};
				self.truncate(wide, target, to_signed)
			}
			(_, Int(_) | UInt(_) | ISize | USize | Rune) => self.truncate(val, target, signed),
			(UInt(64), Typ::Struct(n, _)) if n.starts_with(role::BITS) => {
				let words = self.ld_word(val, 0);
				self.ld_word(words, 0)
			}
			_ => {
				let label = if typ == Str {
					format!("`{target}.try_from(...)` parses strings")
				} else {
					"not castable".into()
				};
				return fail(format!("cannot cast {typ} to {target}"), value.1, label);
			}
		};
		Ok(Some((out, target.clone())))
	}

	// Fit an integer into `target`.
	fn truncate(&mut self, val: Value, target: &Typ, signed: bool) -> Value {
		let val = self.intcast(val, cl_type(target, self.int), signed);
		self.narrow(val, target)
	}

	// A fieldless enum casts as its backing value.
	fn enum_as_backing(&mut self, val: Value, typ: Typ, span: Span) -> Result<TypedVal, Diagnostic> {
		if !typ.is_enumish() {
			return Ok((val, typ));
		}
		if matches!(typ, Typ::Sum(..)) {
			return fail(
				"cannot extract a sum member by casting",
				span,
				"no member extraction yet",
			);
		}
		let variants = self.variants_of(&typ);
		if enum_boxed(&variants) {
			return fail(
				format!("`{typ}` has no backing value to cast"),
				span,
				"no backing value",
			);
		}
		let bt = variants.first().and_then(|v| v.backing.clone()).unwrap_or(Typ::Int(64));
		if bt == Typ::Str {
			let raws: Vec<_> = variants.into_iter().map(|v| (v.disc, v.raw.unwrap_or(v.name))).collect();
			let first = self.str_const(&raws[0].1);
			return Ok((self.select_by_disc(val, first, &raws[1..]), Typ::Str));
		}
		let cl = cl_type(&bt, self.int);
		let val = if cl == self.int {
			val
		} else {
			self.b.ins().ireduce(cl, val)
		};
		Ok((val, bt))
	}
}
