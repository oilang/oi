use crate::compiler::{comp, role};

use super::*;

#[derive(Clone, Copy)]
enum Sign {
	Signed,
	Unsigned,
}

impl<'a, M: Module> Translator<'a, M> {
	// Widen/narrow integers.
	// Sign-extend `val` to i64, clamp to `[low, hi]`.
	fn clamp_to_width(
		&mut self,
		val: Value,
		extend: Sign,
		low: Option<(i64, Sign)>,
		hi: i64,
		hi_sign: Sign,
		target_cl: types::Type,
	) -> Value {
		let src_cl = self.b.func.dfg.value_type(val);
		let v64 = if src_cl == types::I64 {
			val
		} else {
			match extend {
				Sign::Signed => self.b.ins().sextend(types::I64, val),
				Sign::Unsigned => self.b.ins().uextend(types::I64, val),
			}
		};
		let v64 = match low {
			Some((low, lo_sign)) => {
				let lo_c = self.b.ins().iconst(types::I64, low);
				let cc = match lo_sign {
					Sign::Unsigned => IntCC::UnsignedLessThan,
					Sign::Signed => IntCC::SignedLessThan,
				};
				let lt = self.b.ins().icmp(cc, v64, lo_c);
				self.b.ins().select(lt, lo_c, v64)
			}
			None => v64,
		};
		let hi_c = self.b.ins().iconst(types::I64, hi);
		let cc = match hi_sign {
			Sign::Unsigned => IntCC::UnsignedGreaterThan,
			Sign::Signed => IntCC::SignedGreaterThan,
		};
		let gt = self.b.ins().icmp(cc, v64, hi_c);
		let v64 = self.b.ins().select(gt, hi_c, v64);
		if target_cl == types::I64 {
			v64
		} else {
			self.b.ins().ireduce(target_cl, v64)
		}
	}

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

	fn parse_str(&mut self, val: Value, out: Typ) -> Result<TypedVal, Diagnostic> {
		let def = self.generic_fns[role::PARSE_INT].clone();
		let subst = HashMap::from([(def.type_params[0].name.clone(), out)]);
		let sig = self.declare_instance(role::PARSE_INT, &def, subst)?;
		Ok(self.emit_call(&sig, &[val]))
	}

