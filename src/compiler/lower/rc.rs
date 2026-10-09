use super::*;

// Ownership bookkeeping.
// Every ref has one owner: a named binding, a container slot, or the scope that produced it.
// Owned values register in the innermost scope and release when it exits.

// A `defer` body, re-lowered at every exit of its scope.
#[derive(Clone)]
pub(crate) struct Defer {
	pub(crate) body: Spanned<Expr>,
	pub(crate) vars: HashMap<String, Local>,
	pub(crate) when: When,
	pub(crate) armed: Option<Variable>,
}

impl<'a, M: Module> Translator<'a, M> {
	// Check whether a struct has Drop, or owns something that does.
	pub(super) fn is_resource(&self, typ: &Typ) -> bool {
		self.is_resource_seen(typ, &mut Vec::new())
	}

	fn is_resource_seen(&self, typ: &Typ, seen: &mut Vec<String>) -> bool {
		match typ {
			Typ::Struct(_, fields) => {
				self.claims(typ, "Drop") || fields.iter().any(|f| self.is_resource_seen(&f.typ, seen))
			}
			Typ::Array(elem) | Typ::FixedArray(elem, _) => self.is_resource_seen(elem, seen),
			Typ::Map(_, val) => self.is_resource_seen(val, seen),
			Typ::Tuple(fields) => fields.iter().any(|(_, t)| self.is_resource_seen(t, seen)),
			Typ::Enum(name) => {
				if seen.contains(name) {
					return false;
				}
				seen.push(name.clone());
				self.variants_of(typ)
					.iter()
					.any(|v| v.payload.iter().any(|t| self.is_resource_seen(t, seen)))
			}
			_ => false,
		}
	}

	// Whether a scope must release a value of this type.
	pub(super) fn needs_release(&self, typ: &Typ) -> bool {
		releasable(typ) || self.is_resource(typ)
	}

	// Check whether a resource copies through its hook instead of moving.
	pub(super) fn is_copy(&self, typ: &Typ) -> bool {
		match typ {
			Typ::Struct(_, fields) => {
				self.claims(typ, "Copy")
					|| (fields.iter().any(|f| self.is_copy(&f.typ)) && fields.iter().all(|f| !self.is_affine(&f.typ)))
			}
			Typ::Array(t) | Typ::Map(_, t) => self.is_copy(t),
			_ => false,
		}
	}

	// Whether a resource is move-only.
	pub(super) fn is_affine(&self, typ: &Typ) -> bool {
		self.is_resource(typ) && !self.is_copy(typ)
	}

	// Whether a value is a move.
	pub(super) fn handover(&self, val: Value, typ: &Typ) -> bool {
		self.is_affine(typ) || (self.is_copy(typ) && self.temps.contains_key(&val))
	}

	// Transfer ownership of a resource, if required.
	pub fn move_resource(&mut self, e: &Spanned<Expr>, typ: &Typ) -> Result<(), Diagnostic> {
		match self.is_affine(typ) {
			true => self.move_out(e, typ),
			false => Ok(()),
		}
	}

	// Run a type's Drop or Copy hook, if they exist.
	fn run_hook(&mut self, val: Value, typ: &Typ, hook: &str) {
		if let Some(sig) = self.resolve_method(typ, hook) {
			self.emit_call(&sig, &[val]);
		}
	}

	// Settle a struct copy.
	pub(super) fn settle(&mut self, val: Value, dst: Value, typ: &Typ) {
		if self.handover(val, typ) {
			self.untemp(val);
		} else if let Typ::Struct(..) = typ
			&& self.is_copy(typ)
		{
			self.run_hook(dst, typ, "copy");
		}
	}

	// Transfer ownership out of an expression.
	// Projections borrow.
	pub fn move_out(&mut self, e: &Spanned<Expr>, typ: &Typ) -> Result<(), Diagnostic> {
		match &e.0 {
			Expr::Ident(n) if n != "none" => {
				let local = self.local(n, e.1.into_range())?;
				self.move_local(n, &local, e.1.into_range())?;
			}
			Expr::Index { .. } | Expr::Slice { .. } | Expr::Field { .. } => {
				return fail(
					format!("cannot move {typ} out of its container"),
					e.1,
					"only an owned binding can be moved, so use it in place",
				);
			}
			_ => {}
		}
		Ok(())
	}

