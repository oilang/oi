use super::*;

impl<'a, M: Module> Translator<'a, M> {
	// array handle: { data @ 0, len @ 8, cap @ 16 }
	pub(super) fn array_data(&mut self, header: Value) -> Value {
		self.b.ins().load(self.int, MemFlags::new(), header, 0)
	}

	pub(super) fn array_len(&mut self, header: Value) -> Value {
		self.b.ins().load(self.int, MemFlags::new(), header, 8)
	}

	pub(super) fn array_cap(&mut self, header: Value) -> Value {
		self.b.ins().load(self.int, MemFlags::new(), header, 16)
	}

	// Build a fresh array handle, owned by the enclosing scope.
	pub(super) fn make_array(&mut self, data: Value, len: Value, typ: &Typ) -> Value {
		let header = self.call_alloc(3);
		self.b.ins().store(MemFlags::new(), data, header, 0);
		self.b.ins().store(MemFlags::new(), len, header, 8);
		self.b.ins().store(MemFlags::new(), len, header, 16);
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
				let (zero, one) = (self.b.ins().iconst(types::I32, 0), self.b.ins().iconst(types::I32, 1));
				let (val, rtyp) = self.make_range(zero, Some(val), one, inner.1)?;
				unify_elem(&mut elem, &rtyp, inner.1)?;
				let (data, len) = self.heap_alloc(vec![val], &rtyp);
				parts.push(self.make_array(data, len, &Typ::Array(Box::new(rtyp))));
				continue;
			}
			let (val, typ) = self.collect_spread((val, typ));
			let (Typ::Array(t) | Typ::FixedArray(t, _)) = &typ else {
				return Err(
					Diagnostic::new(format!("cannot spread {typ}"), inner.1.into_range()).with_label("not an array")
				);
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
		let size = self.elem_stride(&elem);
		let size = self.b.ins().iconst(self.int, size);
		for part in parts {
			self.rt_call("array_extend", &[out, part, size]);
		}
		Ok((out, typ))
	}

	// Panic if `cond`.
	pub(super) fn trap_if(&mut self, cond: Value, msg: &str) {
		let (bad, ok) = (self.b.create_block(), self.b.create_block());
		self.b.ins().brif(cond, bad, &[], ok, &[]);
		self.b.seal_block(bad);
		self.b.seal_block(ok);

		self.b.switch_to_block(bad);
		let msg = self.str_const(msg);
		self.rt_call("panic", &[msg]);
		self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);

