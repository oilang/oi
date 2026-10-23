use super::*;

impl<'a, M: Module> Translator<'a, M> {
	pub fn write_lit(&mut self, s: &str, sink: runtime::Sink) {
		let ptr = self.str_const(s);
		self.raw(ptr, sink);
	}

	fn raw(&mut self, s: Value, sink: runtime::Sink) {
		self.emit_frag(runtime::Tag::Raw, s, 0, false, sink);
	}

	fn emit_frag(&mut self, tag: runtime::Tag, bits: Value, width: u16, quote: bool, sink: runtime::Sink) {
		let tag = self.b.ins().iconst(self.int, tag as i64);
		let width = self.b.ins().iconst(self.int, width as i64);
		let quote = self.b.ins().iconst(self.int, quote as i64);
		let sink_v = self.b.ins().iconst(self.int, sink as i64);
		self.rt_call("write", &[tag, bits, width, quote, sink_v]);
	}

	// A type's `fmt` into the sink, or its `str` as a fallback.
	fn fmt_impl(&mut self, name: &str, val: Value, typ: &Typ, sink: runtime::Sink) -> bool {
		let own = |s: &mut Self, m: &str| {
			let sig = (s.funcs.get(&format!("{name}.{m}")).cloned())
				.or_else(|| s.recv_instance(&format!("{}.{m}", rc::base_name(name)), typ))?;
			(!sig.default).then_some(sig)
		};
		if let Some(sig) = own(self, "fmt") {
			let vtable = self.data_addr(&oi_symbol(&format!("vtable_{}_{}[string]", role::SINK, role::WRITE)));
			let id = self.b.ins().iconst(self.int, sink as i64);
			let data = self.lent_block(&[], &[id]);
			let obj = self.lent_object(vtable, data);
			self.emit_call(&sig, &[val, obj]);
		} else if let Some(sig) = own(self, "str").filter(|sig| sig.params.len() == 1 && sig.ret == Typ::Str) {
			let s = self.emit_call(&sig, &[val]).0;
			self.raw(s, sink);
		} else {
			return false;
		}
		true
	}

	// Universal `str`, or `repr` when quoted.
	pub(crate) fn derived_str(&mut self, val: Value, typ: &Typ, quote: bool) -> Value {
		let mark = self.rt_call("str_mark", &[]).unwrap();
		self.emit_print(val, typ, quote, runtime::Sink::Buf);
		self.rt_call("str_take", &[mark]).unwrap()
	}

	// The string paired with `tag`'s discriminant, or else `out`.
	pub(super) fn select_by_disc(&mut self, tag: Value, mut out: Value, strs: &[(i64, String)]) -> Value {
		for (disc, s) in strs {
			let s = self.str_const(s);
			let hit = self.b.ins().icmp_imm(IntCC::Equal, tag, *disc);
			out = self.b.ins().select(hit, s, out);
		}
		out
	}

	// Variant printing. Handles recursive sums.
	fn call_variant(&mut self, typ: &Typ, val: Value, quote: bool, sink: runtime::Sink) {
		let sym = oi_symbol(&format!("print_{}_{}_{}", typ.key(), quote as u8, sink as u8));
		if self.module.declarations().get_name(&sym).is_none() {
			self.out.printers.push((sym.clone(), typ.clone(), quote, sink));
		}
		self.extern_call(&sym, &[val], None);
	}

	// Dispatch on the typeid over every type coerced into `any`.
	fn emit_any(&mut self, val: Value, typ: &Typ, quote: bool, sink: runtime::Sink) {
		let done = self.b.create_block();
		let tag = self.enum_tag(typ, val);
		for t in self.out.any_types.clone() {
			self.on_variant(tag, typeid(&t), done, |s| match typ {
				Typ::Any => {
					let pv = s.ld_typ(val, 8, &t);
					s.emit_print(pv, &t, quote, sink)
				}
				_ => s.write_lit(&t.to_string(), sink),
			});
		}
		self.write_lit(&format!("<{typ}>"), sink);
		self.enter(done);
	}

	// Payload `Display`.
	pub(crate) fn emit_variant(&mut self, typ: &Typ, val: Value, quote: bool, sink: runtime::Sink) {
		if matches!(typ, Typ::Any | Typ::TypeId) {
			return self.emit_any(val, typ, quote, sink);
		}
		if let Typ::Ref(_) = typ {
			let (val, inner) = self.deref(val, typ);
			return self.emit_print(val, &inner, quote, sink);
		}
		let named = !matches!(typ, Typ::Sum(..));
		let done = self.b.create_block();
		let variants = self.variants_of(typ);
		let tag = self.enum_tag(typ, val);
		for v in &variants {
			if v.payload.is_empty() {
				continue;
			}
			self.on_variant(tag, v.disc, done, |s| {
				if !named {
					let pv = s.ld_typ(val, 8, &v.payload[0]);
					s.emit_print(pv, &v.payload[0], quote, sink);
				} else {
					let fields: Vec<_> = (v.payload.iter().enumerate())
						.map(|(i, t)| (v.names.get(i).cloned(), t.clone()))
						.collect();
					let delims = match v.names.is_empty() {
						true => [&format!("{}.(", v.name), ")"],
						false => [&format!("{}.{{", v.name), "}"],
					};
					s.print_fields((val, typ), 8, &fields, delims, sink);
				}
			});
		}
		let empty = self.str_const("");
		let names: Vec<_> = variants.into_iter().map(|v| (v.disc, v.name)).collect();
		let name = self.select_by_disc(tag, empty, &names);
		self.raw(name, sink);
		self.enter(done);
	}