	// Hand a temp to a new owner.
	pub(super) fn untemp(&mut self, val: Value) {
		if let Some(var) = self.temps.remove(&val) {
			self.scopes.iter_mut().for_each(|s| s.retain(|(v, _)| *v != var));
		}
	}

	// Emit one release for an owned value.
	pub(super) fn release_value(&mut self, val: Value, typ: &Typ) {
		if let Some((_, release)) = handle_fns(typ) {
			if let Typ::Array(elem) = typ
				&& self.is_resource(elem)
			{
				self.each_elem(val, typ, |s, _, ev| s.release_value(ev, elem));
			}
			if let Typ::Map(_, v) = typ
				&& self.is_resource(v)
			{
				let vals = self.map_entries(val, false, v);
				self.release_value(vals, &Typ::Array(v.clone()));
			}
			self.rt_call(release, &[val]);
		} else if let Typ::FixedArray(elem, _) = typ {
			let elem = (**elem).clone();
			self.each_elem(val, typ, |s, _, ev| s.release_value(ev, &elem));
		} else if let Typ::Struct(_, fields) = typ {
			if self.is_resource(typ) {
				self.run_hook(val, typ, "drop");
			}
			self.release_slots(val, 0, &field_types(fields));
		} else if let Typ::Tuple(fields) = typ
			&& self.is_resource(typ)
		{
			self.release_slots(val, 0, &fields.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>());
		} else if matches!(typ, Typ::Enum(_)) && self.is_resource(typ) {
			let (tag, done) = (self.ld_word(val, 0), self.b.create_block());
			for v in self.variants_of(typ) {
				if !v.payload.iter().any(|t| self.needs_release(t)) {
					continue;
				}
				// release payloads under their own tag
				self.on_variant(tag, v.disc, done, |s| s.release_slots(val, 8, &v.payload));
			}
			self.b.ins().jump(done, &[]);
			self.b.seal_block(done);
			self.b.switch_to_block(done);
		}
	}

	// Release an owned struct field.
	pub(super) fn release_field(&mut self, val: Value, typ: &Typ) {
		self.release_value(val, typ);
		if let Typ::Struct(..) = typ {
			self.rt_call("free", &[val]);
		}
	}

	// Release the owned slots of an aggregate type.
	fn release_slots(&mut self, val: Value, base: i32, types: &[Typ]) {
		for (i, t) in types.iter().enumerate() {
			if owns(t) || self.is_resource(t) {
				let fv = self.ld_typ(val, base + (i * 8) as i32, t);
				self.release_field(fv, t);
			}
		}
	}

	// The address of a struct's trace descriptor symbol.
	fn trace_desc(&mut self, name: &str, slots: &[Typ]) -> Value {
		if self.desc_data(name, slots).is_none() {
			return self.b.ins().iconst(self.int, 0);
		}
		self.data_addr(&oi_symbol(&format!("{name}#trace")))
	}

	// Define trace descriptor on first use.
	fn desc_data(&mut self, name: &str, slots: &[Typ]) -> Option<DataId> {
		if let Some(&id) = self.out.descs.get(name) {
			return Some(id);
		}
		let mut words = vec![0i64];
		let mut relocs = Vec::new();
		for (i, t) in slots.iter().enumerate() {
			// kinds: 0 ref, 1 nested struct + description, 2 array, 3 map
			let off = ((i * 8) as i64) << 2;
			if let Some((_, release)) = handle_fns(t) {
				words.push(
					off | match release {
						"array_release" => 2,
						"map_release" => 3,
						_ => 0,
					},
				);
			} else if let Typ::Struct(n, sub) = t
				&& let Some(child) = self.desc_data(n, &field_types(sub))
			{
				words.push(off | 1);
				relocs.push((words.len() * 8, child));
				words.push(0);
			}
		}
		words[0] = (words.len() - 1 - relocs.len()) as i64;
		if words[0] == 0 {
			return None;
		}
		let mut desc = DataDescription::new();
		// TODO: get this offset dynamically like rustc does
		desc.set_align(8);
		desc.define(words.iter().flat_map(|w| w.to_le_bytes()).collect());
		for (off, child) in relocs {
			let gv = self.module.declare_data_in_data(child, &mut desc);
			desc.write_data_addr(off as u32, gv, 0);
		}
		let sym = oi_symbol(&format!("{name}#trace"));
		let id = self
			.module
			.declare_data(&sym, Linkage::Local, false, false)
			.expect("declare trace");
		self.module.define_data(id, &desc).expect("define trace");
		self.out.descs.insert(name.to_string(), id);
		Some(id)
	}