		self.b.switch_to_block(ok);
	}

	// Spreading a non-array collects it first, if its type knows how.
	pub(super) fn collect_spread(&mut self, (val, typ): TypedVal) -> TypedVal {
		let Some(sig) = self.funcs.get(&format!("{typ}.collect")).cloned() else {
			return (val, typ);
		};
		if is_range(&typ) {
			let (.., open) = self.range_parts(val);
			self.trap_if(open, "cannot spread an open range");
		}
		self.emit_call(&sig, &[val])
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
		let base = self.call_alloc_bytes(n as i64 * self.elem_stride(elem) + 8);
		let one = self.b.ins().iconst(self.int, 1);
		self.b.ins().store(MemFlags::new(), one, base, 0);
		let data = self.b.ins().iadd_imm(base, 8);
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
			return Err(
				Diagnostic::new("`array` takes one type argument and a length", span.into_range())
					.with_label("write `p.array[T](n)`"),
			);
		};
		let elem = self.types().resolve(te, *te_span)?;
		let n = self.int_value(count, "length")?;
		let n = self.b.ins().sextend(self.int, n);
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
			return Err(
				Diagnostic::new("empty array literals aren't supported yet", span.into_range())
					.with_label("needs at least one element to infer its type"),
			);
		};
		let (data, len) = self.heap_alloc(vals, &elem);
		let typ = Typ::Array(Box::new(elem));
		Ok((self.make_array(data, len, &typ), typ))
	}

	// Build a fixed-size array literal.
	pub(super) fn fixed_lit(
		&mut self,
		elems: &[Spanned<Expr>],
		elem: &Typ,
		n: usize,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if elems.len() != n {
			let msg = format!("expected {n} elements, got {}", elems.len());
			return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of elements"));
		}
		let (vals, elem) = self.collect_elems(elems, Some(elem))?;
		let elem = elem.expect("want given");
		let ptr = self.stack_slot((n as i64 * self.elem_stride(&elem)) as u32);
		self.store_all(ptr, vals, &elem);
		Ok((ptr, Typ::FixedArray(Box::new(elem), n)))
	}

	// Infer a fixed array from its elements.
	pub(super) fn fixed_infer(&mut self, elems: &[Spanned<Expr>], span: Span) -> Result<TypedVal, Diagnostic> {
		let (vals, elem) = self.collect_elems(elems, None)?;
		let Some(elem) = elem else {
			return Err(Diagnostic::new("cannot infer the element type here", span.into_range())
				.with_label("needs at least one element to infer its type"));
		};
		let n = vals.len();
		let ptr = self.stack_slot((n as i64 * self.elem_stride(&elem)) as u32);
		self.store_all(ptr, vals, &elem);
		Ok((ptr, Typ::FixedArray(Box::new(elem), n)))
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
			let fields = fields.clone();
			let heap = self.call_alloc(fields.len());
			self.assign_fields(val, heap, &fields, false);
			self.settle(val, heap, typ);
			return heap;
		}
		// move resource to new owner
		if self.handover(val, typ) {
			self.untemp(val);
		}
		if self.is_affine(typ) {
			return val;
		}
		let Some((share, _)) = rc::handle_fns(typ) else {
			return val;
		};
		self.rt_call(share, &[val]).unwrap()
	}

	// Clone the buffer before a write if it's shared.
	pub(super) fn cow_array(&mut self, header: Value, elem: &Typ) {
		let stride = self.elem_stride(elem);
		let size = self.b.ins().iconst(self.int, stride);
		self.rt_call("array_cow", &[header, size]);
	}

	// Lower slice bounds, defaulting to `0..len`.
	pub(super) fn slice_bounds(
		&mut self,
		range: Option<&Spanned<Expr>>,
		len: Value,
	) -> Result<(Value, Value), Diagnostic> {
		let (start, end, inclusive) = range.and_then(|r| r.0.bounds()).unwrap_or_default();
		if start.is_some_and(|e| matches!(e.0, Expr::Range { .. })) {
			return Err(
				Diagnostic::new("strided views aren't supported yet", start.unwrap().1.into_range())
					.with_label("an array slice has no step"),
			);
		}
		let lo = match start {
			Some(e) => {
				let v = self.int_value(e, "slice start")?;
				self.b.ins().sextend(self.int, v)
			}
			None => self.b.ins().iconst(self.int, 0),
		};
		let hi = match end {
			Some(e) => {
				let v = self.int_value(e, "slice end")?;
				let v = self.b.ins().sextend(self.int, v);
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
				return Err(
					Diagnostic::new("slicing fixed arrays is not supported yet", span.into_range())
						.with_label("only dynamic arrays can be sliced for now"),
				);
			}
			_ => {
				return Err(
					Diagnostic::new(format!("cannot slice {typ}"), span.into_range()).with_label("not an array")
				);
			}
		}
		let elem = array_elem(&typ).clone();
		let len = self.array_len(ptr);
		let (lo, hi) = self.slice_bounds(range, len)?;
		let stride = self.elem_stride(&elem);
		let size = self.b.ins().iconst(self.int, stride);
		Ok((self.rt_call("slice", &[ptr, lo, hi, size]).unwrap(), lo, elem))
	}

	pub(super) fn range_slice(
		&mut self,
		(ptr, typ): TypedVal,
		range: Value,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if !matches!(typ, Typ::Array(_) | Typ::Str) {
			return Err(Diagnostic::new(format!("cannot slice {typ}"), span.into_range()).with_label("not an array"));
		}
		let (lo, end, step, open) = self.range_parts(range);
		let strided = self.b.ins().icmp_imm(IntCC::NotEqual, step, 1);
		self.trap_if(strided, "strided views aren't supported yet");
		let lo = self.b.ins().sextend(self.int, lo);
		let end = self.b.ins().sextend(self.int, end);
		let len = self.array_len(ptr);
		let hi = self.b.ins().select(open, len, end);
		if typ == Typ::Str {
			return Ok((self.rt_call("str_slice", &[ptr, lo, hi]).unwrap(), Typ::Str));
		}
		let elem = array_elem(&typ).clone();
		let stride = self.elem_stride(&elem);
		let size = self.b.ins().iconst(self.int, stride);
		let out = self.rt_call("slice", &[ptr, lo, hi, size]).unwrap();
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
			return Err(
				Diagnostic::new(format!("{what} must be Int, got {t}"), e.1.into_range()).with_label("not an Int"),
			);
		}
		Ok(v)
	}

	// Bounds-check `idx` and return the element address.
	pub(super) fn elem_addr(&mut self, data: Value, len: Value, elem: &Typ, idx: Value) -> Value {
		let oob = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, len);

		let panic_block = self.b.create_block();
		let ok_block = self.b.create_block();
		self.b.ins().brif(oob, panic_block, &[], ok_block, &[]);
		self.b.seal_block(panic_block);
		self.b.seal_block(ok_block);

		self.b.switch_to_block(panic_block);
		self.rt_call("panic_oob", &[idx, len]);
		self.b.ins().trap(TrapCode::HEAP_OUT_OF_BOUNDS);

		self.b.switch_to_block(ok_block);
		let stride = self.elem_stride(elem);
		let off = self.b.ins().imul_imm(idx, stride);
		self.b.ins().iadd(data, off)
	}

	pub(super) fn load_index(&mut self, data: Value, len: Value, elem: &Typ, idx: Value) -> Value {
		let addr = self.elem_addr(data, len, elem, idx);
		self.load_elem(addr, 0, elem)
	}

	pub(super) fn store_index(&mut self, data: Value, len: Value, elem: &Typ, idx: Value, val: Value) {
		let addr = self.elem_addr(data, len, elem, idx);
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

	pub(super) fn load_elem(&mut self, addr: Value, off: i32, elem: &Typ) -> Value {
		let mem = self.elem_mem(elem);
		let v = self.b.ins().load(cl_type(&mem, self.int), MemFlags::new(), addr, off);
		self.intcast(v, cl_type(elem, self.int), matches!(mem, Typ::Int(_)))
	}

	pub(super) fn store_elem(&mut self, addr: Value, off: i32, elem: &Typ, val: Value) {
		let mem = self.elem_mem(elem);
		let val = self.intcast(val, cl_type(&mem, self.int), matches!(mem, Typ::Int(_)));
		self.b.ins().store(MemFlags::new(), val, addr, off);
	}

	// Call fn for each element.
	pub(super) fn each_elem(&mut self, val: Value, typ: &Typ, mut f: impl FnMut(&mut Self, Value, Value)) {
		let (data, len) = self.array_parts(val, typ);
		let elem = array_elem(typ);
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
		let ev = self.load_nth(data, iv, elem);
		f(self, iv, ev);
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
			Err(Diagnostic::new(msg, span.into_range()).with_label("mismatched element type"))
		}
		_ => {
			*elem = Some(found.clone());
			Ok(())
		}
	}
}
