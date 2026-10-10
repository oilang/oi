use super::*;

impl<'a, M: Module> Translator<'a, M> {
	// array handle: { data @ 0, len @ 8, cap @ 16 }
	pub(super) fn array_data(&mut self, header: Value) -> Value {
		self.ld_word(header, 0)
	}

	pub(super) fn array_len(&mut self, header: Value) -> Value {
		self.ld_word(header, 8)
	}

	pub(super) fn array_cap(&mut self, header: Value) -> Value {
		self.ld_word(header, 16)
	}

	// Build a fresh array handle, owned by the enclosing scope.
	pub(super) fn make_array(&mut self, data: Value, len: Value, typ: &Typ) -> Value {
		let header = self.heap_slots(&[data, len, len]);
		self.temp(header, typ);
		header
	}

	// Check each element against `want`, collecting values.
	fn collect_elems(
		&mut self,
		elems: &[Spanned<Expr>],
		want: Option<&Typ>,
	) -> Result<(Vec<Value>, Option<Typ>), Diagnostic> {
		let mut elem = want.cloned();
		let mut vals = Vec::with_capacity(elems.len());
		for e in elems {
			let (val, typ) = match &elem {
				Some(t) => self.check_expr(e, t)?,
				None => self.expr(e)?,
			};
			closure_escape(&typ, e.1.into_range(), "stored in an array")?;
			self.move_resource(e, &typ)?;
			unify_elem(&mut elem, &typ, e.1)?;
			vals.push(val);
		}
		Ok((vals, elem))
	}

	fn spread_lit(&mut self, elems: &[Spanned<Expr>], want: Option<&Typ>, span: Span) -> Result<TypedVal, Diagnostic> {
		let (mut elem, mut parts, mut start) = (want.cloned(), Vec::new(), 0);
		for i in 0..=elems.len() {
			let inner = match elems.get(i) {
				Some((Expr::Spread(inner), _)) => Some(inner),
				Some(_) => continue,
				None => None,
			};
			if start < i {
				let (val, typ) = self.array_lit(&elems[start..i], elem.as_ref(), span)?;
				elem = Some(array_elem(&typ).clone());
				parts.push(val);
			}
			start = i + 1;
			let Some(inner) = inner else { break };
			let (val, typ) = self.expr(inner)?;
			if let Typ::Int(_) = typ {
				let (val, rtyp) = self.upto(val, inner.1)?;
				unify_elem(&mut elem, &rtyp, inner.1)?;
				let (data, len) = self.heap_alloc(vec![val], &rtyp);
				parts.push(self.make_array(data, len, &Typ::Array(Box::new(rtyp))));
				continue;
			}
			let (val, typ) = self.collect_spread((val, typ), inner.1)?;
			let (Typ::Array(t) | Typ::FixedArray(t, _)) = &typ else {
				return fail(format!("cannot spread {typ}"), inner.1, "not an array");
			};
			unify_elem(&mut elem, t, inner.1)?;
			parts.push(match &typ {
				Typ::FixedArray(_, n) => self.fixed_to_array(val, t, *n),
				_ => val,
			});
		}
		let elem = elem.expect("a spread sets the element type");
		let typ = Typ::Array(Box::new(elem.clone()));
		if let ([part], [(Expr::Spread(_), _)]) = (&parts[..], elems) {
			return Ok((*part, typ));
		}
		let (data, len) = self.heap_alloc(Vec::new(), &elem);
		let out = self.make_array(data, len, &typ);
		for part in parts {
			self.extend_array(out, part, &elem);
		}
		Ok((out, typ))
	}

	// Panic if `cond`.
	pub(super) fn trap_if(&mut self, cond: Value, msg: &str, span: Span) -> Result<(), Diagnostic> {
		let (bad, ok) = self.fork(cond);

		self.b.switch_to_block(bad);
		let msg = self.str_const(msg);
		self.ctx_panic("panic", msg, span)?;

		self.b.switch_to_block(ok);
		Ok(())
	}

	// Spreading a non-array collects it first, if its type knows how.
	pub(super) fn collect_spread(&mut self, (val, typ): TypedVal, span: Span) -> Result<TypedVal, Diagnostic> {
		let Some(sig) = self.funcs.get(&format!("{typ}.collect")).cloned() else {
			return Ok((val, typ));
		};
		if is_range(&typ) {
			let (.., open) = self.range_parts(val);
			self.trap_if(open, "cannot spread an open range", span)?;
		}
		Ok(self.emit_call(&sig, &[val]))
	}

