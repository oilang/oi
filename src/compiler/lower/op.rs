use crate::compiler::role;

use super::*;

// What `emit_eq` compares directly.
fn comparable(t: &Typ) -> bool {
	use Typ::*;
	t.is_enumish() && *t != Any
		|| matches!(
			t,
			Int(_) | UInt(_) | ISize | USize | Bool | Atom | Float(_) | Str | Error
		)
}

// Get the pointer-sized slots of a heap block.
fn eq_slots(t: &Typ) -> Option<Vec<Typ>> {
	match t {
		Typ::Struct(_, fields) => Some(fields.iter().map(|f| f.typ.clone()).collect()),
		Typ::Tuple(fields) | Typ::TupleStruct(_, fields) => Some(fields.iter().map(|(_, t)| t.clone()).collect()),
		_ => None,
	}
}

// Whether a type should compare structurally, rather than comparing bits.
fn eq_dispatchable(t: &Typ) -> bool {
	t.is_enumish() || eq_slots(t).is_some() || matches!(t, Typ::Array(_) | Typ::FixedArray(..))
}

impl<'a, M: Module> Translator<'a, M> {
	pub(super) fn emit_eq(&mut self, a: Value, b: Value, typ: &Typ) -> Value {
		match typ {
			Typ::Float(_) => self.b.ins().fcmp(FloatCC::Equal, a, b),
			Typ::Error => {
				let (a, b) = (self.error_message(a), self.error_message(b));
				self.emit_eq(a, b, &Typ::Str)
			}
			Typ::Str => self.rt_call("str_eq", &[a, b]).unwrap(),
			_ => self.b.ins().icmp(IntCC::Equal, a, b),
		}
	}

	// Compare two boxed enums.
	// Checks that tags match, and for the hit variant every payload slot matches.
	pub(super) fn emit_enum_eq(&mut self, a: Value, b: Value, typ: &Typ, span: Span) -> Result<Value, Diagnostic> {
		let variants = self.variants_of(typ);
		let owner = typ.to_string();
		let ta = self.enum_tag(typ, a);
		let tb = self.enum_tag(typ, b);
		let tags_eq = self.b.ins().icmp(IntCC::Equal, ta, tb);
		let eq = self.b.declare_var(types::I8);
		self.b.def_var(eq, tags_eq);
		let merge = self.b.create_block();
		for v in variants.iter().filter(|v| !v.payload.is_empty()) {
			let disc = self.b.ins().iconst(self.int, v.disc);
			let same = self.b.ins().icmp(IntCC::Equal, ta, disc);
			let hit = self.b.ins().band(tags_eq, same);
			let (body, next) = (self.b.create_block(), self.b.create_block());
			self.b.ins().brif(hit, body, &[], next, &[]);
			self.b.seal_block(body);
			self.b.seal_block(next);
			self.b.switch_to_block(body);
			for (i, ft) in v.payload.iter().enumerate() {
				let fa = self
					.b
					.ins()
					.load(cl_type(ft, self.int), MemFlags::new(), a, ((i + 1) * 8) as i32);
				let fb = self
					.b
					.ins()
					.load(cl_type(ft, self.int), MemFlags::new(), b, ((i + 1) * 8) as i32);
				let fe = self.emit_val_eq(fa, fb, ft, &owner, span)?;
				let fe = self.b.ins().icmp_imm(IntCC::NotEqual, fe, 0);
				let prev = self.b.use_var(eq);
				let acc = self.b.ins().band(prev, fe);
				self.b.def_var(eq, acc);
			}
			self.b.ins().jump(merge, &[]);
			self.b.switch_to_block(next);
		}
		self.b.ins().jump(merge, &[]);
		self.b.switch_to_block(merge);
		self.b.seal_block(merge);
		Ok(self.b.use_var(eq))
	}