	// Register a producer's fresh handle with the innermost scope.
	pub(super) fn temp(&mut self, val: Value, typ: &Typ) {
		if self.needs_release(typ) {
			let var = self.b.declare_var(self.int);
			self.b.def_var(var, val);
			self.temps.insert(val, var);
			self.own_local(var, typ);
		}
	}

	// Move a value into a fresh rc box, a non-struct T as its slot.
	pub(super) fn box_value(&mut self, ptr: Value, typ: &Typ) -> Value {
		let (key, slots) = match typ {
			Typ::Struct(name, fields) => (name.clone(), field_types(fields)),
			t => (t.key(), vec![t.clone()]),
		};
		let descv = self.trace_desc(&key, &slots);
		let boxp = self.rc_alloc((slots.len() * 8) as i64, &[descv]);
		for i in 0..slots.len() as i32 {
			let v = match typ {
				Typ::Struct(..) => self.ld_word(ptr, i * 8),
				_ => ptr,
			};
			self.st(boxp, i * 8, v);
		}
		boxp
	}

	// An rc'd `any`, with typeid and payload.
	pub(super) fn any_box(&mut self, id: Value, val: Value, typ: &Typ) -> Value {
		let desc = self.trace_desc(&format!("any {}", typ.key()), &[Typ::ISize, typ.clone()]);
		let boxp = self.rc_alloc(16, &[desc]);
		let val = self.copy_in(val, typ);
		self.store_slots(boxp, &[id, val]);
		self.temp(boxp, &Typ::Any);
		boxp
	}

	// A fresh rc'd block of `bytes`.
	pub(super) fn rc_alloc(&mut self, bytes: i64, head: &[Value]) -> Value {
		let words = head.len() as i64 + 1;
		let base = self.call_alloc_bytes(bytes + words * 8);
		let one = self.b.ins().iconst(self.int, 1);
		self.store_slots(base, &[head, &[one]].concat());
		self.b.ins().iadd_imm(base, words * 8)
	}

	// Declare a named binding that owns its value.
	pub fn bind_local(&mut self, name: &str, val: Value, typ: Typ, mutable: bool) {
		let addr = mutable && self.addressed.contains(name);
		let (val, owned) = match addr {
			true => (self.box_value(val, &typ), Typ::Ref(Box::new(typ.clone()))),
			false => (val, typ.clone()),
		};
		let var = self.b.declare_var(self.b.func.dfg.value_type(val));
		self.b.def_var(var, val);
		self.own_local(var, &owned);
		if addr {
			self.aliases.push(var);
		}
		let mut local = Local::plain(var, typ, mutable);
		local.boxed = addr && !matches!(local.typ, Typ::Struct(..));
		self.vars.insert(name.to_string(), local);
	}

	// Make the innermost scope responsible for releasing a variable.
	pub fn own_local(&mut self, var: Variable, typ: &Typ) {
		if self.needs_release(typ) {
			self.scopes.last_mut().expect("scope").push((var, typ.clone()));
		}
	}

	// Emit releases for every scope deeper than `depth`, running its defers first.
	pub(super) fn release_scopes(&mut self, depth: usize, ret: Option<TypedVal>) -> Result<(), Diagnostic> {
		for s in (depth..self.scopes.len()).rev() {
			for i in (0..self.defers[s].len()).rev() {
				let d = self.defers[s][i].clone();
				self.run_defer(d, ret.as_ref())?;
			}
			for i in (0..self.scopes[s].len()).rev() {
				let (var, t) = self.scopes[s][i].clone();
				let v = self.b.use_var(var);
				if !self.flagged.contains(&var) {
					self.release_value(v, &t);
					continue;
				}
				let (live, done) = (self.b.create_block(), self.b.create_block());
				self.b.ins().brif(v, live, &[], done, &[]);
				self.b.seal_block(live);
				self.b.switch_to_block(live);
				self.release_value(v, &t);
				self.b.ins().jump(done, &[]);
				self.b.seal_block(done);
				self.b.switch_to_block(done);
			}
		}
		Ok(())
	}