	// Copy each value into `base` at its stride-sized slot.
	fn store_all(&mut self, base: Value, vals: Vec<Value>, elem: &Typ) {
		let stride = self.elem_stride(elem);
		for (i, val) in vals.into_iter().enumerate() {
			let val = self.copy_in(val, elem);
			self.store_elem(base, (i as i64 * stride) as i32, elem, val);
		}
	}

	// Fresh rc'd heap buffer holding `vals`.
	fn heap_alloc(&mut self, vals: Vec<Value>, elem: &Typ) -> (Value, Value) {
		let n = vals.len();
		let data = self.rc_alloc(n as i64 * self.elem_stride(elem), &[]);
		self.store_all(data, vals, elem);
		(data, self.b.ins().iconst(self.int, n as i64))
	}

	// Copy `n` elements of foreign memory into an owned array.
	pub(super) fn ptr_array(
		&mut self,
		recv: Option<Value>,
		type_args: &[Spanned<TypeExpr>],
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		self.require_unsafe("ptr.array", span)?;
		let (Some(ptr), [(te, te_span)], [count]) = (recv, type_args, args) else {
			return fail(
				"`array` takes one type argument and a length",
				span,
				"write `p.array[T](n)`",
			);
		};
		let elem = self.types.resolve(te, *te_span)?;
		let n = self.int_value(count, "length")?;
		let n = self.intcast(n, self.int, true);
		let stride = self.elem_stride(&elem);
		let bytes = self.b.ins().imul_imm(n, stride);
		let data = self.rt_call("ptr_buffer", &[ptr, bytes]).unwrap();
		let typ = Typ::Array(Box::new(elem));
		Ok((self.make_array(data, n, &typ), typ))
	}

	// Build an array literal.
	pub(super) fn array_lit(
		&mut self,
		elems: &[Spanned<Expr>],
		want: Option<&Typ>,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if elems.iter().any(|e| matches!(e.0, Expr::Spread(_))) {
			return self.spread_lit(elems, want, span);
		}
		let (vals, elem) = self.collect_elems(elems, want)?;
		let Some(elem) = elem else {
			return fail(
				"empty array literals aren't supported yet",
				span,
				"needs at least one element to infer its type",
			);
		};
		let (data, len) = self.heap_alloc(vals, &elem);
		let typ = Typ::Array(Box::new(elem));
		Ok((self.make_array(data, len, &typ), typ))
	}

	// Build a fixed-size array literal of `want` (element, length), or inferred from its elements.
	pub(super) fn fixed_lit(
		&mut self,
		elems: &[Spanned<Expr>],
		want: Option<(&Typ, usize)>,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if let Some((_, n)) = want
			&& elems.len() != n
		{
			let msg = format!("expected {n} elements, got {}", elems.len());
			return fail(msg, span, "wrong number of elements");
		}
		let (vals, elem) = self.collect_elems(elems, want.map(|w| w.0))?;
		let Some(elem) = elem else {
			return fail(
				"cannot infer the element type here",
				span,
				"needs at least one element to infer its type",
			);
		};
		let n = vals.len();
		let ptr = self.stack_slot((n as i64 * self.elem_stride(&elem)) as u32);
		self.store_all(ptr, vals, &elem);
		let typ = Typ::FixedArray(Box::new(elem), n);
		self.temp(ptr, &typ);
		Ok((ptr, typ))
	}

	// Autocast a fixed array into a fresh rc'd dynamic buffer.
	pub(super) fn fixed_to_array(&mut self, ptr: Value, elem: &Typ, n: usize) -> Value {
		let stride = self.elem_stride(elem);
		let vals = (0..n).map(|i| self.load_elem(ptr, (i as i64 * stride) as i32, elem)).collect();
		let (data, len) = self.heap_alloc(vals, elem);
		let typ = Typ::Array(Box::new(elem.clone()));
		self.make_array(data, len, &typ)
	}

