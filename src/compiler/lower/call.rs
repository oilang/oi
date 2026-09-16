use super::*;
use crate::compiler::role;

// A chosen fill and the args to call it on.
type Picked = (FnSig, Vec<Spanned<Expr>>);

// Unwrap a marked call arg.
pub(super) fn arg_inner(arg: &Spanned<Expr>) -> &Spanned<Expr> {
	match &arg.0 {
		Expr::ArgMod(_, inner) => inner,
		_ => arg,
	}
}

// Collect the vararg run into one array literal, leaving a fixed-arity argument list.
pub(super) fn pack_varargs(
	name: &str,
	params: impl Iterator<Item = (bool, bool)>,
	args: &[Spanned<Expr>],
	span: Span,
) -> Result<Option<Vec<Spanned<Expr>>>, Diagnostic> {
	let params: Vec<_> = params.collect();
	let Some(v) = params.iter().position(|&(variadic, _)| variadic) else {
		return Ok(None);
	};
	let named = matches!(args.last(), Some((Expr::Record(es), _)) if !es.is_empty());
	let pos = &args[..args.len() - named as usize];
	// defaults after the vararg are named-only
	let k = params[v + 1..].iter().filter(|&(_, has_default)| !has_default).count();
	if pos.len() < v + k {
		let msg = format!("`{name}` expects {}.. argument(s), got {}", v + k, pos.len());
		return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of arguments"));
	}
	let (fixed, rest) = pos.split_at(v);
	let (loose, tail) = rest.split_at(rest.len() - k);
	let pack = [(Expr::Array(loose.to_vec()), span)];
	Ok(Some([fixed, &pack, tail, &args[pos.len()..]].concat()))
}

// Named arguments.
pub(super) fn arg_slots<'e>(
	name: &str,
	names: &[&str],
	args: &'e [Spanned<Expr>],
	coerces: bool,
) -> Result<Option<Vec<Option<&'e Spanned<Expr>>>>, Diagnostic> {
	let Some(((Expr::Record(entries), _), pos)) = args.split_last() else {
		return Ok(None);
	};
	let named = |k: &Spanned<Expr>| matches!(&k.0, Expr::Ident(n) if !coerces || names.contains(&n.as_str()));
	if entries.is_empty() || pos.len() >= names.len() || !entries.iter().all(|(k, _)| named(k)) {
		return Ok(None);
	}
	let mut slots = vec![None; names.len()];
	for (slot, arg) in slots.iter_mut().zip(pos) {
		*slot = Some(arg);
	}
	for (key, value) in entries {
		let Expr::Ident(k) = &key.0 else { unreachable!() };
		let i = names.iter().position(|n| n == k).ok_or_else(|| {
			Diagnostic::new(format!("`{name}` has no parameter `{k}`"), key.1.into_range())
				.with_label("no such parameter")
		})?;
		if slots[i].replace(value).is_some() {
			return Err(
				Diagnostic::new(format!("`{k}` is already given"), key.1.into_range()).with_label("duplicate argument")
			);
		}
	}
	Ok(Some(slots))
}

// Arg values and what they lend.
type CallArgs = (Vec<Value>, Vec<(Value, Lent)>);

// How a value call reaches its code.
pub(super) enum Callee {
	// a fn value
	Object(Value),
	// a vtable slot
	Addr(Value),
}

// What a mut arg lends to a callee.
pub(super) enum Lent {
	Whole(Local),
	Slice { parent: Local, lo: Value, len: Value },
}

impl<'a, M: Module> Translator<'a, M> {
	pub(super) fn import_fn(
		&mut self,
		name: &str,
		params: &[types::Type],
		ret: Option<types::Type>,
	) -> codegen::ir::FuncRef {
		let mut sig = self.module.make_signature();
		for &p in params {
			sig.params.push(AbiParam::new(p));
		}
		if let Some(r) = ret {
			sig.returns.push(AbiParam::new(r));
		}
		let id = self.module.declare_function(name, Linkage::Import, &sig).unwrap();
		self.module.declare_func_in_func(id, self.b.func)
	}