	// An armed defer runs only if its flag was set.
	pub(super) fn defer(&mut self, body: &Spanned<Expr>, when: When, armed: bool) -> TypedVal {
		let armed = armed.then(|| {
			let (flag, one) = (self.b.declare_var(self.int), self.b.ins().iconst(self.int, 1));
			self.b.def_var(flag, one);
			flag
		});
		let (body, vars) = (body.clone(), self.vars.clone());
		self.defers.last_mut().expect("scope").push(Defer {
			body,
			vars,
			when,
			armed,
		});
		self.unit_value()
	}

	// `$` is the returned value / error.
	fn run_defer(&mut self, mut d: Defer, ret: Option<&TypedVal>) -> Result<(), Diagnostic> {
		// unarmed paths never def the flag, so it reads 0. disarm for the next loop iteration
		if let Some(flag) = d.armed.take() {
			let (run, after) = (self.b.create_block(), self.b.create_block());
			let armed = self.b.use_var(flag);
			self.b.ins().brif(armed, run, &[], after, &[]);
			self.b.seal_block(run);
			self.b.switch_to_block(run);
			let zero = self.b.ins().iconst(self.int, 0);
			self.b.def_var(flag, zero);
			self.run_defer(d, ret)?;
			self.b.ins().jump(after, &[]);
			self.b.seal_block(after);
			self.b.switch_to_block(after);
			return Ok(());
		}
		let mut dollar = ret.cloned();
		let mut join = None;
		if d.when != When::Always {
			let Some((val, typ)) = ret else { return Ok(()) };
			let Some((inner, err)) = self.fallible_split(typ) else {
				let kw = if d.when == When::Ok { "and" } else { "or" };
				let msg = format!("`defer {kw}` needs a fn returning `?T`/`!T`");
				return fail(msg, d.body.1, "this fn cannot fail");
			};
			let tag = self.enum_tag(typ, *val);
			let is_happy = self.b.ins().icmp_imm(IntCC::Equal, tag, err.is_none() as i64);
			let (run, after) = (self.b.create_block(), self.b.create_block());
			match d.when {
				When::Ok => self.b.ins().brif(is_happy, run, &[], after, &[]),
				_ => self.b.ins().brif(is_happy, after, &[], run, &[]),
			};
			self.b.seal_block(run);
			self.b.switch_to_block(run);
			dollar = Some(match (d.when, err) {
				(When::Ok, _) => (self.opt_payload(*val, typ, &inner, 8), inner),
				(_, Some(e)) => (self.ld_typ(*val, 8, &e), e),
				_ => self.unit_value(),
			});
			join = Some(after);
		}
		let dollar = dollar.or_else(|| self.dollar.clone());
		let saved = (
			std::mem::replace(&mut self.vars, d.vars.into()),
			std::mem::take(&mut self.loops), // `break` must not reach an enclosing loop
			std::mem::replace(&mut self.deferring, true),
			std::mem::replace(&mut self.dollar, dollar),
		);
		let out = self.scoped(|s| s.block_tail(std::slice::from_ref(&d.body), None).map(|_| Some(s.unit_value())));
		(self.vars, self.loops, self.deferring, self.dollar) = saved;
		if let Some(after) = join {
			self.b.ins().jump(after, &[]);
			self.b.seal_block(after);
			self.b.switch_to_block(after);
		}
		out.map(drop)
	}

	// Transfer ownership of a local binding.
	pub fn move_local(&mut self, name: &str, local: &Local, span: Range<usize>) -> Result<Value, Diagnostic> {
		if self.deferring {
			return Err(
				Diagnostic::new("cannot move a local out of a defer body", span).with_label("runs on every exit")
			);
		}
		if self.needs_release(&local.typ) {
			let depth = self
				.scopes
				.iter()
				.position(|s| s.iter().any(|(v, _)| *v == local.var))
				.ok_or_else(|| {
					Diagnostic::new(format!("cannot move `{name}`, it is borrowed here"), span.clone())
						.with_label("only an owned binding can be moved")
				})?;
			if let Some(frame) = self.loops.last()
				&& depth < frame.depth
			{
				return Err(
					Diagnostic::new(format!("cannot move `{name}` out of the enclosing loop"), span)
						.with_label("would be moved again on the next iteration"),
				);
			}
			self.scopes[depth].retain(|(v, _)| *v != local.var);
		}
		self.vars.remove(name);
		Ok(self.read_local(local))
	}