	// Copy-in point for rc'd handles.
	// RC bump.
	// The underlying buffer clone waits for a write.
	pub(super) fn copy_in(&mut self, val: Value, typ: &Typ) -> Value {
		if let Typ::Struct(_, fields) = typ {
			let heap = self.call_alloc(fields.len());
			return self.copy_struct(val, heap, typ, fields);
		}
		// a fixed array's buffer is inline, so a copy must escape the frame with its owner
		if let Typ::FixedArray(elem, n) = typ {
			let (elem, n) = ((**elem).clone(), *n);
			let heap = self.call_alloc_bytes(n as i64 * self.elem_stride(&elem));
			return self.fixed_copy(val, heap, &elem, n);
		}
		// move resource to new owner
		if self.handover(val, typ) {
			self.untemp(val);
		}
		if self.is_affine(typ) {
			return val;
		}
		if let Typ::Tuple(fields) = typ
			&& !fields.is_empty()
		{
			let vals: Vec<_> = (fields.iter().enumerate())
				.map(|(i, (_, t))| {
					let v = self.ld_typ(val, (i * 8) as i32, t);
					self.copy_in(v, t)
				})
				.collect();
			return self.heap_slots(&vals);
		}
		if self.enum_box(typ) {
			// a fresh box moves
			if self.temps.contains_key(&val) {
				self.untemp(val);
				return val;
			}
			let words: Vec<_> = (0..enum_slots(&self.variants_of(typ)) as i32)
				.map(|i| self.ld_word(val, i * 8))
				.collect();
			let dst = self.heap_slots(&words);
			self.owned_payloads(val, typ, |s, off, t| {
				let pv = s.ld_typ(val, off, t);
				let pv = s.copy_in(pv, t);
				s.st(dst, off, pv);
			});
			return dst;
		}
		let Some((share, _)) = rc::handle_fns(typ) else {
			return val;
		};
		self.rt_call(share, &[val]).unwrap()
	}

	// Copy a struct's fields into `dst`, settling ownership.
	pub(super) fn copy_struct(&mut self, val: Value, dst: Value, typ: &Typ, fields: &[FieldDef]) -> Value {
		self.assign_fields(val, dst, fields, false);
		self.settle(val, dst, typ);
		dst
	}

	// Clone the buffer before a write if it's shared.
	pub(super) fn cow_array(&mut self, header: Value, elem: &Typ) {
		let size = self.stride_val(elem);
		let n = self.rt_call("array_cow", &[header, size]).unwrap();
		let zero = self.b.ins().iconst(self.int, 0);
		self.elems_rc(header, zero, n, elem, true);
	}

	// Append an owned value, growing the buffer when full.
	pub(super) fn push(&mut self, header: Value, val: Value, elem: &Typ) {
		let (len, cap, stride) = (self.array_len(header), self.array_cap(header), self.elem_stride(elem));
		let full = self.b.ins().icmp(IntCC::Equal, len, cap);
		let (grow, ok) = (self.b.create_block(), self.b.create_block());
		self.b.ins().brif(full, grow, &[], ok, &[]);
		self.b.seal_block(grow);

		self.b.switch_to_block(grow);
		let (min_cap, size) = (self.b.ins().iadd_imm(len, 1), self.b.ins().iconst(self.int, stride));
		self.rt_call("array_reserve", &[header, min_cap, size]);
		self.b.ins().jump(ok, &[]);
		self.b.seal_block(ok);

		self.b.switch_to_block(ok);
		let (data, len) = (self.array_data(header), self.array_len(header));
		let off = self.b.ins().imul_imm(len, stride);
		let addr = self.b.ins().iadd(data, off);
		self.store_elem(addr, 0, elem, val);
		let new_len = self.b.ins().iadd_imm(len, 1);
		self.st(header, 8, new_len);
	}

	// Append `src` to `dst`, retaining the copied elements.
	pub(super) fn extend_array(&mut self, dst: Value, src: Value, elem: &Typ) {
		let (lo, n, size) = (self.array_len(dst), self.array_len(src), self.stride_val(elem));
		self.rt_call("array_extend", &[dst, src, size]);
		self.elems_rc(dst, lo, n, elem, true);
	}

	// Retain every element of a fresh copy.
	pub(super) fn owning(&mut self, header: Value, elem: &Typ) -> Value {
		let (zero, n) = (self.b.ins().iconst(self.int, 0), self.array_len(header));
		self.elems_rc(header, zero, n, elem, true);
		header
	}

