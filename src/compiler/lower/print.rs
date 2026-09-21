use super::*;

impl<'a, M: Module> Translator<'a, M> {
	pub fn write_lit(&mut self, s: &str, sink: runtime::Sink) {
		let ptr = self.str_const(s);
		self.emit_frag(runtime::Tag::Raw, ptr, 0, false, sink);
	}

	fn emit_frag(&mut self, tag: runtime::Tag, bits: Value, width: u16, quote: bool, sink: runtime::Sink) {
		let tag = self.b.ins().iconst(self.int, tag as i64);
		let width = self.b.ins().iconst(self.int, width as i64);
		let quote = self.b.ins().iconst(self.int, quote as i64);
		let sink_v = self.b.ins().iconst(self.int, sink as i64);
		self.rt_call("write", &[tag, bits, width, quote, sink_v]);
	}

	// A named type's `str` impl.
	fn str_impl(&mut self, name: &str, val: Value, typ: &Typ) -> Option<Value> {
		let base = rc::base_name(name);
		let sig = (self.funcs.get(&format!("{name}.str")).cloned())
			.or_else(|| self.recv_instance(&format!("{base}.str"), typ))?;
		(sig.params.len() == 1 && sig.ret == Typ::Str).then(|| self.emit_call(&sig, &[val]).0)
	}

	// Universal `str` method.
	pub(crate) fn derived_str(&mut self, val: Value, typ: &Typ) -> Value {
		let mark = self.rt_call("str_mark", &[]).unwrap();
		self.emit_print(val, typ, false, runtime::Sink::Buf);
		self.rt_call("str_take", &[mark]).unwrap()
	}

	// Enum `Display`.
	pub(super) fn enum_name_str(&mut self, typ: &Typ, val: Value) -> Value {
		let variants = self.variants_of(typ);
		let tag = self.enum_tag(typ, val);
		let mut ptr = self.str_const("");
		for v in &variants {
			let s = self.str_const(&v.name);
			let disc = self.b.ins().iconst(self.int, v.disc);
			let hit = self.b.ins().icmp(IntCC::Equal, tag, disc);
			ptr = self.b.ins().select(hit, s, ptr);
		}
		ptr
	}

	// Variant printing. Handles recursive sums.
	fn call_variant(&mut self, typ: &Typ, val: Value, quote: bool, sink: runtime::Sink) {
		let sym = oi_symbol(&format!("print_{}_{}_{}", typ.key(), quote as u8, sink as u8));
		if self.module.declarations().get_name(&sym).is_none() {
			self.printers.push((sym.clone(), typ.clone(), quote, sink));
		}
		let callee = self.import_fn(&sym, &[cl_type(typ, self.int)], None);
		self.b.ins().call(callee, &[val]);
	}

	// Payload `Display`.
	pub(crate) fn emit_variant(&mut self, typ: &Typ, val: Value, quote: bool, sink: runtime::Sink) {
		let named = !matches!(typ, Typ::Sum(..));
		let done = self.b.create_block();
		let variants = self.variants_of(typ);
		let tag = self.enum_tag(typ, val);
		for v in &variants {
			if v.payload.is_empty() {
				continue;
			}
			let (hit, next) = (self.b.create_block(), self.b.create_block());
			let disc = self.b.ins().iconst(self.int, v.disc);
			let is = self.b.ins().icmp(IntCC::Equal, tag, disc);
			self.b.ins().brif(is, hit, &[], next, &[]);
			self.b.seal_block(hit);
			self.b.switch_to_block(hit);
			if !named {
				let pv = self.b.ins().load(cl_type(&v.payload[0], self.int), MemFlags::new(), val, 8);
				self.emit_print(pv, &v.payload[0], quote, sink);
			} else {
				self.write_lit(&v.name, sink);
				let braced = !v.names.is_empty();
				self.write_lit(if braced { ".{" } else { ".(" }, sink);
				for (i, pt) in v.payload.iter().enumerate() {
					if i > 0 {
						self.write_lit(", ", sink);
					}
					if braced {
						self.write_lit(&format!("{} = ", v.names[i]), sink);
					}
					let pv = self.opt_payload(val, typ, pt, (8 + i * 8) as i32);
					self.emit_print(pv, pt, true, sink);
				}
				self.write_lit(if braced { "}" } else { ")" }, sink);
			}
			self.b.ins().jump(done, &[]);
			self.b.seal_block(next);
			self.b.switch_to_block(next);
		}
		let ptr = self.enum_name_str(typ, val);
		self.emit_frag(runtime::Tag::Raw, ptr, 0, false, sink);
		self.b.ins().jump(done, &[]);
		self.b.seal_block(done);
		self.b.switch_to_block(done);
	}

