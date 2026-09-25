use std::borrow::Cow;

use super::*;

impl<'a, M: Module> Translator<'a, M> {
	// The named types in scope.
	pub(super) fn types(&self) -> TypeCtx<'a> {
		self.types
	}

	// The scope of the module a fn was written in.
	pub(super) fn home_scope(&self, module: &str) -> &'a Scope {
		&self.module_scopes[if module.is_empty() { "main" } else { module }]
	}

	// Qualify a bare top-level name against the module's own items.
	pub(super) fn qualify<'n>(&'n self, name: &'n str) -> Cow<'n, str> {
		match self.types.scope.env.get(name) {
			Some(q) => Cow::Borrowed(q.as_str()),
			None if self.types.scope.module.is_empty() || name.contains("::") => Cow::Borrowed(name),
			None => Cow::Owned(format!("{}::{name}", self.types.scope.module)),
		}
	}

	// Rewrite `mod.T` to the canonical `module::T` ident, so every type path sees a plain name.
	pub(super) fn dotted_type(&self, e: &Spanned<Expr>) -> Option<Spanned<Expr>> {
		let Expr::Field { tuple, field } = &e.0 else {
			return None;
		};
		let Expr::Ident(m) = &tuple.0 else { return None };
		let vis = self.types.scope.visible.get(m).filter(|_| !self.vars.contains_key(m))?;
		let t = match &vis.only {
			None => field,
			Some(only) => only.get(field)?,
		};
		let key = format!("{}::{t}", vis.module);
		let known = self.types.structs.contains_key(&key)
			|| self.types.enums.borrow().contains_key(&key)
			|| self.types.generics.structs.contains_key(&key)
			|| self.types.aliases.contains_key(&key);
		known.then_some((Expr::Ident(key), e.1))
	}

	// Ensure that a static is only written to from within its own module.
	pub(super) fn check_static_write(&self, m: &str, field: &str, span: Span) -> Result<(), Diagnostic> {
		let Some(vis) = self.types.scope.visible.get(m).filter(|_| !self.vars.contains_key(m)) else {
			return Ok(());
		};
		let key = format!("{}::{field}", vis.module);
		if !self.vars.get(&key).is_some_and(|l| l.stat) {
			return Ok(());
		}
		let msg = format!("cannot assign to `{m}.{field}` outside module `{}`", vis.module);
		Err(Diagnostic::new(msg, span.into_range()).with_label("read-only here"))
	}

	// Ensure that no private members are accessed from outside their module.
	pub(super) fn check_member(&self, typ: &str, member: &str, span: Span) -> Result<(), Diagnostic> {
		let def = rc::base_name(typ);
		let owner = def.split_once("::").map_or("", |(m, _)| m);
		if owner == self.types.scope.module || !self.privates.get(def).is_some_and(|ms| ms.contains(member)) {
			return Ok(());
		}
		let msg = format!("`{member}` is private to module `{owner}`");
		Err(Diagnostic::new(msg, span.into_range()).with_label("not public"))
	}

	// Search embedded structs for `wanted`.
	// Returns the embed slot path.
	pub(super) fn pierce<T>(
		&self,
		fields: &[FieldDef],
		wanted: &str,
		span: Span,
		find: impl Fn(&str, &[FieldDef]) -> Option<T>,
	) -> Result<Option<(Vec<usize>, T)>, Diagnostic> {
		let mut level = vec![(Vec::new(), fields)];
		while !level.is_empty() {
			let (mut hits, mut next) = (Vec::new(), Vec::new());
			for (path, fs) in level {
				for (o, sn, inner) in embeds(fs) {
					let path = [path.as_slice(), &[o]].concat();
					match find(sn, inner) {
						Some(t) => hits.push((path, sn, t)),
						None => next.push((path, inner)),
					}
				}
			}
			if let [(a, ..), (b, ..), ..] = &hits[..] {
				return Err(ambiguous(wanted, &fields[a[0]].name, &fields[b[0]].name, span));
			}
			if let Some((path, sn, t)) = hits.pop() {
				self.check_member(sn, wanted, span)?;
				return Ok(Some((path, t)));
			}
			level = next;
		}
		Ok(None)
	}

	// Find `wanted` as a field of an embedded struct.
	pub(super) fn promoted(
		&self,
		fields: &[FieldDef],
		wanted: &str,
		span: Span,
	) -> Result<Option<(Vec<usize>, usize, Typ)>, Diagnostic> {
		let find = |_: &str, f: &[FieldDef]| f.iter().position(|f| f.name == wanted).map(|i| (i, f[i].typ.clone()));
		Ok(self.pierce(fields, wanted, span, find)?.map(|(p, (i, t))| (p, i, t)))
	}

	// Look through a chain of embed slots to the innermost struct pointer.
	pub(super) fn follow(&mut self, ptr: Value, path: &[usize]) -> Value {
		path.iter().fold(ptr, |p, o| {
			self.b.ins().load(self.int, MemFlags::new(), p, (o * 8) as i32)
		})
	}

	// Look up the binding that a mutation targets.
	pub(super) fn mutable_local(&self, name: &str, span: Range<usize>, op: Mutation) -> Result<Local, Diagnostic> {
		// how the mutation reads in errors
		// (verb, verb when immutable, noun for the `:=` hint, suggest `:=`?)
		let (verb, immutable_verb, allow, suggest_declare) = match op {
			Mutation::Assign => ("assign to", "assign to", "assignment", true),
			Mutation::IndexAssign => ("assign to", "assign to element of", "assignment", true),
			Mutation::Append => ("append to", "append to", "append", false),
			Mutation::FieldAssign => ("assign field of", "assign field of", "field assignment", false),
		};
		let local = self.vars.get(name).cloned().ok_or_else(|| {
			let d = Diagnostic::new(format!("cannot {verb} undefined variable `{name}`"), span.clone())
				.with_label("not found in scope");
			if suggest_declare {
				d.with_note(format!("declare it first with `{name} := ...`"))
			} else {
				d
			}
		})?;
		if local.stat {
			self.require_pure(name, span.clone())?;
		}
		if !local.mutable {
			return Err(
				Diagnostic::new(format!("cannot {immutable_verb} immutable `{name}`"), span)
					.with_label("immutably bound")
					.with_note(format!("use `{name} := ...` to allow {allow}")),
			);
		}
		Ok(local)
	}

	// Look up a variable.
	pub(super) fn local(&self, name: &str, span: Range<usize>) -> Result<Local, Diagnostic> {
		let local = self.vars.get(name).cloned().ok_or_else(|| {
			if name == "none" {
				Diagnostic::new("cannot infer the type", span.clone()).with_label("`none` needs type context")
			} else {
				Diagnostic::new(format!("undefined variable `{name}`"), span.clone()).with_label("not found in scope")
			}
		})?;
		if local.stat {
			self.require_pure(name, span)?;
		}
		Ok(local)
	}

	// A static reads and writes through its cell.
	pub fn seed_statics(&mut self, inits: &[(String, Span, Option<Spanned<Expr>>)]) -> Result<(), Diagnostic> {
		let cells: Vec<_> = self
			.statics
			.iter()
			.map(|(k, (s, t))| (k.clone(), s.clone(), t.clone()))
			.collect();
		for (key, sym, typ) in cells {
			let addr = self.data_addr(&sym);
			let var = self.b.declare_var(self.int);
			self.b.def_var(var, addr);
			let local = Local {
				var,
				typ,
				mutable: true,
				boxed: true,
				stat: true,
			};
			let bare = self.types.scope.env.iter().find(|(_, q)| **q == key).map(|(b, _)| b.clone());
			if let Some(bare) = bare {
				self.vars.insert(bare, local.clone());
			}
			self.vars.insert(key, local);
		}
		for (key, span, init) in inits {
			let local = self.vars[key].clone();
			let val = match init {
				Some(init) => self.check_typed(init, &local.typ, "does not match the declared type")?,
				None => self.zero_or_err(&local.typ, *span)?,
			};
			// the cell outlives the frame that filled it
			let val = match &local.typ {
				Typ::Struct(..) => self.copy_in(val, &local.typ),
				_ => val,
			};
			self.untemp(val);
			self.write_local(&local, val);
		}
		Ok(())
	}

	// Promote a local to a heap-boxed cell.
	pub(super) fn box_local(&mut self, name: &str, local: &Local, span: Range<usize>) -> Result<Value, Diagnostic> {
		if local.boxed {
			return Ok(self.b.use_var(local.var));
		}
		if !local.mutable {
			return Err(Diagnostic::new(format!("cannot capture `{name}` as `mut`"), span)
				.with_label("immutably bound")
				.with_note(format!("use `{name} := ...` to allow mutation")));
		}
		let cell = self.call_alloc_bytes(8);
		let cur = self.read_local(local);
		self.b.ins().store(MemFlags::new(), cur, cell, 0);
		let var = self.b.declare_var(self.int);
		self.b.def_var(var, cell);
		self.vars.insert(
			name.to_string(),
			Local {
				var,
				boxed: true,
				mutable: true,
				typ: local.typ.clone(),
				stat: local.stat,
			},
		);
		Ok(cell)
	}

	// Read a local's value.
	pub(super) fn read_local(&mut self, local: &Local) -> Value {
		let raw = self.b.use_var(local.var);
		if local.boxed {
			let cl = cl_type(&local.typ, self.int);
			self.b.ins().load(cl, MemFlags::new(), raw, 0)
		} else {
			raw
		}
	}

	// Write a local's value.
	pub(super) fn write_local(&mut self, local: &Local, val: Value) {
		if local.boxed {
			let ptr = self.b.use_var(local.var);
			self.b.ins().store(MemFlags::new(), val, ptr, 0);
		} else {
			self.b.def_var(local.var, val);
		}
	}

	pub(super) fn unit_value(&mut self) -> TypedVal {
		(self.b.ins().iconst(self.int, 0), Typ::unit())
	}

	// `$` implicit input
	// TODO: migrate to its own submodule. idk what to call it yet so putting it here. `sigils`?
	pub(super) fn dollar(&mut self) -> TypedVal {
		self.dollar.clone().expect("`bind_dollar` runs before the body is lowered")
	}

	// Determine type of `$` once params are bound.
	pub fn bind_dollar(&mut self, params_tuple: bool) {
		let locals = self.params.clone();
		let value = if !params_tuple {
			let local = &locals[0];
			(self.read_local(local), local.typ.clone())
		} else if locals.is_empty() {
			self.unit_value()
		} else {
			let ptr = self.call_alloc(locals.len());
			let fields = locals
				.iter()
				.enumerate()
				.map(|(i, local)| {
					let val = self.read_local(local);
					self.b.ins().store(MemFlags::new(), val, ptr, (i * 8) as i32);
					(None, local.typ.clone())
				})
				.collect();
			(ptr, Typ::Tuple(fields))
		};
		self.dollar = Some(value);
	}
}

// Two embeds both supply `wanted`.
fn ambiguous(wanted: &str, a: &str, b: &str, span: Span) -> Diagnostic {
	Diagnostic::new(
		format!("`{wanted}` is ambiguous, found in embedded `{a}` and `{b}`"),
		span.into_range(),
	)
	.with_label("reach it through the embedded struct")
}
