//! Marshalling Oi structs to/from C layout.

use std::ops::Range;

use cranelift::codegen::ir::ArgumentPurpose;
use cranelift::prelude::*;
use cranelift_module::Module;

use super::{FieldDef, FnParam, Translator, Typ, TypedVal, c_layout, check_c_sig, cl_type, is_c_struct};
use crate::ast::{Access, Expr, Span, Spanned, TypeExpr};
use crate::compiler::{CRegs, c_spread};
use crate::diagnostics::{Diagnostic, fail};

impl<M: Module> Translator<'_, M> {
	pub(super) fn require_unsafe(&self, what: &str, span: Span) -> Result<(), Diagnostic> {
		match self.unsafely {
			0 => fail(format!("`{what}` needs `unsafe`"), span, "no `unsafe` in scope"),
			_ => Ok(()),
		}
	}

	pub(super) fn require_pure(&self, what: &str, span: impl Into<Range<usize>>) -> Result<(), Diagnostic> {
		if self.pure {
			let msg = format!("`{what}` isn't allowed in a `@pure` fn");
			return Err(Diagnostic::new(msg, span.into()).with_label("not `@pure`"));
		}
		Ok(())
	}

	// Copy C struct in/out of foreign memory, based on `read`.
	pub(super) fn ptr_copy(
		&mut self,
		read: bool,
		recv: Option<Value>,
		type_args: &[Spanned<TypeExpr>],
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		self.require_unsafe(if read { "ptr.read" } else { "ptr.write" }, span)?;
		match (read, recv, type_args, args) {
			(true, Some(c), [(te, ts)], []) => {
				let typ = self.types.resolve(te, *ts)?;
				if self.oi_struct(&typ) {
					return Ok((self.copy_in(c, &typ), typ));
				}
				let Some(fields) = self.c_fields(&typ, *ts)? else {
					return Ok((self.c_load(&typ, c, 0), typ));
				};
				let oi = self.struct_slot(&typ, &[])?;
				self.copy_fields(oi, c, 0, &fields, false);
				Ok((oi, typ))
			}
			(false, Some(c), [], [value]) => {
				let (oi, typ) = self.expr(value)?;
				if self.oi_struct(&typ) {
					self.assign_slots(oi, c, &typ, false);
				} else if let Some(fields) = self.c_fields(&typ, value.1)? {
					self.copy_fields(oi, c, 0, &fields, true);
				} else {
					self.c_store(&typ, oi, c, 0);
				}
				Ok(self.unit_value())
			}
			_ => {
				let usage = if read { "p.read[T]()" } else { "p.write(v)" };
				fail(
					"this copies a `@c` struct or C scalar through a `ptr`",
					span,
					format!("write `{usage}`"),
				)
			}
		}
	}

	// Spread a C ABI call's `@c` struct args into C ABI params, after any returned struct's buffer.
	pub(super) fn c_args(&mut self, params: &[FnParam], ret: &Typ, vals: &[Value]) -> Vec<Value> {
		let arch = self.module.isa().triple().architecture;
		let mut out = vec![];
		if let Some([(None, _)]) = ret.c_by_val(&self.types, arch).as_deref() {
			out.push(self.stack_slot(ret.c_size_align(&self.types).expect("validated").0));
		}
		let mut spread = c_spread(params.iter().map(|p| &p.typ), ret, &self.types, arch).into_iter();
		for (i, &v) in vals.iter().enumerate() {
			let typ = params.get(i).map(|p| &p.typ);
			let by_val = spread.next().flatten();
			let (Some(Typ::Struct(_, fields)), Some(regs)) = (typ, by_val) else {
				out.push(v);
				continue;
			};
			let c = self.stack_slot(c_layout(fields, &self.types).expect("validated").size.next_multiple_of(8));
			self.copy_fields(v, c, 0, fields, true);
			for (at, p) in regs {
				out.push(at.map_or(c, |at| self.b.ins().load(p.value_type, MemFlags::trusted(), c, at)));
			}
		}
		out
	}

	// Gather a foreign call's `@c` struct result from its registers or buffer.
	pub(super) fn c_ret(&mut self, call: codegen::ir::Inst, typ: &Typ) -> Option<Value> {
		let regs = typ.c_by_val(&self.types, self.module.isa().triple().architecture);
		let vals = match regs.as_deref()? {
			[(None, _)] => vec![self.b.func.dfg.inst_variable_args(call)[0]],
			_ => self.b.inst_results(call).to_vec(),
		};
		self.c_gather(typ, regs, &vals).map(|(oi, _)| oi)
	}

	// Scatter a C ABI fn's `@c` struct result into its registers or the caller's buffer.
	pub(super) fn c_rets(&mut self, val: Value, typ: &Typ) -> Option<Vec<Value>> {
		let arch = self.module.isa().triple().architecture;
		let Typ::Struct(_, fields) = typ else { return None };
		let regs = typ.c_by_val(&self.types, arch).filter(|_| self.c_abi)?;
		let sret = self.b.func.special_param(ArgumentPurpose::StructReturn);
		let buf = sret.unwrap_or_else(|| {
			self.stack_slot(c_layout(fields, &self.types).expect("validated").size.next_multiple_of(8))
		});
		self.copy_fields(val, buf, 0, fields, true);
		let regs = regs.into_iter().filter_map(|(at, p)| Some((at?, p.value_type)));
		let loads = regs.map(|(at, cl)| self.b.ins().load(cl, MemFlags::trusted(), buf, at));
		Some(loads.collect())
	}

	// Convert a C ABI fn's params to Oi values, gathering each `@c` struct from its ABI params.
	pub(crate) fn c_params(&mut self, params: &[(String, Typ, Access)], vals: &[Value]) -> Vec<Value> {
		let arch = self.module.isa().triple().architecture;
		let ret = self.ret.clone().map_or(Typ::unit(), |r| r.0);
		let spread = c_spread(params.iter().map(|(_, t, _)| t), &ret, &self.types, arch);
		let sret = self.b.func.signature.uses_special_param(ArgumentPurpose::StructReturn);
		let mut vals = &vals[sret as usize..];
		let mut out = vec![];
		for ((_, typ, _), regs) in params.iter().zip(spread) {
			let (val, used) = self.c_gather(typ, regs, vals).unwrap_or_else(|| (self.c_norm(vals[0], typ), 1));
			out.push(val);
			vals = &vals[used..];
		}
		out
	}

	// Rebuild a `@c` struct from its leading ABI values, registers at their offsets or a pointer to its copy.
	fn c_gather(&mut self, typ: &Typ, regs: Option<CRegs>, vals: &[Value]) -> Option<(Value, usize)> {
		let Typ::Struct(_, fields) = typ else { return None };
		let regs = regs?;
		let c = match regs[..] {
			[(None, _)] => vals[0],
			_ => self.stack_slot(c_layout(fields, &self.types).expect("validated").size.next_multiple_of(8)),
		};
		for (&v, at) in vals.iter().zip(regs.iter().filter_map(|r| r.0)) {
			self.b.ins().store(MemFlags::trusted(), v, c, at);
		}
		let oi = self.zero(typ);
		self.copy_fields(oi, c, 0, fields, false);
		Some((oi, regs.len()))
	}

	// A `ptr` cast to a fn type.
	pub(super) fn cast_fn_ptr(&mut self, addr: Value, typ: &Typ, span: Span) -> Result<TypedVal, Diagnostic> {
		self.require_unsafe(&format!("{typ} cast"), span)?;
		let (sig, bare) = match typ {
			Typ::Annotated(_, inner) => (&**inner, true),
			t => (t, false),
		};
		let Typ::Fn(params, ret) = sig else {
			return fail(format!("cannot cast core::ptr to {typ}"), span, "no conversion");
		};
		check_c_sig(self.types, &typ.to_string(), params, ret, span)?;
		if !bare && let Some(p) = params.iter().find(|p| matches!(p.typ, Typ::Fn(..))) {
			let msg = format!("`{typ}` can't take a fn pointer");
			let label = format!("`{}` would cross as a cell", p.typ);
			return fail(msg, span, label);
		}
		let val = if bare { addr } else { self.fn_cell(addr) };
		Ok((val, typ.clone()))
	}

	// Box a bare fn pointer as an Oi fn value.
	pub(crate) fn fn_cell(&mut self, addr: Value) -> Value {
		let zero = self.b.ins().iconst(self.int, 0);
		let cell = self.rc_alloc(8, &[zero]);
		self.st(cell, 0, addr);
		cell
	}

	fn oi_struct(&self, typ: &Typ) -> bool {
		matches!(typ, Typ::Struct(name, _) if !is_c_struct(self.types.consts.anns, name))
	}

	fn c_fields(&self, typ: &Typ, span: Span) -> Result<Option<Vec<FieldDef>>, Diagnostic> {
		match typ {
			Typ::Struct(name, fields) if is_c_struct(self.types.consts.anns, name) => Ok(Some(fields.clone())),
			t if t.is_c_repr(&self.types) && !matches!(t, Typ::Fn(..)) => Ok(None),
			_ => fail(
				format!("`{typ}` has no C layout"),
				span,
				"only a struct or C scalar crosses a `ptr`",
			),
		}
	}

	// bools are a byte in C but a word in Oi.
	fn c_load(&mut self, typ: &Typ, c: Value, off: i32) -> Value {
		match typ.newtype().unwrap_or(typ) {
			t if let Some(n) = t.bool_bytes() => {
				let raw = self.load_elem(c, off, &Typ::UInt(n * 8));
				self.intcast(raw, self.int, false)
			}
			t => self.load_elem(c, off, t),
		}
	}

	// Extend an enum from C, whose upper bits are undefined.
	pub(crate) fn c_norm(&mut self, v: Value, typ: &Typ) -> Value {
		let Some(b) = typ.c_enum(&self.types) else { return v };
		let w = cl_type(&b, self.int);
		let v = if self.b.func.dfg.value_type(v) == w {
			v
		} else {
			self.b.ins().ireduce(w, v)
		};
		match b {
			Typ::UInt(_) => self.b.ins().uextend(self.int, v),
			_ => self.b.ins().sextend(self.int, v),
		}
	}

	fn c_store(&mut self, typ: &Typ, v: Value, c: Value, off: i32) {
		match typ.newtype().unwrap_or(typ) {
			t if let Some(n) = t.bool_bytes() => self.store_elem(c, off, &Typ::UInt(n * 8), v),
			t => self.store_elem(c, off, t, v),
		}
	}

	// Copy each field between its Oi slot and its C offset.
	fn copy_fields(&mut self, oi: Value, c: Value, at: i32, fields: &[FieldDef], to_c: bool) {
		let offsets = c_layout(fields, &self.types).expect("validated").offsets;
		for ((i, f), off) in fields.iter().enumerate().zip(offsets) {
			let (slot, off) = ((i * 8) as i32, at + off as i32);
			match f.typ.newtype().unwrap_or(&f.typ) {
				Typ::Struct(_, inner) => {
					// nested structs are inline in C, behind a pointer in Oi
					let child = self.ld_word(oi, slot);
					self.copy_fields(child, c, off, inner, to_c);
				}
				Typ::FixedArray(e, n) => {
					let (at, buf) = (self.b.ins().iadd_imm(c, off as i64), self.ld_word(oi, slot));
					let (dst, src) = if to_c { (at, buf) } else { (buf, at) };
					self.fixed_move(dst, src, e, *n, true);
				}
				typ if to_c => {
					let v = self.ld_typ(oi, slot, typ);
					self.c_store(typ, v, c, off);
				}
				typ => {
					let v = self.c_load(typ, c, off);
					self.st(oi, slot, v);
				}
			}
		}
	}
}
