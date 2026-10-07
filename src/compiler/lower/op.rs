use crate::compiler::role;

use super::*;

// What `emit_eq` compares directly.
fn comparable(t: &Typ) -> bool {
	use Typ::*;
	t.is_enumish() && *t != Any
		|| matches!(
			t,
			Int(_) | UInt(_) | ISize | USize | Bool | Rune | Atom | Float(_) | Str | Error
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
	t.is_enumish() || eq_slots(t).is_some() || matches!(t, Typ::Array(_) | Typ::FixedArray(..) | Typ::Map(..))
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
			let same = self.b.ins().icmp_imm(IntCC::Equal, ta, v.disc);
			let hit = self.b.ins().band(tags_eq, same);
			let (body, next) = self.fork(hit);
			self.b.switch_to_block(body);
			let all = self.slots_eq(a, b, 8, &v.payload, &owner, span)?;
			self.b.def_var(eq, all);
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
		if t.nominal().is_some()
			&& let Some(sig) = self.fill(t, role::EQ, "eq", 2)
		{
			return Ok(self.emit_call(&sig, &[a, b]).0);
		}
		if let Some(inner) = t.newtype().cloned() {
			// a newtype compares as its inner value
			return self.emit_val_eq(a, b, &inner, owner, span);
		}
		match t {
			t if let Some(slots) = eq_slots(t) => self.slots_eq(a, b, 0, &slots, &t.to_string(), span),
			Typ::Array(_) | Typ::FixedArray(..) => self.emit_array_eq(a, b, t, span),
			Typ::Map(..) => self.emit_map_eq(a, b, t, span),
			t if t.is_enumish() && enum_boxed(&self.variants_of(t)) && !rc::opt_niche(t) => {
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
	pub(super) fn fill(&mut self, typ: &Typ, tn: &str, method: &str, arity: usize) -> Option<FnSig> {
		let name = typ.key();
		let found = match self.trait_impls.contains(&(name.clone(), tn.to_string())) {
			true => self.funcs.get(&format!("{name}.{method}")).cloned(),
			false if self.claims(typ, tn) => self.recv_instance(&format!("{}.{method}", rc::base_name(&name)), typ),
			false => None,
		};
		found.filter(|s| s.params.len() == arity)
	}

	// Inside its own methods, a newtype without a trait claim operates on its field.
	pub(super) fn own_field<'t>(&self, t: &'t Typ, tn: &str) -> &'t Typ {
		match t {
			Typ::TupleStruct(n, _) if self.self_type.as_ref() == Some(n) && !self.claims(t, tn) => {
				t.newtype().unwrap_or(t)
			}
			_ => t,
		}
	}

	// Contains claim.
	pub(super) fn emit_contains(
		&mut self,
		c: Value,
		ct: &Typ,
		(v, vt): TypedVal,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let Some(sig) = self.find_fill(&format!("{}.contains", ct.key()), 1, &vt) else {
			return Err(
				Diagnostic::new(format!("cannot search {vt} in {ct}"), span.into_range())
					.with_label(format!("`{ct}` claims no `Contains[{vt}]`")),
			);
		};
		Ok(self.emit_call(&sig, &[c, v]))
	}

	// Whether `typ` claims `tn`.
	pub(super) fn claims(&self, typ: &Typ, tn: &str) -> bool {
		let key = typ.key();
		let generic = self.generic_claims.get(&(rc::base_name(&key).to_string(), tn.to_string()));
		self.trait_impls.contains(&(key.clone(), tn.to_string()))
			|| generic.is_some_and(|tps| {
				let args = self.types.generics.instance_args(&key).unwrap_or_default();
				(tps.iter().zip(&args)).all(|(p, a)| p.bound.as_ref().is_none_or(|b| self.claims(a, b)))
			}) || (self.core_traits.contains(tn) && builtin_claim(typ, tn))
	}

	// Compare two heap blocks slot by slot, from byte `base`.
	fn slots_eq(
		&mut self,
		a: Value,
		b: Value,
		base: i32,
		slots: &[Typ],
		owner: &str,
		span: Span,
	) -> Result<Value, Diagnostic> {
		let mut acc = self.b.ins().iconst(types::I8, 1);
		for (i, st) in slots.iter().enumerate() {
			let fa = self.ld_typ(a, base + (i * 8) as i32, st);
			let fb = self.ld_typ(b, base + (i * 8) as i32, st);
			let eq = self.emit_val_eq(fa, fb, st, owner, span)?;
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

	// Check whether every key/value pair is equal between two maps.
	fn emit_map_eq(&mut self, a: Value, b: Value, typ: &Typ, span: Span) -> Result<Value, Diagnostic> {
		let Typ::Map(k, v) = typ else {
			unreachable!("emit_map_eq on {typ}")
		};
		let (owner, kt) = (typ.to_string(), Typ::Array(k.clone()));
		let tag = map_key_tag(k).expect("map keys are taggable");
		let (la, lb) = (
			self.rt_call("map_len", &[a]).unwrap(),
			self.rt_call("map_len", &[b]).unwrap(),
		);
		let seed = self.b.ins().icmp(IntCC::Equal, la, lb);
		let keys = self.map_entries(a, true, k);
		self.temp(keys, &kt);

		let (data, n) = self.array_parts(keys, &kt);
		let slot = self.stack_slot(8);

		self.emit_scan(n, true, seed, |s, i| {
			let key = s.load_nth(data, i, k);
			let bits = s.map_bits(key);
			let ga = s.map_rt("get", a, tag, bits, &[]);
			s.st(slot, 0, ga);

			// writes b's value into `slot`, returning whether the key was there at all
			let hit = s.map_rt("find", b, tag, bits, &[slot]);
			let gb = s.ld_word(slot, 0);
			let (va, vb) = (s.unmap_bits(ga, v), s.unmap_bits(gb, v));
			let same = s.emit_val_eq(va, vb, v, &owner, span)?;
			let same = s.b.ins().icmp_imm(IntCC::NotEqual, same, 0);
			let hit = s.b.ins().icmp_imm(IntCC::NotEqual, hit, 0);
			Ok(s.b.ins().band(same, hit))
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

	// Wrap `val` back to its declared bit width.
	pub(super) fn narrow(&mut self, val: Value, typ: &Typ) -> Value {
		match typ {
			Typ::Int(w) => self.reduce_int(val, *w),
			Typ::UInt(w) => self.reduce_uint(val, *w),
			_ => val,
		}
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
		rhint: impl FnOnce(&mut Self, &Typ) -> Typ,
	) -> Result<(TypedVal, TypedVal), Diagnostic> {
		if l.0.anon() && !r.0.anon() {
			let rhs = self.expr(r)?;
			return Ok((self.check_expr(l, &rhs.1)?, rhs));
		}
		let lhs = self.expr(l)?;
		let hint = rhint(self, &lhs.1);
		let rhs = self.check_expr(r, &hint)?;
		Ok((lhs, rhs))
	}

	// An expression known at comptime to be a plain int.
	fn const_int(&self, e: &Spanned<Expr>) -> Option<i64> {
		match &e.0 {
			Expr::Int(n) => Some(*n),
			Expr::Negative(inner) => self.const_int(inner).map(i64::wrapping_neg),
			Expr::Ident(name) => match self.types.consts.map.get(self.qualify(name).as_ref())? {
				(Expr::Int(n), _) => Some(*n),
				_ => None,
			},
			_ => None,
		}
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
			BinOp::BitAnd => (role::BIT_AND, "bitand"),
			BinOp::BitOr => (role::BIT_OR, "bitor"),
			BinOp::BitXor => (role::BIT_XOR, "bitxor"),
			BinOp::Shl => (role::SHL, "shl"),
			BinOp::Shr => (role::SHR, "shr"),
			_ => unreachable!("non-arithmetic op in binop"),
		};
		let ((lv, lt), (rv, rt)) = self.operands(l, r, |s, lt| {
			(lt.nominal().and_then(|_| s.fill(lt, tn, method, 2))).map_or(lt.clone(), |sig| sig.params[1].typ.clone())
		})?;
		let own = [&lt, &rt].into_iter().find(|t| self.own_field(t, tn) != *t).cloned();
		let (lt, rt) = (self.own_field(&lt, tn).clone(), self.own_field(&rt, tn).clone());

		if let Some(name) = lt.nominal() {
			// overloads
			let key = format!("{name}.{method}");
			let plain = self.fill(&lt, tn, method, 2);
			let picked = (plain.clone().filter(|s| s.params[1].typ == rt))
				.or_else(|| self.find_fill(&key, 1, &rt))
				.or(plain);
			let Some(sig) = picked else {
				let label = match self.funcs.keys().any(|k| k.starts_with(&format!("{key}#"))) {
					true => format!("`{name}` claims no `{tn}[{rt}]`"),
					false => format!("implement `{tn}` for `{name}` to overload `{op}`"),
				};
				return Err(
					Diagnostic::new(format!("cannot apply `{op}` to {lt}"), span.into_range()).with_label(label),
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
			&& let Some(name) = rt.nominal()
			&& let Some(sig) = (self.fill(&rt, tn, method, 2).filter(|s| s.params[1].typ == lt))
				.or_else(|| self.find_fill(&format!("{name}.{method}"), 1, &lt))
		{
			return Ok(self.emit_call(&sig, &[rv, lv]));
		}

		// string concatenation
		if let (BinOp::Add, Typ::Str, Typ::Str) = (op, &lt, &rt) {
			return Ok((self.call_concat(lv, rv), own.unwrap_or(Typ::Str)));
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
		if let NumKind::Float = kind
			&& matches!(
				op,
				BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr
			) {
			return Err(
				Diagnostic::new(format!("cannot apply `{op}` to {lt}"), span.into_range())
					.with_label("bitwise operators need integer operands"),
			);
		}
		if let (BinOp::Shl | BinOp::Shr, Some(n)) = (op, self.const_int(r)) {
			let width = match &lt {
				Typ::Int(w) | Typ::UInt(w) => *w as i64,
				_ => 64, // isize/usize
			};
			// cranelift apparently masks a runtime shift count by the container width, so reject comptime-known counts outside that range
			if !(0..width).contains(&n) {
				return Err(Diagnostic::new(
					format!("shift count {n} is out of range for {lt} ({width} bits)"),
					r.1.into_range(),
				)
				.with_label(format!("must be between 0 and {}", width - 1)));
			}
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
			(BinOp::BitAnd, _) => b.band(lv, rv),
			(BinOp::BitOr, _) => b.bor(lv, rv),
			(BinOp::BitXor, _) => b.bxor(lv, rv),
			(BinOp::Shl, _) => b.ishl(lv, rv),
			(BinOp::Shr, NumKind::UInt) => b.ushr(lv, rv),
			(BinOp::Shr, _) => b.sshr(lv, rv),
			_ => unreachable!("non-arithmetic op in binop"),
		};
		let out = self.narrow(out, &lt);
		Ok((out, own.unwrap_or(lt)))
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
			| (Typ::Rune, Typ::Rune)
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
				} else if l.nominal().is_some()
					&& let Some(sig) = self.fill(l, role::ORD, "lt", 2)
				{
					let (a, b) = if reversed { (rv, lv) } else { (lv, rv) };
					let less = self.emit_call(&sig, &[a, b]).0;
					self.b.ins().icmp_imm(cc, less, 0)
				} else if l.is_enumish() && (!enum_boxed(&self.variants_of(l)) || rc::opt_niche(l)) {
					self.b.ins().icmp(icc, lv, rv)
				} else {
					let claimable = matches!(l, Typ::Struct(..)) || matches!(l, Typ::Enum(n) if sugar(n).is_none());
					let label = match claimable {
						true => format!("claim `Ord` for `{lt}` to define ordering"),
						false => "only `==` and `!=` are supported".into(),
					};
					return Err(
						Diagnostic::new(format!("cannot compare {lt} and {rt}"), span.into_range()).with_label(label),
					);
				}
			}
			(Typ::Float(_), Typ::Float(_)) => self.b.ins().fcmp(fcc, lv, rv),
			(Typ::Str, Typ::Str) | (Typ::Error, Typ::Error) | (Typ::Ast, Typ::Str) | (Typ::Str, Typ::Ast)
				if matches!(icc, IntCC::Equal | IntCC::NotEqual) =>
			{
				let eq = match (&lt, &rt) {
					(Typ::Ast, _) => self.ast_method(lv, "==", Some(rv)),
					(_, Typ::Ast) => self.ast_method(rv, "==", Some(lv)),
					_ => self.emit_eq(lv, rv, &lt),
				};
				let ne_cc = if icc == IntCC::Equal {
					IntCC::NotEqual
				} else {
					IntCC::Equal
				};
				self.b.ins().icmp_imm(ne_cc, eq, 0)
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

		if self.claims(&rhs_typ, role::CONTAINS) {
			let v = self.expr(lhs)?;
			return self.emit_contains(rhs_val, &rhs_typ, v, lhs.1);
		}

		let elem = match rhs_typ {
			Typ::Array(ref e) => (**e).clone(),
			_ => {
				return Err(Diagnostic::new(
					format!("right side of `in` must be an array, Str or a `Contains` type, got {rhs_typ}"),
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
		// scoped, so a temp the rhs allocates is released here
		let (rv, rt) = self.block_expr(std::slice::from_ref(r), r.1)?;
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