	// Lower slice bounds, defaulting to `0..len`.
	pub(super) fn slice_bounds(
		&mut self,
		range: Option<&Spanned<Expr>>,
		len: Value,
	) -> Result<(Value, Value), Diagnostic> {
		let (start, end, inclusive) = range.and_then(|r| r.0.bounds()).unwrap_or_default();
		if start.is_some_and(|e| matches!(e.0, Expr::Range { .. })) {
			return fail(
				"strided views aren't supported yet",
				start.unwrap().1,
				"an array slice has no step",
			);
		}
		let lo = match start {
			Some(e) => {
				let v = self.int_value(e, "slice start")?;
				self.intcast(v, self.int, true)
			}
			None => self.b.ins().iconst(self.int, 0),
		};
		let hi = match end {
			Some(e) => {
				let v = self.int_value(e, "slice end")?;
				let v = self.intcast(v, self.int, true);
				self.b.ins().iadd_imm(v, inclusive as i64)
			}
			None => len,
		};
		Ok((lo, hi))
	}

	// Slice an already-evaluated array operand into a fresh copy.
	pub(super) fn slice_copy(
		&mut self,
		(ptr, typ): TypedVal,
		span: Span,
		range: Option<&Spanned<Expr>>,
	) -> Result<(Value, Value, Typ), Diagnostic> {
		match typ {
			Typ::Array(_) => {}
			Typ::FixedArray(..) => {
				return fail(
					"slicing fixed arrays is not supported yet",
					span,
					"only dynamic arrays can be sliced for now",
				);
			}
			_ => {
				return fail(format!("cannot slice {typ}"), span, "not an array");
			}
		}
		let elem = array_elem(&typ).clone();
		let len = self.array_len(ptr);
		let (lo, hi) = self.slice_bounds(range, len)?;
		let size = self.stride_val(&elem);
		let out = self.rt_call("slice", &[ptr, lo, hi, size]).unwrap();
		Ok((self.owning(out, &elem), lo, elem))
	}

	pub(super) fn range_slice(
		&mut self,
		(ptr, typ): TypedVal,
		range: Value,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if !matches!(typ, Typ::Array(_) | Typ::Str) {
			return fail(format!("cannot slice {typ}"), span, "not an array");
		}
		let (lo, end, step, open) = self.range_parts(range);
		let strided = self.b.ins().icmp_imm(IntCC::NotEqual, step, 1);
		self.trap_if(strided, "strided views aren't supported yet", span)?;
		let lo = self.intcast(lo, self.int, true);
		let end = self.intcast(end, self.int, true);
		let len = self.array_len(ptr);
		let hi = self.b.ins().select(open, len, end);
		if typ == Typ::Str {
			return Ok((self.rt_call("str_slice", &[ptr, lo, hi]).unwrap(), Typ::Str));
		}
		let elem = array_elem(&typ).clone();
		let size = self.stride_val(&elem);
		let out = self.rt_call("slice", &[ptr, lo, hi, size]).unwrap();
		let out = self.owning(out, &elem);
		let typ = Typ::Array(Box::new(elem));
		self.temp(out, &typ);
		Ok((out, typ))
	}

	// (data pointer, length) for an array.
	pub(super) fn array_parts(&mut self, val: Value, typ: &Typ) -> (Value, Value) {
		match typ {
			Typ::FixedArray(_, n) => (val, self.b.ins().iconst(self.int, *n as i64)),
			_ => (self.array_data(val), self.array_len(val)),
		}
	}

	pub(super) fn int_value(&mut self, e: &Spanned<Expr>, what: &str) -> Result<Value, Diagnostic> {
		let (v, t) = self.expr(e)?;
		if !matches!(t, Typ::Int(_)) {
			return fail(format!("{what} must be Int, got {t}"), e.1, "not an Int");
		}
		Ok(v)
	}

	pub(super) fn bool_value(&mut self, e: &Spanned<Expr>, what: &str) -> Result<Value, Diagnostic> {
		let (v, t) = self.expr(e)?;
		if t != Typ::Bool {
			return fail(format!("{what} must be Bool, got {t}"), e.1, "not a Bool");
		}
		Ok(v)
	}

	// Bounds-check `idx` and return the element address.
	pub(super) fn elem_addr(&mut self, data: Value, len: Value, elem: &Typ, idx: Value, span: Span) -> Value {
		let oob = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, len);

		let (panic_block, ok_block) = self.fork(oob);