	// Print fields with delimeters and names where applicable.
	fn print_fields(
		&mut self,
		(val, typ): (Value, &Typ),
		base: i32,
		fields: &[(Option<String>, Typ)],
		delims: [&str; 2],
		sink: runtime::Sink,
	) {
		self.write_lit(delims[0], sink);
		for (i, (name, ft)) in fields.iter().enumerate() {
			if i > 0 {
				self.write_lit(", ", sink);
			}
			if let Some(name) = name {
				self.write_lit(&format!("{name} = "), sink);
			}
			let fv = self.opt_payload(val, typ, ft, base + (i * 8) as i32);
			self.emit_print(fv, ft, true, sink);
		}
		self.write_lit(delims[1], sink);
	}

	pub fn emit_print(&mut self, val: Value, typ: &Typ, quote: bool, sink: runtime::Sink) {
		let own = match typ {
			Typ::Sum(n, _) => Some(n.as_str()),
			Typ::Rune => Some("rune"),
			Typ::Map(..) => Some("map"),
			_ => typ.nominal(),
		};
		if own.is_some_and(|name| self.fmt_impl(name, val, typ, sink)) {
			return;
		}
		match typ {
			Typ::Tuple(fields) => self.print_fields((val, typ), 0, fields, ["(", ")"], sink),

			Typ::Array(elem) | Typ::FixedArray(elem, _) => {
				self.write_lit("[", sink);
				self.each_elem(val, typ, |s, i, ev| {
					let sink_v = s.b.ins().iconst(s.int, sink as i64);
					s.rt_call("write_sep", &[i, sink_v]);
					s.emit_print(ev, elem, true, sink);
				});
				self.write_lit("]", sink);
			}

			Typ::Struct(sname, fields) => {
				// less-noisy anonymous struct names
				let anon = sname.starts_with("struct{");
				let sname = if anon { "" } else { display_name(sname) };
				let fields: Vec<_> = fields.iter().map(|f| (Some(f.name.clone()), f.typ.clone())).collect();
				self.print_fields((val, typ), 0, &fields, [&format!("{sname}.{{"), "}"], sink);
			}

			Typ::TupleStruct(name, fields) => {
				let name = display_name(name).to_string();
				let body = Typ::Tuple(fields.clone());
				self.write_lit(&format!("{name}."), sink);
				match typ.newtype() {
					Some(_) => {
						let boxed = self.heap_slots(&[val]);
						self.emit_print(boxed, &body, quote, sink);
						self.rt_call("free", &[boxed]);
					}
					None => self.emit_print(val, &body, quote, sink),
				}
			}

			Typ::Atom => self.raw(val, sink),

			Typ::Enum(_) | Typ::Sum(..) => self.call_variant(&typ.clone(), val, quote, sink),

			Typ::Fn(..) | Typ::Closure(..) => self.write_lit("<fn>", sink),
			Typ::Map(..) => self.write_lit("<map>", sink),
			Typ::Ast => {
				let s = self.ast_method(val, "str", None);
				self.emit_print(s, &Typ::Str, quote, sink)
			}
			Typ::Any | Typ::TypeId => self.call_variant(typ, val, quote, sink),

			Typ::Annotated(_, t) => self.emit_print(val, &t.clone(), quote, sink),

			Typ::Ref(_) => self.call_variant(typ, val, quote, sink),

			Typ::Trait(tn) => {
				let ((_, _, tfields, tmethods, _), _) = self.types.trait_instance(tn);
				let slot = (trait_fns(tmethods).count() + tfields.len()) * 8;
				let (vtable, data) = self.unbox(val);
				let fnptr = self.ld_word(vtable, slot as i32);
				let sig = Typ::Fn(vec![FnParam::new(typ.clone())], Box::new(Typ::Str));
				let Ok((s, _)) = self.call_value("str", Callee::Addr(fnptr), &sig, &[], Some(data), Span::default())
				else {
					unreachable!("no args to check")
				};
				self.raw(s, sink);
			}

			Typ::Error => {
				let s = self.error_message(val);
				self.emit_print(s, &Typ::Str, quote, sink);
			}

			Typ::Rune => {
				let n = self.b.ins().uextend(self.int, val);
				self.emit_frag(runtime::Tag::UInt, n, 0, quote, sink);
			}

			Typ::Int(128) | Typ::UInt(128) => {
				let signed = self.b.ins().iconst(self.int, matches!(typ, Typ::Int(_)) as i64);
				let sink = self.b.ins().iconst(self.int, sink as i64);
				self.rt_call("write_wide", &[val, signed, sink]);
			}

			_ => {
				let tag = match typ {
					Typ::CStr => runtime::Tag::UInt,
					_ => map_key_tag(typ).expect("handled above"),
				};
				// floats print at their own width, so f16/f32 travel unpromoted
				let (bits, float_width) = match typ {
					Typ::Float(16) => {
						let i16v = self.b.ins().bitcast(types::I16, MemFlags::new(), val);
						(self.b.ins().uextend(self.int, i16v), 16)
					}
					Typ::Float(32) => {
						let i32v = self.b.ins().bitcast(types::I32, MemFlags::new(), val);
						(self.b.ins().uextend(self.int, i32v), 32)
					}
					Typ::Float(64) => (self.scalar_bits(val, typ), 64),
					Typ::Float(128) => {
						panic!("f128 printing not yet supported by the JIT backend")
					}
					Typ::Float(w) => panic!("unsupported float width f{w}"),
					_ => (self.scalar_bits(val, typ), 0),
				};
				self.emit_frag(tag, bits, float_width, quote, sink);
			}
		}
	}
}