	// `Eq` fill, or structural diff by default.
	fn emit_val_eq(&mut self, a: Value, b: Value, t: &Typ, owner: &str, span: Span) -> Result<Value, Diagnostic> {
		if let Typ::Struct(n, _) | Typ::TupleStruct(n, _) | Typ::Enum(n) = t
			&& let Some(sig) = self.fill(n, role::EQ, "eq", 2)
		{
			return Ok(self.emit_call(&sig, &[a, b]).0);
		}
		if let Some(inner) = t.newtype().cloned() {
			// a newtype compares as its inner value
			return self.emit_val_eq(a, b, &inner, owner, span);
		}
		match t {
			t if eq_slots(t).is_some() => self.emit_slots_eq(a, b, t, span),
			Typ::Array(_) | Typ::FixedArray(..) => self.emit_array_eq(a, b, t, span),
			t if t.is_enumish() && enum_boxed(&self.variants_of(t)) && !rc::opt_ref(t) => {
				self.emit_enum_eq(a, b, t, span)
			}
			t if comparable(t) => Ok(self.emit_eq(a, b, t)),
			t => Err(
				Diagnostic::new(format!("cannot compare {owner}: contains {t}"), span.into_range())
					.with_label(format!("claim `Eq` for `{owner}` to define equality")),
			),
		}
	}

	// The `method` fill of trait `tn` claimed for `name`, if any.
	pub(super) fn fill(&self, name: &str, tn: &str, method: &str, arity: usize) -> Option<FnSig> {
		let claimed = self.trait_impls.contains(&(name.to_string(), tn.to_string()));
		self.funcs
			.get(&format!("{name}.{method}"))
			.cloned()
			.filter(|s| claimed && s.params.len() == arity)
	}

	// Whether `typ` claims `tn`.
	pub(super) fn claims(&self, typ: &Typ, tn: &str) -> bool {
		let key = typ.key();
		self.trait_impls.contains(&(key.clone(), tn.to_string()))
			|| self.generic_claims.contains(&(rc::base_name(&key).to_string(), tn.to_string()))
			|| (self.core_traits.contains(tn) && builtin_claim(typ, tn))
	}

	// Compare two heap blocks slot by slot.
	fn emit_slots_eq(&mut self, a: Value, b: Value, typ: &Typ, span: Span) -> Result<Value, Diagnostic> {
		let Some(slots) = eq_slots(typ) else {
			unreachable!("emit_slots_eq on {typ}")
		};
		let owner = typ.to_string();
		let mut acc = self.b.ins().iconst(types::I8, 1);
		for (i, st) in slots.iter().enumerate() {
			let cl = cl_type(st, self.int);
			let fa = self.b.ins().load(cl, MemFlags::new(), a, (i * 8) as i32);
			let fb = self.b.ins().load(cl, MemFlags::new(), b, (i * 8) as i32);
			let eq = self.emit_val_eq(fa, fb, st, &owner, span)?;
			let eq = self.b.ins().icmp_imm(IntCC::NotEqual, eq, 0);
			acc = self.b.ins().band(acc, eq);
		}
		Ok(acc)
	}