	// A numeric cast builtin.
	pub(super) fn cast_call(
		&mut self,
		name: &str,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<Option<TypedVal>, Diagnostic> {
		// `int` and `float` are aliases for the default-width casts
		let name = match name {
			"int" => "i32",
			"float" => "f64",
			other => other,
		};

		if name == "string" {
			let (val, typ) = self.cast_operand(name, args, span)?;
			let (val, typ) = self.enum_as_backing(val, typ, args[0].1)?;
			if typ == Typ::Str {
				return Ok(Some((val, Typ::Str)));
			}
			if let Typ::Array(ref e) = typ
				&& **e == Typ::UInt(8)
			{
				return Ok(Some((self.rt_call("str_from_bytes", &[val]).unwrap(), Typ::Str)));
			}
			return Err(
				Diagnostic::new(format!("cannot cast {typ} to string"), args[0].1.into_range())
					.with_label("not castable to string"),
			);
		}

		if matches!(name, "isize" | "usize") {
			let signed = name == "isize";
			let (val, typ) = self.cast_operand(name, args, span)?;
			let out = match (&typ, signed) {
				(Typ::ISize, true) | (Typ::USize, false) => val,
				// isize -> usize: clamp negative to 0
				(Typ::ISize, false) => {
					let zero = self.b.ins().iconst(self.int, 0);
					let lt = self.b.ins().icmp(IntCC::SignedLessThan, val, zero);
					self.b.ins().select(lt, zero, val)
				}
				// usize -> isize: saturate at isize::MAX
				(Typ::USize, true) => {
					let max_v = self.b.ins().iconst(self.int, i64::MAX);
					let gt = self.b.ins().icmp(IntCC::UnsignedGreaterThan, val, max_v);
					self.b.ins().select(gt, max_v, val)
				}
				// int -> isize: sign-extend
				(Typ::Int(_), true) => {
					let src_cl = cl_type(&typ, self.int);
					if src_cl == self.int {
						val
					} else {
						self.b.ins().sextend(self.int, val)
					}
				}
				// uint -> usize: zero-extend
				(Typ::UInt(_), false) => {
					let src_cl = cl_type(&typ, self.int);
					if src_cl == self.int {
						val
					} else {
						self.b.ins().uextend(self.int, val)
					}
				}
				// int -> usize: sign-extend then clamp negative to 0
				(Typ::Int(_), false) => {
					let src_cl = cl_type(&typ, self.int);
					let v = if src_cl == self.int {
						val
					} else {
						self.b.ins().sextend(self.int, val)
					};
					let zero = self.b.ins().iconst(self.int, 0);
					let lt = self.b.ins().icmp(IntCC::SignedLessThan, v, zero);
					self.b.ins().select(lt, zero, v)
				}
				// uint -> isize: zero-extend then saturate at isize::MAX
				(Typ::UInt(_), true) => {
					let src_cl = cl_type(&typ, self.int);
					let v = if src_cl == self.int {
						val
					} else {
						self.b.ins().uextend(self.int, val)
					};
					let max_v = self.b.ins().iconst(self.int, i64::MAX);
					let gt = self.b.ins().icmp(IntCC::UnsignedGreaterThan, v, max_v);
					self.b.ins().select(gt, max_v, v)
				}
				_ => {
					return Err(
						Diagnostic::new(format!("cannot cast {typ} to {name}"), args[0].1.into_range())
							.with_label("not an integer"),
					);
				}
			};
			let out_typ = if signed { Typ::ISize } else { Typ::USize };
			return Ok(Some((out, out_typ)));
		}

		if let Some(target) = int_cast_width('i', name) {
			let (val, typ) = self.cast_operand(name, args, span)?;
			if typ == Typ::Str {
				return Ok(Some(self.parse_str(val, Typ::Int(target))?));
			}
			let (val, typ) = self.enum_as_backing(val, typ, args[0].1)?;
			let target_cl = cl_type(&Typ::Int(target), self.int);
			let out = match &typ {
				Typ::Int(w) if *w == target => val,
				Typ::Int(_) => self.clamp_to_width(
					val,
					Sign::Signed,
					Some((int_min(target), Sign::Signed)),
					int_max(target),
					Sign::Signed,
					target_cl,
				),
				Typ::UInt(_) => {
					self.clamp_to_width(val, Sign::Unsigned, None, int_max(target), Sign::Unsigned, target_cl)
				}
				_ => {
					return Err(
						Diagnostic::new(format!("cannot cast {typ} to i{target}"), args[0].1.into_range())
							.with_label("not an integer"),
					);
				}
			};
			return Ok(Some((out, Typ::Int(target))));
		}

		if let Some(target) = int_cast_width('u', name) {
			let (val, typ) = self.cast_operand(name, args, span)?;
			if typ == Typ::Str {
				return Ok(Some(self.parse_str(val, Typ::UInt(target))?));
			}
			let (val, typ) = self.enum_as_backing(val, typ, args[0].1)?;
			let target_cl = cl_type(&Typ::UInt(target), self.int);
			let out = match &typ {
				Typ::UInt(w) if *w == target => val,
				Typ::UInt(_) => {
					self.clamp_to_width(val, Sign::Unsigned, None, uint_max(target), Sign::Unsigned, target_cl)
				}
				Typ::Int(_) => self.clamp_to_width(
					val,
					Sign::Signed,
					Some((0, Sign::Signed)),
					uint_max(target),
					Sign::Unsigned,
					target_cl,
				),
				_ => {
					return Err(
						Diagnostic::new(format!("cannot cast {typ} to u{target}"), args[0].1.into_range())
							.with_label("not an integer"),
					);
				}
			};
			return Ok(Some((out, Typ::UInt(target))));
		}

		if matches!(name, "f16" | "f32" | "f64" | "f128") {
			let target: u16 = match name {
				"f16" => 16,
				"f32" => 32,
				"f128" => 128,
				_ => 64,
			};
			if args.len() != 1 {
				return Err(
					Diagnostic::new(format!("`{name}` cast takes exactly 1 argument"), span.into_range())
						.with_label("wrong number of arguments"),
				);
			}
			if target == 16 || target == 128 {
				return Err(Diagnostic::new(
					format!("f{target} casts are not yet supported by the JIT backend"),
					span.into_range(),
				)
				.with_label("not yet implemented"));
			}
			let (val, typ) = self.expr(&args[0])?;
			if typ == Typ::Str && target == 64 {
				let sig = self.funcs[role::PARSE_FLOAT].clone();
				return Ok(Some(self.emit_call(&sig, &[val])));
			}
			let target_cl = cl_type(&Typ::Float(target), self.int);
			let out = match &typ {
				Typ::Float(w) if *w == target => val,
				Typ::Float(_) if target == 64 => self.b.ins().fpromote(types::F64, val),
				Typ::Float(_) => self.b.ins().fdemote(types::F32, val),
				Typ::Int(_) => self.b.ins().fcvt_from_sint(target_cl, val),
				_ => {
					return Err(
						Diagnostic::new(format!("cannot cast {typ} to f{target}"), args[0].1.into_range())
							.with_label("not a number"),
					);
				}
			};
			return Ok(Some((out, Typ::Float(target))));
		}

		Ok(None)
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