		self.b.switch_to_block(panic_block);
		let ctx = self.ctx_value(crate::compiler::CONTEXT);
		let (at, _) = self.src_lit(span).unwrap_or_else(|_| unreachable!("`Src` always resolves"));
		self.rt_call("panic_oob", &[ctx, idx, len, at]);
		self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);

		self.b.switch_to_block(ok_block);
		let stride = self.elem_stride(elem);
		let off = self.b.ins().imul_imm(idx, stride);
		self.b.ins().iadd(data, off)
	}

	pub(super) fn load_index(&mut self, data: Value, len: Value, elem: &Typ, idx: Value, span: Span) -> Value {
		let addr = self.elem_addr(data, len, elem, idx, span);
		self.load_elem(addr, 0, elem)
	}

	pub(super) fn store_index(&mut self, data: Value, len: Value, typ: &Typ, idx: Value, val: Value, span: Span) {
		let elem = array_elem(typ);
		let addr = self.elem_addr(data, len, elem, idx, span);
		if self.slot_owns(elem) {
			let old = self.load_elem(addr, 0, elem);
			self.release_field(old, elem);
		}
		self.store_elem(addr, 0, elem, val);
	}

	// An element's in-memory type.
	fn elem_mem(&self, elem: &Typ) -> Typ {
		match elem {
			Typ::Enum(_) => self.variants_of(elem).first().and_then(|v| v.backing.clone()),
			_ => None,
		}
		.unwrap_or_else(|| elem.clone())
	}

	pub(super) fn elem_stride(&self, elem: &Typ) -> i64 {
		elem_size(&self.elem_mem(elem))
	}

	// An element's stride as a runtime arg.
	pub(super) fn stride_val(&mut self, elem: &Typ) -> Value {
		let stride = self.elem_stride(elem);
		self.b.ins().iconst(self.int, stride)
	}

	pub(super) fn load_elem(&mut self, addr: Value, off: i32, elem: &Typ) -> Value {
		let mem = self.elem_mem(elem);
		let v = self.ld_typ(addr, off, &mem);
		self.intcast(v, cl_type(elem, self.int), matches!(mem, Typ::Int(_)))
	}

	pub(super) fn store_elem(&mut self, addr: Value, off: i32, elem: &Typ, val: Value) {
		let mem = self.elem_mem(elem);
		let val = self.intcast(val, cl_type(&mem, self.int), matches!(mem, Typ::Int(_)));
		self.st(addr, off, val);
	}

	// Call fn for each element.
	pub(super) fn each_elem(&mut self, val: Value, typ: &Typ, mut f: impl FnMut(&mut Self, Value, Value)) {
		let (data, len) = self.array_parts(val, typ);
		let elem = array_elem(typ);
		self.repeat(len, |s, iv| {
			let ev = s.load_nth(data, iv, elem);
			f(s, iv, ev)
		});
	}

	// Call fn with each index below `len`.
	pub(super) fn repeat(&mut self, len: Value, mut f: impl FnMut(&mut Self, Value)) {
		let (head, body, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
		let i = self.b.declare_var(self.int);
		let zero = self.b.ins().iconst(self.int, 0);
		self.b.def_var(i, zero);
		self.b.ins().jump(head, &[]);

		self.b.switch_to_block(head);
		let iv = self.b.use_var(i);
		let more = self.b.ins().icmp(IntCC::SignedLessThan, iv, len);
		self.b.ins().brif(more, body, &[], exit, &[]);
		self.b.seal_block(body);
		self.b.seal_block(exit);

		self.b.switch_to_block(body);
		f(self, iv);
		let next = self.b.ins().iadd_imm(iv, 1);
		self.b.def_var(i, next);
		self.b.ins().jump(head, &[]);
		self.b.seal_block(head);

		self.b.switch_to_block(exit);
	}

	// nth element of a raw data pointer.
	pub(super) fn load_nth(&mut self, data: Value, idx: Value, elem: &Typ) -> Value {
		let stride = self.elem_stride(elem);
		let off = self.b.ins().imul_imm(idx, stride);
		let addr = self.b.ins().iadd(data, off);
		self.load_elem(addr, 0, elem)
	}
}

// Try to fold one more element type into the running one.
fn unify_elem(elem: &mut Option<Typ>, found: &Typ, span: Span) -> Result<(), Diagnostic> {
	match elem {
		Some(t) if t != found => {
			let msg = format!("array elements must share a type: expected {t}, got {found}");
			fail(msg, span, "mismatched element type")
		}
		_ => {
			*elem = Some(found.clone());
			Ok(())
		}
	}
}