	// Call a `core/rt` runtime fn.
	pub(super) fn rt_call(&mut self, name: &str, args: &[Value]) -> Option<Value> {
		let key = format!("rt::oi_{name}");
		let sig = self
			.funcs
			.get(&key)
			.unwrap_or_else(|| panic!("`{key}` is not declared in `core/rt`"));
		let (id, unit) = (sig.id, sig.ret.is_unit());
		self.wanted.push(id);
		let func = self.module.declare_func_in_func(id, self.b.func);
		let call = self.b.ins().call(func, args);
		(!unit).then(|| self.b.inst_results(call)[0])
	}

	// Type arguments only mean something on a generic fn.
	pub(super) fn check_type_args(
		&self,
		name: &str,
		key: &str,
		type_args: &[Spanned<TypeExpr>],
		span: Span,
	) -> Result<(), Diagnostic> {
		match type_args.is_empty() || self.generic_fns.contains_key(key) {
			true => Ok(()),
			false => Err(Diagnostic::new(format!("`{name}` is not generic"), span.into_range())
				.with_label("unexpected type arguments")),
		}
	}

	// Whether C may call this fn value.
	pub(super) fn c_callable(&self, arg: Option<&Spanned<Expr>>) -> bool {
		match arg.map(|a| &a.0) {
			Some(Expr::Ident(n)) if !self.vars.contains_key(n) => {
				self.funcs.get(self.qualify(n).as_ref()).is_none_or(|s| s.foreign)
			}
			Some(Expr::AnonFn { .. }) => false,
			_ => true,
		}
	}

	// Call a `pub` function of an imported module.
	pub(super) fn module_call(
		&mut self,
		module: &str,
		method: &str,
		type_args: &[Spanned<TypeExpr>],
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let key = format!("{module}::{method}");
		let key = self.reexports.get(&key).cloned().unwrap_or(key);
		let known = self.funcs.contains_key(&key) || self.generic_fns.contains_key(&key);
		if !self.publics.contains(&key) {
			let (msg, label) = if known {
				(format!("`{method}` is private to module `{module}`"), "not public")
			} else {
				(
					format!("module `{module}` has no function `{method}`"),
					"no such function",
				)
			};
			return Err(Diagnostic::new(msg, span.into_range()).with_label(label));
		}
		self.check_type_args(method, &key, type_args, span)?;
		if let Some(sig) = self.funcs.get(&key).cloned() {
			return self.call_sig(method, sig, None, None, args, span);
		}
		if let Some(def) = self.generic_fns.get(&key).cloned() {
			return self.call_generic(&key, &def, type_args, args, None, span);
		}
		Err(Diagnostic::new(
			format!("module `{module}` has no function `{method}`"),
			span.into_range(),
		)
		.with_label("no such function"))
	}

	// Emit a call to a resolved fn.
	pub(super) fn call_sig(
		&mut self,
		name: &str,
		sig: FnSig,
		recv: Option<Value>,
		recv_expr: Option<&Spanned<Expr>>,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if sig.unsafe_call {
			self.require_unsafe(name, span)?;
		}
		if !sig.pure {
			self.require_pure(name, span)?;
		}
		let self_n = recv.is_some() as usize;
		// `@params`
		let synth;
		let args = if args.len() + self_n + 1 == sig.params.len()
			&& let Some(Typ::Struct(n, _)) = sig.params.last().map(|p| &p.typ)
			&& self
				.annotations
				.get(n)
				.is_some_and(|anns| anns.iter().any(|(e, _)| matches!(e, Expr::Ident(q) if q == role::PARAMS)))
		{
			synth = [args, &[(Expr::Record(vec![]), span)]].concat();
			&synth[..]
		} else {
			args
		};
		let mut params = sig.value_params();
		for p in params.iter_mut().filter(|p| sig.foreign && matches!(p.typ, Typ::Fn(..))) {
			p.typ = Typ::Annotated(vec![role::C.into()], Box::new(p.typ.clone()));
		}
		let (vals, lent) = self.call_args(name, &params, recv, recv_expr, args, span)?;
		let out = self.emit_call(&sig, &vals);
		self.reload_lent(&lent);
		Ok(out)
	}