	// Fold body over range, starting at `seed` and stopping once it leaves `all`.
	fn emit_scan(
		&mut self,
		len: Value,
		all: bool,
		seed: Value,
		body: impl FnOnce(&mut Self, Value) -> Result<Value, Diagnostic>,
	) -> Result<Value, Diagnostic> {
		let acc = self.b.declare_var(types::I8);
		let i = self.b.declare_var(self.int);
		let z = self.b.ins().iconst(self.int, 0);
		self.b.def_var(acc, seed);
		self.b.def_var(i, z);
		let (head, work, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
		self.b.ins().jump(head, &[]);

		self.b.switch_to_block(head);
		let (iv, av) = (self.b.use_var(i), self.b.use_var(acc));
		let more = self.b.ins().icmp(IntCC::SignedLessThan, iv, len);
		let open = self.b.ins().icmp_imm(IntCC::Equal, av, all as i64);
		let go = self.b.ins().band(more, open);
		self.b.ins().brif(go, work, &[], exit, &[]);
		self.b.seal_block(work);

		self.b.switch_to_block(work);
		let iv = self.b.use_var(i);
		let v = body(self, iv)?;
		let v = self.b.ins().icmp_imm(IntCC::NotEqual, v, 0);
		self.b.def_var(acc, v);
		let next = self.b.ins().iadd_imm(iv, 1);
		self.b.def_var(i, next);
		self.b.ins().jump(head, &[]);
		self.b.seal_block(head);

		self.b.switch_to_block(exit);
		self.b.seal_block(exit);
		Ok(self.b.use_var(acc))
	}

	// Check whether every element is equal between two arrays.
	fn emit_array_eq(&mut self, a: Value, b: Value, typ: &Typ, span: Span) -> Result<Value, Diagnostic> {
		let (elem, owner) = (array_elem(typ).clone(), typ.to_string());
		let ((da, la), (db, lb)) = (self.array_parts(a, typ), self.array_parts(b, typ));
		let seed = self.b.ins().icmp(IntCC::Equal, la, lb);
		self.emit_scan(la, true, seed, |s, i| {
			let (ea, eb) = (s.load_nth(da, i, &elem), s.load_nth(db, i, &elem));
			s.emit_val_eq(ea, eb, &elem, &owner, span)
		})
	}

	// Cast int-like to int-like.
	// ref: https://github.com/rust-lang/rustc_codegen_cranelift/blob/main/src/cast.rs
	pub(super) fn intcast(&mut self, val: Value, to: types::Type, signed: bool) -> Value {
		let from = self.b.func.dfg.value_type(val);
		if from == to {
			val
		} else if from.bits() > to.bits() {
			self.b.ins().ireduce(to, val)
		} else if signed {
			self.b.ins().sextend(to, val)
		} else {
			self.b.ins().uextend(to, val)
		}
	}

	// Sign-extend the low `w` bits of `val` within its container.
	// NOTE: noop for standard cranelift widths (8, 16, 32, 64).
	pub(super) fn reduce_int(&mut self, val: Value, w: u16) -> Value {
		let cl = cl_type(&Typ::Int(w), self.int);
		let shift = cl.bits() as i64 - w as i64;
		if shift == 0 {
			return val;
		}
		let shift_v = self.b.ins().iconst(cl, shift);
		let up = self.b.ins().ishl(val, shift_v);
		self.b.ins().sshr(up, shift_v)
	}

	// Zero-extend (mask) `val` to exactly `w` bits within its Cranelift container.
	pub(super) fn reduce_uint(&mut self, val: Value, w: u16) -> Value {
		let cl = cl_type(&Typ::UInt(w), self.int);
		if cl.bits() as u16 == w {
			return val;
		}
		let mask = ((1u64 << w) - 1) as i64;
		let mask_v = self.b.ins().iconst(cl, mask);
		self.b.ins().band(val, mask_v)
	}

	// Promote ints to larger-width ints and floats.
	fn promote(&mut self, lv: Value, lt: Typ, rv: Value, rt: Typ) -> (Value, Typ, Value, Typ) {
		let ints = |t: &Typ| matches!(t, Typ::Int(_) | Typ::ISize | Typ::UInt(_) | Typ::USize);
		let common = match (&lt, &rt) {
			(Typ::Int(a), Typ::Int(b)) => Typ::Int(*a.max(b)),
			(Typ::UInt(a), Typ::UInt(b)) => Typ::UInt(*a.max(b)),
			(Typ::Float(a), Typ::Float(b)) => Typ::Float(*a.max(b)),
			(Typ::Float(w), o) | (o, Typ::Float(w)) if ints(o) => Typ::Float(*w),
			_ => return (lv, lt, rv, rt),
		};
		let (lv, rv) = (self.numcast(lv, &lt, &common), self.numcast(rv, &rt, &common));
		(lv, common.clone(), rv, common)
	}

	// Cast a numeric value to a wider (or same) numeric type.
	fn numcast(&mut self, val: Value, from: &Typ, to: &Typ) -> Value {
		if from == to {
			return val;
		}
		let cl = cl_type(to, self.int);
		match (from, to) {
			(Typ::Float(_), _) => self.b.ins().fpromote(cl, val),
			(Typ::UInt(_) | Typ::USize, Typ::Float(_)) => self.b.ins().fcvt_from_uint(cl, val),
			(_, Typ::Float(_)) => self.b.ins().fcvt_from_sint(cl, val),
			(Typ::UInt(_), _) => self.intcast(val, cl, false),
			_ => self.intcast(val, cl, true),
		}
	}

	// Lower the hinted side first so an anon literal on the other side borrows its type.
	fn operands(
		&mut self,
		l: &Spanned<Expr>,
		r: &Spanned<Expr>,
		rhint: impl FnOnce(&Self, &Typ) -> Typ,
	) -> Result<(TypedVal, TypedVal), Diagnostic> {
		if l.0.anon() && !r.0.anon() {
			let rhs = self.expr(r)?;
			return Ok((self.check_expr(l, &rhs.1)?, rhs));
		}
		let lhs = self.expr(l)?;
		let rhs = self.check_expr(r, &rhint(self, &lhs.1))?;
		Ok((lhs, rhs))
	}

	pub(super) fn binop(
		&mut self,
		op: BinOp,
		l: &Spanned<Expr>,
		r: &Spanned<Expr>,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let (tn, method) = match op {
			BinOp::Add => (role::ADD, "add"),
			BinOp::Sub => (role::SUB, "sub"),
			BinOp::Mul => (role::MUL, "mul"),
			BinOp::Div => (role::DIV, "div"),
			BinOp::Mod => (role::MOD, "mod"),
			BinOp::Pow => (role::POW, "pow"),
			_ => unreachable!("non-arithmetic op in binop"),
		};
		let ((lv, lt), (rv, rt)) = self.operands(l, r, |s, lt| match lt {
			Typ::Struct(n, _) | Typ::Enum(n) => {
				s.fill(n, tn, method, 2).map_or(lt.clone(), |sig| sig.params[1].typ.clone())
			}
			_ => lt.clone(),
		})?;

		if let Typ::Struct(name, _) | Typ::Enum(name) = &lt {
			// overloads
			let Some(sig) = self.fill(name, tn, method, 2) else {
				return Err(
					Diagnostic::new(format!("cannot apply `{op}` to {lt}"), span.into_range())
						.with_label(format!("implement `{tn}` for `{name}` to overload `{op}`")),
				);
			};
			if rt != sig.params[1].typ {
				return Err(Diagnostic::new(
					format!("expected {} argument, got {rt}", sig.params[1].typ),
					r.1.into_range(),
				)
				.with_label("wrong argument type"));
			}
			return Ok(self.emit_call(&sig, &[lv, rv]));
		}

		// commutative operators
		if matches!(op, BinOp::Add | BinOp::Mul)
			&& matches!(lt, Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize | Typ::Float(_))
			&& let Typ::Struct(name, _) | Typ::Enum(name) = &rt
			&& let Some(sig) = self.fill(name, tn, method, 2)
			&& sig.params[1].typ == lt
		{
			return Ok(self.emit_call(&sig, &[rv, lv]));
		}

		// string concatenation
		if let (BinOp::Add, Typ::Str, Typ::Str) = (op, &lt, &rt) {
			return Ok((self.call_concat(lv, rv), Typ::Str));
		}
		let (lv, lt, rv, rt) = self.promote(lv, lt, rv, rt);

		#[derive(Clone, Copy)]
		enum NumKind {
			Int,
			UInt,
			Float,
		}
		let kind = match (&lt, &rt) {
			(Typ::Int(lw), Typ::Int(rw)) if lw == rw => NumKind::Int,
			(Typ::ISize, Typ::ISize) => NumKind::Int,
			(Typ::UInt(lw), Typ::UInt(rw)) if lw == rw => NumKind::UInt,
			(Typ::USize, Typ::USize) => NumKind::UInt,
			(Typ::Float(lw), Typ::Float(rw)) if lw == rw => NumKind::Float,
			_ => {
				return Err(
					Diagnostic::new(format!("cannot apply `{op}` to {lt} and {rt}"), span.into_range())
						.with_label("operands have mismatched types"),
				);
			}
		};
		if let (BinOp::Mod, NumKind::Float) = (op, kind) {
			// TODO: cranelift has no float remainder
			return Err(
				Diagnostic::new("`%` is not yet supported on floats".to_string(), span.into_range())
					.with_label("only integer operands"),
			);
		}
		// cranelift apparently has no pow instruction, so `**` widens to 64 bits and calls into the runtime
		let pow = matches!(op, BinOp::Pow).then(|| {
			let (name, wide, t) = match kind {
				NumKind::Float => ("pow_float", Typ::Float(64), types::F64),
				_ => ("pow_int", Typ::ISize, types::I64),
			};
			let (l, r) = (self.numcast(lv, &lt, &wide), self.numcast(rv, &rt, &wide));
			let out = self.rt_call(name, &[l, r]).unwrap();
			match cl_type(&lt, self.int) {
				cl if cl == t => out,
				cl if cl.is_float() => self.b.ins().fdemote(cl, out),
				cl => self.intcast(out, cl, matches!(kind, NumKind::Int)),
			}
		});
		let b = self.b.ins();
		let out = match (op, kind) {
			(BinOp::Pow, _) => pow.unwrap(),
			(BinOp::Add, NumKind::Float) => b.fadd(lv, rv),
			(BinOp::Add, _) => b.iadd(lv, rv),
			(BinOp::Sub, NumKind::Float) => b.fsub(lv, rv),
			(BinOp::Sub, _) => b.isub(lv, rv),
			(BinOp::Mul, NumKind::Float) => b.fmul(lv, rv),
			(BinOp::Mul, _) => b.imul(lv, rv),
			(BinOp::Div, NumKind::Float) => b.fdiv(lv, rv),
			(BinOp::Div, NumKind::UInt) => b.udiv(lv, rv),
			(BinOp::Div, NumKind::Int) => b.sdiv(lv, rv),
			(BinOp::Mod, NumKind::Float) => unreachable!("float `%` rejected above"),
			(BinOp::Mod, NumKind::UInt) => b.urem(lv, rv),
			(BinOp::Mod, NumKind::Int) => b.srem(lv, rv),
			_ => unreachable!("non-arithmetic op in binop"),
		};
		// For non-standard widths, wrap the result back to the declared bit width.
		let out = match &lt {
			Typ::Int(w) if cl_type(&Typ::Int(*w), self.int).bits() as u16 != *w => self.reduce_int(out, *w),
			Typ::UInt(w) if cl_type(&Typ::UInt(*w), self.int).bits() as u16 != *w => self.reduce_uint(out, *w),
			_ => out,
		};
		Ok((out, lt))
	}

	pub(super) fn cmp(
		&mut self,
		icc: IntCC,
		fcc: FloatCC,
		l: &Spanned<Expr>,
		r: &Spanned<Expr>,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let ((lv, lt), (rv, rt)) = self.operands(l, r, |_, t| t.clone())?;
		let (lv, lt, rv, rt) = self.promote(lv, lt, rv, rt);

		// () == ()
		if let (Typ::Tuple(lf), Typ::Tuple(rf)) = (&lt, &rt)
			&& lf.is_empty()
			&& rf.is_empty()
		{
			let result = match icc {
				IntCC::Equal => self.b.ins().iconst(self.int, 1),
				IntCC::NotEqual => self.b.ins().iconst(self.int, 0),
				_ => {
					return Err(
						Diagnostic::new("unit type `()` only supports `==` and `!=`", span.into_range())
							.with_label("unsupported comparison"),
					);
				}
			};
			return Ok((result, Typ::Bool));
		}

		let icc = if matches!((&lt, &rt), (Typ::UInt(_), Typ::UInt(_)) | (Typ::USize, Typ::USize)) {
			unsigned_cc(icc)
		} else {
			icc
		};
		let raw = match (&lt, &rt) {
			(Typ::Int(_), Typ::Int(_))
			| (Typ::UInt(_), Typ::UInt(_))
			| (Typ::ISize, Typ::ISize)
			| (Typ::USize, Typ::USize)
			| (Typ::Bool, Typ::Bool)
			| (Typ::Atom, Typ::Atom) => self.b.ins().icmp(icc, lv, rv),
			(l, _) if lt == rt && eq_dispatchable(l) => {
				let reversed = matches!(icc, IntCC::SignedGreaterThan | IntCC::SignedLessThanOrEqual);
				let negated = matches!(
					icc,
					IntCC::NotEqual | IntCC::SignedLessThanOrEqual | IntCC::SignedGreaterThanOrEqual
				);
				let cc = if negated { IntCC::Equal } else { IntCC::NotEqual };
				if let IntCC::Equal | IntCC::NotEqual = icc {
					let eq = self.emit_val_eq(lv, rv, l, &lt.to_string(), span)?;
					self.b.ins().icmp_imm(cc, eq, 0)
				} else if let Typ::Struct(n, _) | Typ::TupleStruct(n, _) | Typ::Enum(n) = l
					&& let Some(sig) = self.fill(n, role::ORD, "lt", 2)
				{
					let (a, b) = if reversed { (rv, lv) } else { (lv, rv) };
					let less = self.emit_call(&sig, &[a, b]).0;
					self.b.ins().icmp_imm(cc, less, 0)
				} else if l.is_enumish() && (!enum_boxed(&self.variants_of(l)) || rc::opt_ref(l)) {
					self.b.ins().icmp(icc, lv, rv)
				} else {
					let label = match l {
						Typ::Enum(_) | Typ::Struct(..) => format!("claim `Ord` for `{lt}` to define ordering"),
						_ => "only `==` and `!=` are supported".into(),
					};
					return Err(
						Diagnostic::new(format!("cannot compare {lt} and {rt}"), span.into_range()).with_label(label),
					);
				}
			}
			(Typ::Float(_), Typ::Float(_)) => self.b.ins().fcmp(fcc, lv, rv),
			(Typ::Str, Typ::Str) | (Typ::Error, Typ::Error) if icc == IntCC::Equal || icc == IntCC::NotEqual => {
				let typ = lt.clone();
				let eq = self.emit_eq(lv, rv, &typ);
				// emit_eq returns 1 for equal, invert for Ne
				// wrap in icmp so uextend below works consistently
				if icc == IntCC::NotEqual {
					self.b.ins().icmp_imm(IntCC::Equal, eq, 0)
				} else {
					self.b.ins().icmp_imm(IntCC::NotEqual, eq, 0)
				}
			}
			(Typ::Ast, Typ::Str) | (Typ::Str, Typ::Ast) if icc == IntCC::Equal || icc == IntCC::NotEqual => {
				let (ast_val, str_val) = if lt == Typ::Ast { (lv, rv) } else { (rv, lv) };
				let eq = self.ast_method(ast_val, "==", Some(str_val));
				if icc == IntCC::NotEqual {
					self.b.ins().icmp_imm(IntCC::Equal, eq, 0)
				} else {
					self.b.ins().icmp_imm(IntCC::NotEqual, eq, 0)
				}
			}
			_ => {
				return Err(
					Diagnostic::new(format!("cannot compare {lt} and {rt}"), span.into_range())
						.with_label("not comparable"),
				);
			}
		};
		let out = self.b.ins().uextend(self.int, raw);
		Ok((out, Typ::Bool))
	}

	// `lhs in rhs`.
	pub(super) fn in_op(&mut self, lhs: &Spanned<Expr>, rhs: &Spanned<Expr>) -> Result<TypedVal, Diagnostic> {
		let (rhs_val, rhs_typ) = self.expr(rhs)?;

		// substring
		if rhs_typ == Typ::Str {
			let (lhs_val, lhs_typ) = self.expr(lhs)?;
			if lhs_typ != Typ::Str {
				return Err(
					Diagnostic::new(format!("cannot search {lhs_typ} in Str"), lhs.1.into_range())
						.with_label("type mismatch: value must be Str"),
				);
			}
			let sig = self.funcs.get(role::STR_CONTAINS).cloned().ok_or_else(|| {
				Diagnostic::new(format!("core is missing `{}`", role::STR_CONTAINS), rhs.1.into_range())
					.with_label("required for `in` on Str")
			})?;
			return Ok(self.emit_call(&sig, &[rhs_val, lhs_val]));
		}

		if is_range(&rhs_typ) {
			let n = self.int_value(lhs, "`in` value")?;
			let n = self.intcast(n, types::I64, true);
			let hit = self.range_contains(rhs_val, n, rhs.1)?;
			return Ok((hit, Typ::Bool));
		}

		let elem = match rhs_typ {
			Typ::Array(ref e) => (**e).clone(),
			_ => {
				return Err(Diagnostic::new(
					format!("right side of `in` must be an array, Str or Range, got {rhs_typ}"),
					rhs.1.into_range(),
				)
				.with_label("not an array or string"));
			}
		};
		let (val, val_typ) = self.expr(lhs)?;
		if val_typ != elem {
			return Err(
				Diagnostic::new(format!("cannot search {val_typ} in {elem} array"), lhs.1.into_range())
					.with_label("type mismatch"),
			);
		}

		let len = self.array_len(rhs_val);
		let data = self.array_data(rhs_val);
		let no = self.b.ins().iconst(types::I8, 0);
		let found = self.emit_scan(len, false, no, |s, i| {
			let e = s.load_nth(data, i, &elem);
			s.emit_val_eq(val, e, &elem, &elem.to_string(), lhs.1)
		})?;
		Ok((self.b.ins().uextend(self.int, found), Typ::Bool))
	}

	// Short-circuits. `&&` only evaluates the right side when the left is true, and `||` does the inverse.
	pub(super) fn logical(&mut self, and: bool, l: &Spanned<Expr>, r: &Spanned<Expr>) -> Result<TypedVal, Diagnostic> {
		let (lv, lt) = self.expr(l)?;
		if lt != Typ::Bool {
			return Err(Diagnostic::new(format!("expected Bool, got {lt}"), l.1.into_range())
				.with_label("logical operators need Bool operands"));
		}

		// result defaults to the short-circuit value
		let result = self.b.declare_var(self.int);
		let short = self.b.ins().iconst(self.int, if and { 0 } else { 1 });
		self.b.def_var(result, short);

		let rhs_block = self.b.create_block();
		let merge = self.b.create_block();
		let (then, els) = if and { (rhs_block, merge) } else { (merge, rhs_block) };
		self.b.ins().brif(lv, then, &[], els, &[]);

		self.b.switch_to_block(rhs_block);
		self.b.seal_block(rhs_block);
		let (rv, rt) = self.expr(r)?;
		if rt != Typ::Bool {
			return Err(Diagnostic::new(format!("expected Bool, got {rt}"), r.1.into_range())
				.with_label("logical operators need Bool operands"));
		}
		self.b.def_var(result, rv);
		self.b.ins().jump(merge, &[]);

		self.b.switch_to_block(merge);
		self.b.seal_block(merge);
		Ok((self.b.use_var(result), Typ::Bool))
	}
}
