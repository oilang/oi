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
		match name {
			"print" | "write" | "eprint" | "ewrite" => {
				self.require_pure(name, span)?;
				let sink = match name {
					"eprint" | "ewrite" => runtime::Sink::Err,
					_ => runtime::Sink::Out,
				};
				let newline = matches!(name, "print" | "eprint");
				if newline && args.is_empty() {
					self.write_lit("\n", sink);
				}
				for (i, arg) in args.iter().enumerate() {
					if i > 0 {
						self.write_lit(" ", sink);
					}
					let (val, typ) = self.expr(arg)?;
					self.emit_print(val, &typ, false, sink);
				}
				if newline && !args.is_empty() {
					self.write_lit("\n", sink);
				}
				Ok(Some(self.unit_value()))
			}

			"error" => {
				if args.len() != 1 {
					return Err(Diagnostic::new(
						format!("`error` takes 1 argument, got {}", args.len()),
						span.into_range(),
					)
					.with_label("wrong number of arguments"));
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
						Err(Diagnostic::new(msg, args[0].1.into_range()).with_label("not usable as an error"))
					}
				}
			}

			"ord" => {
				let (val, typ) = self.cast_operand(name, args, span)?;
				if !typ.is_enumish() {
					return Err(
						Diagnostic::new(format!("`ord` expects an Ordinal, got {typ}"), span.into_range())
							.with_label("not an enum or sum"),
					);
				}
				let tag = self.enum_tag(&typ, val);
				Ok(Some((self.intcast(tag, types::I64, true), Typ::Int(64))))
			}

			"type_info" => {
				let [(Expr::Ident(name), at)] = args else {
					return Err(Diagnostic::new("`type_info` takes one type name", span.into_range())
						.with_label("expected a struct or enum"));
				};
				let def = self.type_def(name, *at)?;
				Ok(Some(self.quote(&[(def, *at)], *at)?))
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
		let (file, line, _) = self.map.locate_span(span.into_range());
		let field = |n: &str, e| (Some(n.to_string()), (e, span));
		let fields = [
			field("file", Expr::String(file.to_string())),
			field("line", Expr::Int(line as i64)),
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
				return Err(
					Diagnostic::new(format!("`{t}` has no definition to reflect"), span.into_range())
						.with_label("not a struct or enum"),
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
				let func = self.import_fn(comp::RT_COMP_STRUCT, &[self.int; 2], None);
				self.b.ins().call(func, &[name, nfields]);
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
			Typ::Int(_) | Typ::ISize | Typ::UInt(_) | Typ::USize => (comp::TAG_INT, self.scalar_bits(val, typ)),
			Typ::Float(32 | 64) => (comp::TAG_FLOAT, self.scalar_bits(val, typ)),
			t if t.is_unit() => (comp::TAG_UNIT, self.b.ins().iconst(self.int, 0)),
			_ => {
				return Err(Diagnostic::new(comp::UNREIFIABLE, span.into_range()).with_label("not comptime-reifiable"));
			}
		};
		let tag_v = self.b.ins().iconst(self.int, tag);
		let func = self.import_fn(comp::RT_COMP_YIELD, &[self.int, self.int], None);
		self.b.ins().call(func, &[tag_v, bits]);
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
				_ => Err(
					Diagnostic::new(format!("`{target}` casts a single value"), span.into_range())
						.with_label("wrong number of arguments"),
				),
			};
		};
		if let Some(out) = self.cast_prim(target, value, span)? {
			return Ok(out);
		}
		if let Some((ok, err)) = self.types.result_parts(target) {
			return self.result_init(ok, err, value);
		}
		let (val, typ) = self.check_expr(value, target)?;
		if typ == *target {
			return Ok((val, typ));
		}
		if let Some(out) = self.assert_cast(val, &typ, target, span)? {
			return Ok(out);
		}
		if let (Typ::Struct(name, _), Typ::TupleStruct(p, _)) = (target, &typ)
			&& p == role::PTR
		{
			// a raw address is a place
			self.require_unsafe(&format!("{target} cast"), span)?;
			if is_c_struct(self.types.consts.anns, name) {
				let msg = format!("`{target}` is a `@c` struct, C layout behind a `ptr`");
				return Err(Diagnostic::new(msg, span.into_range()).with_label("copy it with `p.read[T]()`"));
			}
			return Ok((val, target.clone()));
		}
		if let (Typ::CStr, Typ::TupleStruct(p, _)) = (target, &typ)
			&& p == role::PTR
		{
			self.require_unsafe("cstr cast", span)?;
			return Ok((val, Typ::CStr));
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
			self.assign_fields(val, ptr, fs, false);
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
		Err(Diagnostic::new(format!("cannot cast {typ} to {target}"), value.1.into_range()).with_label("no conversion"))
	}

	// Assertion casts.
	fn assert_cast(&mut self, obj: Value, typ: &Typ, target: &Typ, span: Span) -> Result<Option<TypedVal>, Diagnostic> {
		if !matches!(typ, Typ::Any | Typ::Trait(_) | Typ::Error) {
			return Ok(None);
		}
		let opt = self.types.core_enum(role::OPTION, std::slice::from_ref(target));
		let none = self.make_option(&opt, None);
		// `any` is tagged by typeid, a trait object by its vtable.
		let want = if *typ == Typ::Any {
			self.b.ins().iconst(self.int, typeid(target))
		} else {
			let tn = if let Typ::Trait(tn) = typ { tn } else { role::ERROR };
			if !self.trait_impls.contains(&(target.key(), tn.to_string())) {
				return Ok(Some((none, opt)));
			}
			self.data_addr(&oi_symbol(&format!("vtable_{}_{tn}", target.key())))
		};
		let got = self.ld_word(obj, 0);
		let same = self.b.ins().icmp(IntCC::Equal, got, want);
		let data = self.load_bind(obj, typ, target, 8, span);
		let some = self.make_option(&opt, Some(data));
		Ok(Some((self.b.ins().select(same, some, none), opt)))
	}

	// Numeric and string casts.
	fn cast_prim(&mut self, target: &Typ, value: &Spanned<Expr>, span: Span) -> Result<Option<TypedVal>, Diagnostic> {
		use Typ::{Array, Float, ISize, Int, Rune, Str, UInt, USize};
		if !matches!(target, Int(_) | UInt(_) | ISize | USize | Float(_) | Rune | Str) {
			return Ok(None);
		}
		if let Float(w) = target
			&& !matches!(w, 32 | 64)
		{
			return Err(Diagnostic::new(
				format!("f{w} casts are not yet supported by the JIT backend"),
				span.into_range(),
			)
			.with_label("not yet implemented"));
		}
		let (val, typ) = self.expr(value)?;
		if typ == Typ::Any {
			return self.assert_cast(val, &typ, target, span);
		}
		let (val, typ) = self.enum_as_backing(val, typ, value.1)?;
		if *target == Str && !matches!(typ, Str | Array(_)) {
			return Err(
				Diagnostic::new(format!("cannot cast {typ} to {target}"), value.1.into_range())
					.with_label("`.str()` formats a value"),
			);
		}
		if typ == *target {
			return Ok(Some((val, typ)));
		}
		let cl = cl_type(target, self.int);
		let signed = matches!(typ, Int(_) | ISize);
		let out = match (target, &typ) {
			(Str, Array(e)) if **e == UInt(8) => self.rt_call("str_from_bytes", &[val]).unwrap(),
			(Float(_), Float(64)) => self.b.ins().fdemote(cl, val),
			(Float(_), Float(_)) => self.b.ins().fpromote(cl, val),
			(Float(_), UInt(_) | USize) => self.b.ins().fcvt_from_uint(cl, val),
			(Float(_), Int(_) | ISize) => self.b.ins().fcvt_from_sint(cl, val),
			(_, Float(_)) => {
				let to_signed = !matches!(target, UInt(_) | USize);
				let wide = if to_signed {
					self.b.ins().fcvt_to_sint_sat(self.int, val)
				} else {
					self.b.ins().fcvt_to_uint_sat(self.int, val)
				};
				self.truncate(wide, target, to_signed)
			}
			(_, Int(_) | UInt(_) | ISize | USize | Rune) => self.truncate(val, target, signed),
			_ => {
				let label = if typ == Str {
					format!("`{target}.try_from(...)` parses strings")
				} else {
					"not castable".into()
				};
				return Err(
					Diagnostic::new(format!("cannot cast {typ} to {target}"), value.1.into_range()).with_label(label),
				);
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
			return Err(
				Diagnostic::new("cannot extract a sum member by casting", span.into_range())
					.with_label("no member extraction yet"),
			);
		}
		let variants = self.variants_of(&typ);
		if enum_boxed(&variants) {
			return Err(
				Diagnostic::new(format!("`{typ}` has no backing value to cast"), span.into_range())
					.with_label("no backing value"),
			);
		}
		let bt = variants.first().and_then(|v| v.backing.clone()).unwrap_or(Typ::Int(64));
		if bt == Typ::Str {
			let raw = |v: &VariantInfo| v.raw.clone().unwrap_or_else(|| v.name.clone());
			let mut out = self.str_const(&raw(&variants[0]));
			for v in &variants[1..] {
				let d = self.b.ins().iconst(self.int, v.disc);
				let hit = self.b.ins().icmp(IntCC::Equal, val, d);
				let s = self.str_const(&raw(v));
				out = self.b.ins().select(hit, s, out);
			}
			return Ok((out, Typ::Str));
		}
		let cl = cl_type(&bt, self.int);
		let val = if cl == self.int {
			val
		} else {
			self.b.ins().ireduce(cl, val)
		};
		Ok((val, bt))
	}

	// Evaluate the operand of a single-argument cast.
	pub(super) fn cast_operand(
		&mut self,
		name: &str,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if args.len() != 1 {
			return Err(
				Diagnostic::new(format!("`{name}` cast takes exactly 1 argument"), span.into_range())
					.with_label("wrong number of arguments"),
			);
		}
		self.expr(&args[0])
	}
}