	// Picks one of key's numbered fills by the first arg's type, lowering that arg into a hidden temp.
	pub(super) fn pick_fill(
		&mut self,
		key: &str,
		skip: usize,
		args: &[Spanned<Expr>],
	) -> Result<Option<Picked>, Diagnostic> {
		let tag = format!("{key}#");
		if args.is_empty() || !self.funcs.keys().any(|k| k.starts_with(&tag)) {
			return Ok(None);
		}
		let (val, typ) = self.expr(&args[0])?;
		let Some(sig) = self.find_fill(key, skip, &typ) else {
			let msg = format!("no claim fills `{}` for a `{typ}`", display_name(key));
			return Err(Diagnostic::new(msg, args[0].1.into_range()).with_label("no matching claim"));
		};
		let name = format!("$fill{}", self.vars.len());
		let var = self.b.declare_var(self.b.func.dfg.value_type(val));
		self.b.def_var(var, val);
		self.vars.insert(name.clone(), Local::plain(var, typ, false));
		let mut args = args.to_vec();
		args[0] = (Expr::Ident(name), args[0].1);
		Ok(Some((sig, args)))
	}

	// Find a fill whose parameter takes the given type.
	pub(super) fn find_fill(&self, key: &str, skip: usize, typ: &Typ) -> Option<FnSig> {
		let tag = format!("{key}#");
		let mut keys: Vec<&String> = self.funcs.keys().filter(|k| k.starts_with(&tag)).collect();
		keys.sort();
		let hit =
			(keys.into_iter()).find(|k| self.funcs[*k].params.get(skip).is_some_and(|p| access_peel(&p.typ) == typ))?;
		Some(self.funcs[hit].clone())
	}

	// Swap each spread `..x` for reads of a hidden temp holding x.
	fn expand_spreads(&mut self, args: &[Spanned<Expr>]) -> Result<Option<Vec<Spanned<Expr>>>, Diagnostic> {
		if !args.iter().any(|a| matches!(a.0, Expr::Spread(_))) {
			return Ok(None);
		}
		let mut out = Vec::with_capacity(args.len());
		for (i, arg) in args.iter().enumerate() {
			let (Expr::Spread(inner), at) = arg else {
				out.push(arg.clone());
				continue;
			};
			let (mut val, mut typ) = self.expr(inner)?;
			(val, typ) = self.collect_spread((val, typ));
			let name = format!("$spread{i}");
			let ident = || Box::new((Expr::Ident(name.clone()), *at));
			match &typ {
				Typ::Tuple(fs) => out.extend((0..fs.len()).map(|f| {
					let (tuple, field) = (ident(), f.to_string());
					(Expr::Field { tuple, field }, *at)
				})),
				Typ::FixedArray(_, n) => out.extend((0..*n).map(|f| {
					let (collection, index) = (ident(), Box::new((Expr::Int(f as i64), *at)));
					(Expr::Index { collection, index }, *at)
				})),
				Typ::Array(_) => out.push((Expr::Spread(ident()), *at)),
				Typ::Int(_) => {
					let (zero, one) = (self.b.ins().iconst(types::I64, 0), self.b.ins().iconst(types::I64, 1));
					(val, typ) = self.make_range(zero, Some(val), one, inner.1)?;
					out.push(*ident());
				}
				_ => {
					return Err(Diagnostic::new(format!("cannot spread {typ}"), inner.1.into_range())
						.with_label("not a tuple or array"));
				}
			}
			let var = self.b.declare_var(self.b.func.dfg.value_type(val));
			self.b.def_var(var, val);
			self.vars.insert(name, Local::plain(var, typ, false));
		}
		Ok(Some(out))
	}

