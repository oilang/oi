use crate::compiler::comp;

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
				if args.is_empty() {
					return Err(
						Diagnostic::new(format!("`{name}` takes at least 1 argument"), span.into_range())
							.with_label("missing argument"),
					);
				}
				let sink = match name {
					"eprint" | "ewrite" => runtime::Sink::Err,
					_ => runtime::Sink::Out,
				};
				let newline = matches!(name, "print" | "eprint");
				for (i, arg) in args.iter().enumerate() {
					if i > 0 {
						self.write_lit(" ", sink);
					}
					let (val, typ) = self.expr(arg)?;
					self.emit_print(val, &typ, false, sink);
				}
				if newline {
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
				let (av, at) = match self.ret.clone() {
					// resolve enum shorthands
					Some((Typ::Result(_, err), _)) if *err != Typ::Error => self.check_expr(&args[0], &err)?,
					_ => self.expr(&args[0])?,
				};
				match self.ret.clone() {
					Some((Typ::Result(ok, err), _)) if at == *err => {
						let v = self.make_enum(&result_variants(&ok, &err), 1, &[av]);
						Ok(Some((v, Typ::Result(ok, err))))
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
				let out = if self.int == types::I32 {
					tag
				} else {
					self.b.ins().ireduce(types::I32, tag)
				};
				Ok(Some((out, Typ::Int(32))))
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

	// Yield a value to the `comp` host (recursive).
	fn comp_yield(&mut self, val: Value, typ: &Typ, span: Span) -> Result<(), Diagnostic> {
		let narrow = |w: u16| cl_int_for_width(w).bits() < self.int.bits();
		let (tag, bits) = match typ {
			Typ::Struct(name, fields) => {
				for (i, f) in fields.iter().enumerate() {
					let fv = self
						.b
						.ins()
						.load(cl_type(&f.typ, self.int), MemFlags::new(), val, (i * 8) as i32);
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
				(comp::TAG_ARRAY, self.array_parts(val, typ).1)
			}
			Typ::Bool => (comp::TAG_BOOL, val),
			Typ::Str => (comp::TAG_STR, val),
			Typ::Int(w) if narrow(*w) => (comp::TAG_INT, self.b.ins().sextend(self.int, val)),
			Typ::Int(_) | Typ::ISize => (comp::TAG_INT, val),
			Typ::UInt(w) if narrow(*w) => (comp::TAG_INT, self.b.ins().uextend(self.int, val)),
			Typ::UInt(_) | Typ::USize => (comp::TAG_INT, val),
			Typ::Float(32) => {
				let f64v = self.b.ins().fpromote(types::F64, val);
				(comp::TAG_FLOAT, self.b.ins().bitcast(self.int, MemFlags::new(), f64v))
			}
			Typ::Float(64) => (comp::TAG_FLOAT, self.b.ins().bitcast(self.int, MemFlags::new(), val)),
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
			let Typ::TupleStruct(name, _) = target else {
				return Err(
					Diagnostic::new(format!("`{target}` casts a single value"), span.into_range())
						.with_label("wrong number of arguments"),
				);
			};
			return self.construct_tuple_struct(name, args, span);
		};
		if let Some(out) = self.cast_prim(target, value, span)? {
			return Ok(out);
		}
		if let Typ::Result(ok, err) = target
			&& **err == Typ::Error
		{
			return self.result_init((**ok).clone(), value);
		}
		let (val, typ) = self.check_expr(value, target)?;
		if typ == *target {
			return Ok((val, typ));
		}
		if let (Typ::Array(e), Typ::Str) = (target, &typ)
			&& **e == Typ::UInt(8)
		{
			let (data, len) = self.array_parts(val, &typ);
			let data = self.rt_call("ptr_buffer", &[data, len]).unwrap();
			return Ok((self.make_array(data, len, target), target.clone()));
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

	// Numeric and string casts.
	fn cast_prim(&mut self, target: &Typ, value: &Spanned<Expr>, span: Span) -> Result<Option<TypedVal>, Diagnostic> {
		use Typ::{Array, Float, ISize, Int, Str, UInt, USize};
		if !matches!(target, Int(_) | UInt(_) | ISize | USize | Float(_) | Str) {
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
		let (val, typ) = self.enum_as_backing(val, typ, value.1)?;
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
			(_, Int(_) | UInt(_) | ISize | USize) => self.truncate(val, target, signed),
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
		match target {
			Typ::Int(w) => self.reduce_int(val, *w),
			Typ::UInt(w) => self.reduce_uint(val, *w),
			_ => val,
		}
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
