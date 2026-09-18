use super::*;

// Ownership bookkeeping.
// Every ref has one owner: a named binding, a container slot, or the scope that produced it.
// Owned values register in the innermost scope and release when it exits.

// A `defer` body, re-lowered at every exit of its scope.
#[derive(Clone)]
pub(crate) struct Defer {
	pub(crate) body: Spanned<Expr>,
	pub(crate) vars: HashMap<String, Local>,
	pub(crate) on_err: bool,
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
			Typ::Option(_) | Typ::Enum(_) => {
				if let Typ::Enum(name) = typ {
					if seen.contains(name) {
						return false;
					}
					seen.push(name.clone());
				}
				self.variants_of(typ)
					.iter()
					.any(|v| v.payload.iter().any(|t| self.is_resource_seen(t, seen)))
			}
			_ => false,
		}
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
	fn run_hook(&mut self, val: Value, typ: &Typ, name: &str, hook: &str) {
		if let Some(sig) = self
			.funcs
			.get(&format!("{name}.{hook}"))
			.cloned()
			.or_else(|| self.recv_instance(&format!("{}.{hook}", base_name(name)), typ))
		{
			self.emit_call(&sig, &[val]);
		}
	}

	// Settle a struct copy.
	pub(super) fn settle(&mut self, val: Value, dst: Value, typ: &Typ) {
		if self.handover(val, typ) {
			self.untemp(val);
		} else if let Typ::Struct(name, _) = typ
			&& self.is_copy(typ)
		{
			self.run_hook(dst, typ, name, "copy");
		}
	}

	// Transfer ownership out of an expression.
	// Projections borrow.
	pub fn move_out(&mut self, e: &Spanned<Expr>, typ: &Typ) -> Result<(), Diagnostic> {
		match &e.0 {
			Expr::Ident(n) => {
				let local = self.local(n, e.1.into_range())?;
				self.move_local(n, &local, e.1.into_range())?;
			}
			Expr::Index { .. } | Expr::Slice { .. } | Expr::Field { .. } => {
				return Err(
					Diagnostic::new(format!("cannot move {typ} out of its container"), e.1.into_range())
						.with_label("only an owned binding can be moved, so use it in place"),
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
		} else if let Typ::Struct(name, fields) = typ {
			if self.is_resource(typ) {
				self.run_hook(val, typ, name, "drop");
			}
			self.release_slots(val, 0, &fields.iter().map(|f| f.typ.clone()).collect::<Vec<_>>());
		} else if let Typ::Tuple(fields) = typ
			&& self.is_resource(typ)
		{
			self.release_slots(val, 0, &fields.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>());
		} else if matches!(typ, Typ::Option(_) | Typ::Enum(_)) && self.is_resource(typ) {
			let tag = self.b.ins().load(self.int, MemFlags::new(), val, 0);
			for v in self.variants_of(typ) {
				if !v.payload.iter().any(|t| releasable(t) || self.is_resource(t)) {
					continue;
				}
				// release payloads under their own tag
				let (hit, next) = (self.b.create_block(), self.b.create_block());
				let is = self.b.ins().icmp_imm(IntCC::Equal, tag, v.disc);
				self.b.ins().brif(is, hit, &[], next, &[]);
				self.b.seal_block(hit);
				self.b.switch_to_block(hit);
				self.release_slots(val, 8, &v.payload);
				self.b.ins().jump(next, &[]);
				self.b.seal_block(next);
				self.b.switch_to_block(next);
			}
		}
	}

	// Release the owned slots of an aggregate type.
	fn release_slots(&mut self, val: Value, base: i32, types: &[Typ]) {
		for (i, t) in types.iter().enumerate() {
			if releasable(t) || self.is_resource(t) {
				let cl = cl_type(t, self.int);
				let fv = self.b.ins().load(cl, MemFlags::new(), val, base + (i * 8) as i32);
				self.release_value(fv, t);
			}
		}
	}

	// The address of a struct's trace descriptor symbol.
	pub(super) fn trace_desc(&mut self, name: &str, fields: &[FieldDef]) -> Value {
		if self.desc_data(name, fields).is_none() {
			return self.b.ins().iconst(self.int, 0);
		}
		self.data_addr(&oi_symbol(&format!("{name}#trace")))
	}

	// Define trace descriptor on first use.
	fn desc_data(&mut self, name: &str, fields: &[FieldDef]) -> Option<DataId> {
		if let Some(&id) = self.descs.get(name) {
			return Some(id);
		}
		let mut words = vec![0i64];
		let mut relocs = Vec::new();
		for (i, f) in fields.iter().enumerate() {
			let off = ((i * 8) as i64) << 1;
			if ref_like(&f.typ) {
				words.push(off);
			} else if let Typ::Struct(n, sub) = &f.typ
				&& let Some(child) = self.desc_data(n, sub)
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
		self.descs.insert(name.to_string(), id);
		Some(id)
	}

	// Register a producer's fresh handle with the innermost scope.
	pub(super) fn temp(&mut self, val: Value, typ: &Typ) {
		if handle_fns(typ).is_some() || self.is_resource(typ) {
			let var = self.b.declare_var(self.int);
			self.b.def_var(var, val);
			self.temps.insert(val, var);
			self.scopes.last_mut().expect("scope").push((var, typ.clone()));
		}
	}

	// Declare a named binding that owns its value.
	pub fn bind_local(&mut self, name: &str, val: Value, typ: Typ, mutable: bool) {
		let var = self.b.declare_var(self.b.func.dfg.value_type(val));
		self.b.def_var(var, val);
		self.own_local(var, &typ);
		self.vars.insert(name.to_string(), Local::plain(var, typ, mutable));
	}

	// Make the innermost scope responsible for releasing a variable.
	pub fn own_local(&mut self, var: Variable, typ: &Typ) {
		if releasable(typ) || self.is_resource(typ) {
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
				self.release_value(v, &t);
			}
		}
		Ok(())
	}

	// `$` is the returned value / error.
	fn run_defer(&mut self, d: Defer, ret: Option<&TypedVal>) -> Result<(), Diagnostic> {
		let mut dollar = ret.cloned();
		let mut join = None;
		if d.on_err {
			let Some((val, typ)) = ret else { return Ok(()) };
			let (happy, err) = match typ {
				Typ::Option(_) => (1, None),
				Typ::Result(_, e) => (0, Some((**e).clone())),
				_ => {
					return Err(
						Diagnostic::new("`defer or` needs a fn returning `?T`/`!T`", d.body.1.into_range())
							.with_label("this fn cannot fail"),
					);
				}
			};
			let tag = self.enum_tag(typ, *val);
			let happy = self.b.ins().iconst(self.int, happy);
			let is_happy = self.b.ins().icmp(IntCC::Equal, tag, happy);
			let (sad, after) = (self.b.create_block(), self.b.create_block());
			self.b.ins().brif(is_happy, after, &[], sad, &[]);
			self.b.seal_block(sad);
			self.b.switch_to_block(sad);
			dollar = Some(match err {
				Some(e) => (self.b.ins().load(cl_type(&e, self.int), MemFlags::new(), *val, 8), e),
				None => self.unit_value(),
			});
			join = Some(after);
		}
		let dollar = dollar.or_else(|| self.dollar.clone());
		let saved = (
			std::mem::replace(&mut self.vars, d.vars),
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
		if releasable(&local.typ) || self.is_resource(&local.typ) {
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

	// A bind takes its own copy.
	pub(super) fn copy_bind(&mut self, val: Value, typ: &Typ) -> Value {
		if self.handover(val, typ) {
			self.untemp(val);
			return val;
		}
		match typ {
			Typ::Struct(_, fields) => {
				let fields = fields.clone();
				let dst = self.stack_slot((fields.len() * 8) as u32);
				self.assign_fields(val, dst, &fields, false);
				self.settle(val, dst, typ);
				dst
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

pub(super) fn releasable(typ: &Typ) -> bool {
	match typ {
		Typ::Struct(_, fields) => fields.iter().any(|f| releasable(&f.typ)),
		_ => handle_fns(typ).is_some(),
	}
}

// The runtime share/release fns for rc'd types.
pub(super) fn handle_fns(typ: &Typ) -> Option<(&'static str, &'static str)> {
	match typ {
		Typ::Array(_) => Some(("array_share", "array_release")),
		Typ::Map(..) => Some(("map_share", "map_release")),
		t if ref_like(t) => Some(("ref_share", "ref_release")),
		t => handle_fns(t.newtype()?),
	}
}

// Is `typ` a `?&T`?
pub(super) fn opt_ref(typ: &Typ) -> bool {
	matches!(typ, Typ::Option(i) if matches!(&**i, Typ::Ref(_)))
}

// Is type a ref pointer?
pub(super) fn ref_like(typ: &Typ) -> bool {
	matches!(typ, Typ::Ref(_)) || opt_ref(typ)
}