	// Slot the args, fill defaults, and evaluate.
	fn call_args(
		&mut self,
		name: &str,
		params: &[FnParam],
		recv: Option<Value>,
		recv_expr: Option<&Spanned<Expr>>,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<CallArgs, Diagnostic> {
		let self_n = recv.is_some() as usize;
		let expanded = self.expand_spreads(args)?;
		let args = expanded.as_deref().unwrap_or(args);
		let access: Vec<Access> = params.iter().map(|p| access_of(&p.typ)).collect();
		let names: Vec<&str> = (params.iter().skip(self_n))
			.map(|p| p.name.as_deref().unwrap_or_default())
			.collect();
		let coerces = matches!(params.last().map(|p| access_peel(&p.typ)), Some(Typ::Struct(..)));
		let packed = pack_varargs(
			name,
			params[self_n..].iter().map(|p| (p.variadic, p.default.is_some())),
			args,
			span,
		)?;
		let args = packed.as_deref().unwrap_or(args);
		if let Some(arg) = args.iter().find(|a| matches!(a.0, Expr::Spread(_))) {
			return Err(
				Diagnostic::new("a `[]T` spread may only feed the vararg slot", arg.1.into_range())
					.with_label("this slice has no static length"),
			);
		}
		let named = arg_slots(name, &names, args, coerces)?;
		if named.is_none() {
			let n_defaults = params.iter().rev().take_while(|p| p.default.is_some()).count();
			let total = args.len() + self_n;
			if total + n_defaults < params.len() || total > params.len() {
				let mut want = (params.len() - n_defaults - self_n).to_string();
				if n_defaults > 0 {
					want = format!("{want}..{}", params.len() - self_n);
				}
				let msg = format!("`{name}` expects {want} argument(s), got {}", args.len());
				return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of arguments"));
			}
		}
		let slots: Vec<_> = named.unwrap_or_else(|| (0..names.len()).map(|i| args.get(i)).collect());
		match recv_expr {
			Some(re) => {
				self.check_args(&access, recv_expr, &slots)?;
				if access[0] == Access::Move {
					self.move_out(re, access_peel(&params[0].typ))?;
				}
			}
			None => self.check_args(&access[self_n..], None, &slots)?,
		}
		let fills = slots.iter().any(Option::is_none);
		let saved: Vec<_> = match fills {
			true => (params.iter().filter_map(|p| p.name.as_ref()))
				.map(|n| (n.clone(), self.vars.remove(n)))
				.collect(),
			false => Vec::new(),
		};
		let mut vals = Vec::with_capacity(params.len());
		vals.extend(recv);
		let mut lent = Vec::new();
		for (i, p) in params.iter().enumerate() {
			let want = access_peel(&p.typ);
			if i >= self_n {
				let (val, typ) = match slots[i - self_n] {
					Some(arg) => {
						let (val, typ, entry) = self.arg_value(access[i], arg, Some(want))?;
						lent.extend(entry.map(|e| (val, e)));
						(val, typ)
					}
					None => {
						let Some(default) = &p.default else {
							let msg = format!(
								"`{name}` is missing argument `{}`",
								p.name.as_deref().unwrap_or_default()
							);
							return Err(
								Diagnostic::new(msg, span.into_range()).with_label("no value for this parameter")
							);
						};
						(
							self.check_typed(default, want, "not a valid default for this parameter")?,
							want.clone(),
						)
					}
				};
				if &typ != want {
					let at = slots[i - self_n].expect("a default is checked as it is evaluated").1;
					return Err(
						Diagnostic::new(format!("expected {want} argument, got {typ}"), at.into_range())
							.with_label("wrong argument type"),
					);
				}
				vals.push(val);
			}
			if fills && let Some(n) = &p.name {
				let var = self.b.declare_var(cl_type(want, self.int));
				self.b.def_var(var, vals[i]);
				self.vars.insert(n.clone(), Local::plain(var, want.clone(), false));
			}
		}
		for (name, old) in saved {
			match old {
				Some(local) => self.vars.insert(name, local),
				None => self.vars.remove(&name),
			};
		}
		Ok((vals, lent))
	}

	// Evaluate one argument under its access mod.
	pub(super) fn arg_value(
		&mut self,
		access: Access,
		arg: &Spanned<Expr>,
		want: Option<&Typ>,
	) -> Result<(Value, Typ, Option<Lent>), Diagnostic> {
		let arg = arg_inner(arg);
		if access == Access::Mut {
			let (cell, typ, entry) = self.lend_mut(arg)?;
			return Ok((cell, typ, Some(entry)));
		}
		let (val, typ) = match want {
			Some(want) => self.check_expr(arg, want)?,
			None => self.expr(arg)?,
		};
		if access == Access::Move {
			self.move_out(arg, &typ)?;
			self.untemp(val);
		}
		Ok((val, typ, None))
	}

	// Pass the address of the caller's binding.
	pub(super) fn lend_mut(&mut self, inner: &Spanned<Expr>) -> Result<(Value, Typ, Lent), Diagnostic> {
		let (cur, typ, entry) = match &inner.0 {
			Expr::Slice { collection, range } => {
				let Expr::Ident(name) = &collection.0 else {
					unreachable!("check_muts admits only ident-based slices")
				};
				let parent = self.local(name, collection.1.into_range())?;
				let opnd = self.expr(collection)?;
				let (copy, lo, elem) = self.slice_copy(opnd, collection.1, range.as_deref())?;
				let len = self.array_len(copy);
				(copy, Typ::Array(Box::new(elem)), Lent::Slice { parent, lo, len })
			}
			Expr::Ident(name) => {
				let local = self.local(name, inner.1.into_range())?;
				(self.read_local(&local), local.typ.clone(), Lent::Whole(local))
			}
			_ => unreachable!("check_muts admits only idents and ident-based slices"),
		};
		let slot = self.stack_slot(8);
		self.b.ins().store(MemFlags::new(), cur, slot, 0);
		Ok((slot, typ, entry))
	}

	// After a call, reload a binding.
	pub(super) fn reload_lent(&mut self, lent: &[(Value, Lent)]) {
		for (slot, entry) in lent {
			let val = self.b.ins().load(self.int, MemFlags::new(), *slot, 0);
			match entry {
				Lent::Whole(local) => self.write_local(local, val),
				Lent::Slice { parent, lo, len } => {
					let elem = array_elem(&parent.typ).clone();
					let base = self.read_local(parent);
					self.cow_array(base, &elem);
					let stride = self.elem_stride(&elem);
					let size = self.b.ins().iconst(self.int, stride);
					self.rt_call("array_write_back", &[base, *lo, *len, val, size]);
					self.release_value(val, &parent.typ);
				}
			}
		}
	}

	// Callsite access checks.
	pub(super) fn check_args(
		&self,
		access: &[Access],
		recv: Option<&Spanned<Expr>>,
		args: &[Option<&Spanned<Expr>>],
	) -> Result<(), Diagnostic> {
		if let Some(re) = recv
			&& access[0] == Access::Mut
			&& !matches!(re.0, Expr::Cast { .. })
		{
			self.mut_place(re, "calling a `mut self` method needs a `mut` binding")?;
		}
		for (i, (&arg, &want)) in args.iter().zip(&access[recv.is_some() as usize..]).enumerate() {
			let Some(arg) = arg else { continue };
			let given = match &arg.0 {
				Expr::ArgMod(a, _) => *a,
				_ => Access::Read,
			};
			if given != want {
				let (msg, label) = match given {
					Access::Read => (
						format!("this parameter is `{want}`, missing `{want}` at the callsite"),
						format!("wrap it, e.g. `f({want} x)`"),
					),
					given => (
						format!("this parameter is not `{given}`"),
						format!("remove `{given}` here"),
					),
				};
				return Err(Diagnostic::new(msg, arg.1.into_range()).with_label(label));
			}
			if want != Access::Mut {
				continue;
			}
			let name = match &arg_inner(arg).0 {
				Expr::Slice { collection, .. } => {
					self.mut_place(collection, "only a mutable binding can be lent `mut`")?
				}
				_ => self.mut_place(arg_inner(arg), "only a mutable binding can be lent `mut`")?,
			};
			let mut touched = HashSet::new();
			let others = args.iter().enumerate().filter(|&(j, _)| j != i).filter_map(|(_, a)| *a);
			for e in recv.into_iter().chain(others) {
				e.0.idents(&mut touched);
			}
			if touched.contains(name) {
				let msg = format!("cannot use `{name}` while it is lent `mut`");
				return Err(Diagnostic::new(msg, arg.1.into_range()).with_label("borrowed exclusively for this call"));
			}
		}
		Ok(())
	}

	// Require a mutable binding place.
	fn mut_place<'e>(&self, e: &'e Spanned<Expr>, msg: &str) -> Result<&'e String, Diagnostic> {
		let Expr::Ident(name) = &e.0 else {
			return Err(Diagnostic::new(msg, e.1.into_range()).with_label("not a binding"));
		};
		if !self.local(name, e.1.into_range())?.mutable {
			return Err(Diagnostic::new(msg, e.1.into_range()).with_label("immutably bound"));
		}
		Ok(name)
	}

	// Emit the actual call instruction for a resolved fn signature.
	pub(super) fn emit_call(&mut self, sig: &FnSig, vals: &[Value]) -> TypedVal {
		self.wanted.push(sig.id);
		let func = self.module.declare_func_in_func(sig.id, self.b.func);
		let call = self.b.ins().call(func, vals);
		let ret_val = if sig.ret.is_unit() {
			self.b.ins().iconst(self.int, 0)
		} else {
			self.b.inst_results(call)[0]
		};
		self.temp(ret_val, &sig.ret);
		(ret_val, sig.ret.clone())
	}

	// Call through a value as a function.
	pub(super) fn call_value(
		&mut self,
		name: &str,
		callee: Callee,
		typ: &Typ,
		args: &[Spanned<Expr>],
		recv: Option<Value>,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		if let Typ::Annotated(_, inner) = typ {
			let (Callee::Object(addr) | Callee::Addr(addr)) = callee;
			return self.call_value(name, Callee::Addr(addr), inner, args, recv, span);
		}
		let (params, ret) = match typ {
			Typ::Fn(params, ret) | Typ::Closure(params, ret, _) => (params, &**ret),
			typ => {
				return Err(Diagnostic::new(format!("`{name}` is not callable"), span.into_range())
					.with_label(format!("this is {typ}, not a function")));
			}
		};
		self.require_pure(name, span)?;
		let (mut vals, lent) = self.call_args(name, params, recv, None, args, span)?;
		let mut sig = self.module.make_signature();
		sig.params
			.extend(params.iter().map(|p| AbiParam::new(cl_type(&p.typ, self.int))));
		let addr = match callee {
			Callee::Addr(addr) => addr,
			Callee::Object(obj) => {
				sig.params.push(AbiParam::new(self.int));
				vals.push(obj);
				self.b.ins().load(self.int, MemFlags::new(), obj, 0)
			}
		};
		let is_unit = ret.is_unit();
		if !is_unit {
			sig.returns.push(AbiParam::new(cl_type(ret, self.int)));
		}
		let sig_ref = self.b.import_signature(sig);
		let call = self.b.ins().call_indirect(sig_ref, addr, &vals);
		let ret_val = if is_unit {
			self.b.ins().iconst(self.int, 0)
		} else {
			self.b.inst_results(call)[0]
		};
		self.reload_lent(&lent);
		self.temp(ret_val, ret);
		Ok((ret_val, ret.clone()))
	}

	pub(super) fn call_concat(&mut self, a: Value, b: Value) -> Value {
		self.rt_call("str_concat", &[a, b]).unwrap()
	}

	pub(super) fn call_alloc(&mut self, n: usize) -> Value {
		self.call_alloc_bytes((n * 8) as i64)
	}

	pub(super) fn call_alloc_bytes(&mut self, bytes: i64) -> Value {
		let size = self.b.ins().iconst(self.int, bytes);
		self.rt_call("alloc", &[size]).unwrap()
	}

	// Pack a value into an i64 slot for the map's fixed width.
	pub(super) fn map_bits(&mut self, val: Value) -> Value {
		let cl = self.b.func.dfg.value_type(val);
		let iv = if cl.is_float() {
			self.b.ins().bitcast(cl_int_for_width(cl.bits() as u16), MemFlags::new(), val)
		} else {
			val
		};
		if cl.bits() < 64 {
			self.b.ins().uextend(self.int, iv)
		} else {
			iv
		}
	}

	// Recover a value's native width/kind.
	pub(super) fn unmap_bits(&mut self, val: Value, typ: &Typ) -> Value {
		let cl = cl_type(typ, self.int);
		let iv = if cl.bits() < 64 {
			self.b.ins().ireduce(cl_int_for_width(cl.bits() as u16), val)
		} else {
			val
		};
		if cl.is_float() {
			self.b.ins().bitcast(cl, MemFlags::new(), iv)
		} else {
			iv
		}
	}

	// Type-check a map index against key type `key_typ`.
	pub(super) fn map_key(
		&mut self,
		index: &Spanned<Expr>,
		key_typ: &Typ,
	) -> Result<(runtime::Tag, Value), Diagnostic> {
		let tag = map_key_tag(key_typ).ok_or_else(|| {
			Diagnostic::new(format!("{key_typ} cannot be used as a map key"), index.1.into_range())
				.with_label("unsupported key type")
		})?;
		let (val, typ) = self.check_expr(index, key_typ)?;
		if &typ != key_typ {
			return Err(
				Diagnostic::new(format!("expected {key_typ} key, got {typ}"), index.1.into_range())
					.with_label("wrong key type"),
			);
		}
		Ok((tag, self.map_bits(val)))
	}

	pub(super) fn call_map_new(&mut self) -> Value {
		self.rt_call("map_new", &[]).unwrap()
	}

	pub(super) fn call_map_get(&mut self, map: Value, tag: runtime::Tag, bits: Value) -> Value {
		let tag_v = self.b.ins().iconst(self.int, tag as i64);
		self.rt_call("map_get", &[map, tag_v, bits]).unwrap()
	}

	pub(super) fn call_map_set(&mut self, map: Value, tag: runtime::Tag, bits: Value, value: Value) -> Value {
		let tag_v = self.b.ins().iconst(self.int, tag as i64);
		self.rt_call("map_set", &[map, tag_v, bits, value]).unwrap()
	}

	// A map's keys or values as an array.
	pub(super) fn map_entries(&mut self, map: Value, keys: bool, elem: &Typ) -> Value {
		let width = self.elem_stride(elem);
		let keys = self.b.ins().iconst(self.int, keys as i64);
		let width = self.b.ins().iconst(self.int, width);
		self.rt_call("map_entries", &[map, keys, width]).unwrap()
	}

	pub(super) fn call_map_delete(&mut self, map: Value, tag: runtime::Tag, bits: Value) -> Value {
		let tag_v = self.b.ins().iconst(self.int, tag as i64);
		self.rt_call("map_delete", &[map, tag_v, bits]).unwrap()
	}

	// Dispatch a trait-object method through its vtable.
	pub(super) fn dyn_call(
		&mut self,
		boxv: Value,
		tn: &str,
		method: &str,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let (.., tmethods) = self.traits[tn];
		let Some((idx, (_, params, ret))) = trait_fns(tmethods).enumerate().find(|(_, (n, ..))| *n == method) else {
			let msg = format!("trait `{tn}` has no method `{method}`");
			return Err(Diagnostic::new(msg, span.into_range()).with_label("no such method"));
		};
		if params.first().is_some_and(|p| p.access == Access::Move) {
			let msg = format!("cannot call `{method}` through a `{tn}` object");
			return Err(Diagnostic::new(msg, span.into_range()).with_label("an object only borrows its data"));
		}
		// the receiver slot is the erased data pointer, the rest resolve like any signature
		let mut typs = vec![FnParam::new(Typ::Trait(tn.into()))];
		for p in params.iter().skip(1) {
			typs.push(FnParam::of(
				p,
				access_wrap(p.access, self.types().resolve(&p.typ, p.span)?),
			));
		}
		let ret = match ret {
			Some((te, s)) => self.types().resolve(te, *s)?,
			None => Typ::unit(),
		};
		let vtable = self.b.ins().load(self.int, MemFlags::new(), boxv, 0);
		let data = self.b.ins().load(self.int, MemFlags::new(), boxv, 8);
		let fnptr = self.b.ins().load(self.int, MemFlags::new(), vtable, (idx * 8) as i32);
		let typ = Typ::Fn(typs, Box::new(ret));
		self.call_value(method, Callee::Addr(fnptr), &typ, args, Some(data), span)
	}

	// Dyn-dispatch `message()` on a boxed `Error`.
	pub(super) fn error_message(&mut self, boxv: Value) -> Value {
		let Ok((s, _)) = self.dyn_call(boxv, role::ERROR, "message", &[], (0..0).into()) else {
			unreachable!("core::Error always has `message`")
		};
		s
	}

	// Read a required trait field.
	pub(super) fn trait_field(
		&mut self,
		boxv: Value,
		tn: &str,
		field: &str,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let (_, _, tfields, tmethods) = self.traits[tn];
		let Some(idx) = tfields.iter().position(|f| f.name == field) else {
			let msg = format!("trait `{tn}` has no field `{field}`");
			return Err(Diagnostic::new(msg, span.into_range()).with_label("no such field"));
		};
		let ftyp = self.types().resolve(&tfields[idx].typ, tfields[idx].span)?;
		let vtable = self.b.ins().load(self.int, MemFlags::new(), boxv, 0);
		let data = self.b.ins().load(self.int, MemFlags::new(), boxv, 8);
		let m = trait_fns(tmethods).count();
		// slot offset lives after the method pointers in the vtable
		let off = self.b.ins().load(self.int, MemFlags::new(), vtable, ((m + idx) * 8) as i32);
		let addr = self.rt_call("trait_field", &[data, off]).unwrap();
		let v = self.b.ins().load(cl_type(&ftyp, self.int), MemFlags::new(), addr, 0);
		Ok((v, ftyp))
	}
}