	// End a reached branch in its own tail, noting what it still owns.
	pub(super) fn branch_tail(&mut self, owned: &Owned, tails: &mut Vec<(Block, Owned)>, reached: bool) -> Block {
		let tail = self.b.create_block();
		let kept = std::mem::replace(&mut self.scopes, owned.clone());
		if reached {
			tails.push((tail, kept));
		}
		tail
	}

	// A binding moved on some paths is nulled there and drops at scope exit.
	pub(super) fn join_moves(&mut self, owned: Owned, tails: Vec<(Block, Owned)>, merge: Block) {
		let has = |s: &Owned, var| s.iter().flatten().any(|(v, _)| *v == var);
		let moved = |var| has(&owned, var) && tails.iter().any(|(_, s)| !has(s, var));
		for (tail, s) in &tails {
			self.b.switch_to_block(*tail);
			self.b.seal_block(*tail);
			for (var, _) in owned.iter().flatten().filter(|(v, _)| !has(s, *v)) {
				let null = self.b.ins().iconst(self.int, 0);
				self.b.def_var(*var, null);
				self.flagged.push(*var);
			}
			self.b.ins().jump(merge, &[]);
		}
		self.vars.retain(|_, l| !moved(l.var));
		self.scopes = owned;
	}

	// A bind takes its own copy.
	pub(crate) fn copy_bind(&mut self, val: Value, typ: &Typ) -> Value {
		if self.handover(val, typ) {
			self.untemp(val);
			return val;
		}
		match typ {
			Typ::Struct(_, fields) => {
				let dst = self.stack_slot((fields.len() * 8) as u32);
				self.copy_struct(val, dst, typ, fields)
			}
			_ => self.copy_in(val, typ),
		}
	}
}

// A generic instance's base name
// ex: `Box[int]` -> `Box`.
pub(super) fn base_name(name: &str) -> &str {
	name.split('[').next().unwrap_or(name)
}

// The bindings owned by each scope.
pub(super) type Owned = Vec<Vec<(Variable, Typ)>>;

pub(super) fn releasable(typ: &Typ) -> bool {
	match typ {
		Typ::Struct(_, fields) => fields.iter().any(|f| owns(&f.typ)),
		_ => handle_fns(typ).is_some(),
	}
}

// Whether a struct field slot owns its value.
pub(super) fn owns(typ: &Typ) -> bool {
	matches!(typ, Typ::Struct(..)) || releasable(typ)
}

fn field_types(fields: &[FieldDef]) -> Vec<Typ> {
	fields.iter().map(|f| f.typ.clone()).collect()
}

// The runtime share/release fns for rc'd types.
pub(super) fn handle_fns(typ: &Typ) -> Option<(&'static str, &'static str)> {
	match typ {
		Typ::Array(_) => Some(("array_share", "array_release")),
		Typ::Map(..) => Some(("map_share", "map_release")),
		t if ref_like(t) || *t == Typ::Any => Some(("ref_share", "ref_release")),
		t => handle_fns(t.newtype()?),
	}
}

// Whether `typ` is a `?^T`.
fn opt_ref(typ: &Typ) -> bool {
	matches!(typ, Typ::Enum(n) if n.strip_prefix(role::OPTION).is_some_and(|args| args.starts_with("[^")))
}

// Whether a type is a `?^T` or `?fn`, whose none is a null pointer.
pub(super) fn opt_niche(typ: &Typ) -> bool {
	let Typ::Enum(n) = typ else { return false };
	let arg = n.strip_prefix(role::OPTION).and_then(|a| a.strip_prefix('['));
	let bare = arg.and_then(|a| a.split(' ').find(|w| !w.starts_with('@')));
	opt_ref(typ) || bare.is_some_and(|w| w.starts_with("fn("))
}

// Is type a ref pointer?
pub(super) fn ref_like(typ: &Typ) -> bool {
	matches!(typ, Typ::Ref(_)) || opt_ref(typ)
}