	pub fn emit_print(&mut self, val: Value, typ: &Typ, quote: bool, sink: runtime::Sink) {
		match typ {
			Typ::Tuple(fields) => {
				self.write_lit("(", sink);
				for (i, (name, ft)) in fields.iter().enumerate() {
					if i > 0 {
						self.write_lit(", ", sink);
					}
					if let Some(name) = name {
						self.write_lit(&format!("{name} = "), sink);
					}
					let cl = cl_type(ft, self.int);
					let fv = self.b.ins().load(cl, MemFlags::new(), val, (i * 8) as i32);
					self.emit_print(fv, ft, true, sink);
				}
				self.write_lit(")", sink);
			}

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
				if let Some(s) = self.str_impl(sname, val, typ) {
					return self.emit_frag(runtime::Tag::Raw, s, 0, false, sink);
				}
				// less-noisy anonymous struct names
				let anon = sname.starts_with("struct{");
				let sname = if anon { "" } else { display_name(sname) }.to_string();
				let fields = fields.clone();
				self.write_lit(&format!("{sname}.{{"), sink);
				for (i, f) in fields.iter().enumerate() {
					if i > 0 {
						self.write_lit(", ", sink);
					}
					self.write_lit(&format!("{} = ", f.name), sink);
					let cl = cl_type(&f.typ, self.int);
					let fv = self.b.ins().load(cl, MemFlags::new(), val, (i * 8) as i32);
					self.emit_print(fv, &f.typ, true, sink);
				}
				self.write_lit("}", sink);
			}

			Typ::TupleStruct(name, fields) => {
				if let Some(s) = self.str_impl(name, val, typ) {
					return self.emit_frag(runtime::Tag::Raw, s, 0, false, sink);
				}
				let name = display_name(name).to_string();
				let body = Typ::Tuple(fields.clone());
				self.write_lit(&name, sink);
				let val = match typ.newtype() {
					Some(_) => {
						let tmp = self.call_alloc(1);
						self.b.ins().store(MemFlags::new(), val, tmp, 0);
						tmp
					}
					None => val,
				};
				self.emit_print(val, &body, quote, sink);
			}

			Typ::Atom => {
				self.emit_frag(runtime::Tag::Raw, val, 0, false, sink);
			}

			Typ::Enum(_) | Typ::Option(_) | Typ::Result(..) | Typ::Sum(..) => {
				self.call_variant(&typ.clone(), val, quote, sink);
			}

			Typ::Fn(..) | Typ::Closure(..) => self.write_lit("<fn>", sink),
			Typ::Map(..) => self.write_lit("<map>", sink),
			Typ::Ast => {
				let s = self.ast_method(val, "str", None);
				self.emit_print(s, &Typ::Str, quote, sink)
			}
			Typ::Any => self.write_lit("<any>", sink),

			Typ::Annotated(_, t) => self.emit_print(val, &t.clone(), quote, sink),

			Typ::Ref(_) => {
				let inner = self.peeled(typ);
				self.emit_print(val, &inner, quote, sink)
			}

			Typ::Trait(tn) => {
				let (_, _, tfields, tmethods) = self.types.traits[tn.as_str()];
				let slot = (trait_fns(tmethods).count() + tfields.len()) * 8;
				let vtable = self.b.ins().load(self.int, MemFlags::new(), val, 0);
				let data = self.b.ins().load(self.int, MemFlags::new(), val, 8);
				let fnptr = self.b.ins().load(self.int, MemFlags::new(), vtable, slot as i32);
				let sig = Typ::Fn(vec![FnParam::new(typ.clone())], Box::new(Typ::Str));
				let Ok((s, _)) = self.call_value("str", Callee::Addr(fnptr), &sig, &[], Some(data), (0..0).into())
				else {
					unreachable!("no args to check")
				};
				self.emit_frag(runtime::Tag::Raw, s, 0, false, sink);
			}

			Typ::Error => {
				let s = self.error_message(val);
				self.emit_print(s, &Typ::Str, quote, sink);
			}

			_ => {
				let tag = match typ {
					Typ::Bool => runtime::Tag::Bool,
					Typ::Int(_) | Typ::ISize => runtime::Tag::Int,
					Typ::UInt(_) | Typ::USize | Typ::CStr => runtime::Tag::UInt,
					Typ::Float(_) => runtime::Tag::Float,
					Typ::Str => runtime::Tag::Str,
					Typ::Atom
					| Typ::Tuple(_)
					| Typ::Array(_)
					| Typ::FixedArray(..)
					| Typ::Struct(..)
					| Typ::TupleStruct(..)
					| Typ::Enum(_)
					| Typ::Option(_)
					| Typ::Result(..)
					| Typ::Sum(..)
					| Typ::Fn(..)
					| Typ::Annotated(..)
					| Typ::Closure(..)
					| Typ::Trait(_)
					| Typ::Error
					| Typ::Map(..)
					| Typ::Access(..)
					| Typ::Ast
					| Typ::Any
					| Typ::Ref(_) => {
						unreachable!("handled above")
					}
				};
				// normalize to pointer-sized before passing to the runtime
				let (bits, float_width) = match typ {
					Typ::Float(16) => {
						let i16v = self.b.ins().bitcast(types::I16, MemFlags::new(), val);
						(self.b.ins().uextend(self.int, i16v), 16)
					}
					Typ::Float(32) => {
						let i32v = self.b.ins().bitcast(types::I32, MemFlags::new(), val);
						(self.b.ins().uextend(self.int, i32v), 32)
					}
					Typ::Float(64) => (self.b.ins().bitcast(self.int, MemFlags::new(), val), 64),
					Typ::Float(128) => {
						panic!("f128 printing not yet supported by the JIT backend")
					}
					Typ::Float(w) => panic!("unsupported float width f{w}"),
					Typ::Int(w) if cl_int_for_width(*w).bits() < self.int.bits() => {
						(self.b.ins().sextend(self.int, val), 0)
					}
					Typ::UInt(w) if cl_int_for_width(*w).bits() < self.int.bits() => {
						(self.b.ins().uextend(self.int, val), 0)
					}
					_ => (val, 0),
				};
				self.emit_frag(tag, bits, float_width, quote, sink);
			}
		}
	}
}
