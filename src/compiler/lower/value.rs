use super::generic::unify;
use super::*;
use crate::compiler::role;

// Error when a `@required` field isn't fulfilled.
fn check_required(
	name: &str,
	struct_fields: &[FieldDef],
	entries: &[(Option<String>, Spanned<Expr>)],
	span: Span,
) -> Result<(), Diagnostic> {
	let required = |f: &&FieldDef| {
		f.annotations
			.iter()
			.any(|a| matches!(&a.0, Expr::Ident(n) if n == role::REQUIRED))
	};
	for (i, f) in struct_fields.iter().enumerate().filter(|(_, f)| required(f)) {
		let set = entries.iter().enumerate().any(|(j, (n, v))| {
			matches!(v.0, Expr::Spread(_)) || n.as_deref() == Some(f.name.as_str()) || (n.is_none() && i == j)
		});
		if !set {
			return Err(
				Diagnostic::new(format!("`{name}.{}` is required", f.name), span.into_range())
					.with_label("set it in the literal"),
			);
		}
	}
	Ok(())
}

// A data symbol holding raw bytes, 8-aligned.
pub(crate) fn define_data<M: Module>(m: &mut M, sym: &str, bytes: Vec<u8>) -> DataId {
	let id = m.declare_data(sym, Linkage::Local, false, false).unwrap();
	let mut desc = DataDescription::new();
	desc.set_align(8);
	desc.define(bytes.into_boxed_slice());
	m.define_data(id, &desc).unwrap();
	id
}

// A data symbol holding a pointer, followed by `tail` bytes.
// A generalized str header.
pub(crate) fn define_ptr_data<M: Module>(m: &mut M, sym: &str, target: DataId, tail: &[u8]) -> DataId {
	let mut desc = DataDescription::new();
	desc.set_align(8);
	desc.define([&0i64.to_le_bytes()[..], tail].concat().into_boxed_slice());
	let gv = m.declare_data_in_data(target, &mut desc);
	desc.write_data_addr(0, gv, 0);
	let id = m.declare_data(sym, Linkage::Local, false, false).unwrap();
	m.define_data(id, &desc).unwrap();
	id
}

impl<'a, M: Module> Translator<'a, M> {
	pub(super) fn str_const(&mut self, s: &str) -> Value {
		let len = s.len() as i64;
		let mut bytes = s.as_bytes().to_vec();
		bytes.push(0);
		let sym = format!("__str_{}", *self.string_idx);
		*self.string_idx += 1;
		let bytes_id = define_data(&mut self.module, &format!("{sym}_bytes"), bytes);
		let hdr_sym = format!("{sym}_hdr");
		define_ptr_data(&mut self.module, &hdr_sym, bytes_id, &len.to_le_bytes());
		self.data_addr(&hdr_sym)
	}

	// Intern an atom name to a pointer-sized symbol.
	pub(super) fn atom_const(&mut self, name: &str) -> Value {
		let sym = format!("__atom_{name}");
		let hdr_sym = format!("{sym}_hdr");
		if self.atoms.insert(name.to_string()) {
			let text = format!(":{name}");
			let len = text.len() as i64;
			let mut bytes = text.into_bytes();
			bytes.push(0);
			let bytes_id = define_data(&mut self.module, &sym, bytes);
			define_ptr_data(&mut self.module, &hdr_sym, bytes_id, &len.to_le_bytes());
		}
		self.data_addr(&hdr_sym)
	}

	// A capture-free fn's value.
	pub(crate) fn fn_object(&mut self, id: FuncId) -> Value {
		self.wanted.push(id);
		let mut desc = DataDescription::new();
		desc.set_align(8);
		desc.define(vec![0; 8].into_boxed_slice());
		let fr = self.module.declare_func_in_data(id, &mut desc);
		desc.write_function_addr(0, fr);
		let data = self.module.declare_anonymous_data(false, false).unwrap();
		self.module.define_data(data, &desc).unwrap();
		let gv = self.module.declare_data_in_func(data, self.b.func);
		self.b.ins().symbol_value(self.int, gv)
	}

	// The address of a data symbol.
	pub(super) fn data_addr(&mut self, sym: &str) -> Value {
		let id = self.module.declare_data(sym, Linkage::Local, false, false).unwrap();
		let gv = self.module.declare_data_in_func(id, self.b.func);
		self.b.ins().symbol_value(self.int, gv)
	}

	pub(super) fn zero(&mut self, typ: &Typ) -> Value {
		let typ = typ.newtype().unwrap_or(typ);
		match typ {
			Typ::Float(16) => self.b.ins().f16const(Ieee16::with_bits(0)),
			Typ::Float(32) => self.b.ins().f32const(0.0),
			Typ::Float(64) => self.b.ins().f64const(0.0),
			Typ::Float(128) => {
				let c = self.b.func.dfg.constants.insert(Ieee128::with_bits(0).into());
				self.b.ins().f128const(c)
			}
			Typ::Float(w) => panic!("unsupported float width f{w}"),
			Typ::Str => self.str_const(""),
			Typ::Error => {
				let msg = self.str_const("");
				self.box_error(msg, &Typ::Str)
			}
			Typ::Atom => self.atom_const(""),
			Typ::Int(w) => self.b.ins().iconst(cl_type(&Typ::Int(*w), self.int), 0),
			Typ::UInt(w) => self.b.ins().iconst(cl_type(&Typ::UInt(*w), self.int), 0),
			Typ::Bool | Typ::ISize | Typ::USize | Typ::CStr => self.b.ins().iconst(self.int, 0),
			Typ::Fn(_, ret) => {
				// call `core::zero[ret]`
				let def = self.generic_fns[role::ZERO].clone();
				let subst = HashMap::from([(def.type_params[0].name.clone(), (**ret).clone())]);
				let Ok(sig) = self.declare_instance(role::ZERO, &def, subst) else {
					unreachable!("core::zero instance")
				};
				self.fn_object(sig.id)
			}
			Typ::Annotated(..) | Typ::Closure(..) | Typ::Trait(_) | Typ::Ref(_) => self.b.ins().iconst(self.int, 0),
			Typ::Access(..) => unreachable!("an access mod only marks params inside a fn"),
			Typ::Option(inner) => self.make_option(inner, None),
			// default to first variant, with zero'd payload fields
			Typ::Enum(_) | Typ::Result(..) | Typ::Sum(..) => {
				let variants = self.variants_of(typ);
				let v = variants.first().cloned();
				let disc = v.as_ref().map_or(0, |v| v.disc);
				let fields: Vec<Value> =
					v.map(|v| v.payload.iter().map(|t| self.zero(t)).collect()).unwrap_or_default();
				self.make_enum(&variants, disc, &fields)
			}
			_ if typ.is_unit() => self.b.ins().iconst(self.int, 0),
			Typ::Struct(_, fields) => {
				let fields = fields.clone();
				let ptr = self.stack_slot((fields.len() * 8) as u32);
				for (i, f) in fields.iter().enumerate() {
					let z = self.zero(&f.typ);
					self.b.ins().store(MemFlags::new(), z, ptr, (i * 8) as i32);
				}
				ptr
			}
			Typ::TupleStruct(_, fields) => self.zero(&Typ::Tuple(fields.clone())),
			Typ::Tuple(fields) => {
				let fields = fields.clone();
				let ptr = self.call_alloc(fields.len());
				for (i, (_, ftyp)) in fields.iter().enumerate() {
					let z = self.zero(ftyp);
					self.b.ins().store(MemFlags::new(), z, ptr, (i * 8) as i32);
				}
				ptr
			}
			Typ::Array(_) => {
				let z = self.b.ins().iconst(self.int, 0);
				self.make_array(z, z, typ)
			}
			Typ::FixedArray(elem, n) => {
				let elem = (**elem).clone();
				let stride = self.elem_stride(&elem);
				let ptr = self.stack_slot((*n as i64 * stride) as u32);
				for i in 0..*n {
					let z = self.zero(&elem);
					self.store_elem(ptr, (i as i64 * stride) as i32, &elem, z);
				}
				ptr
			}
			Typ::Any => {
				let ptr = self.call_alloc(2);
				let z = self.b.ins().iconst(self.int, 0);
				self.b.ins().store(MemFlags::new(), z, ptr, 0);
				self.b.ins().store(MemFlags::new(), z, ptr, 8);
				ptr
			}
			Typ::Map(..) => {
				let m = self.call_map_new();
				self.temp(m, typ);
				m
			}
			Typ::Ast => self.b.ins().iconst(self.int, 0),
		}
	}

	// A numeric literal takes the binding's declared type directly.
	pub(super) fn coerce_lit(&mut self, value: &Spanned<Expr>, target: &Typ) -> Result<Option<Value>, Diagnostic> {
		let (neg, inner) = match &value.0 {
			Expr::Negative(e) => (true, &e.0),
			v => (false, v),
		};
		let oob = |n| {
			Diagnostic::new(format!("{n} is out of range for {target}"), value.1.into_range())
				.with_label(format!("doesn't fit in {target}"))
		};
		let v = match (inner, target) {
			(Expr::Int(n), Typ::Int(w)) => {
				let n = if neg { -*n } else { *n };
				if n < int_min(*w) || n > int_max(*w) {
					return Err(oob(n));
				}
				self.b.ins().iconst(cl_int_for_width(*w), n)
			}
			(Expr::Int(n), Typ::UInt(w)) => {
				let n = if neg { -*n } else { *n };
				if n < 0 || (*w < 64 && n > uint_max(*w)) {
					return Err(oob(n));
				}
				self.b.ins().iconst(cl_int_for_width(*w), n)
			}
			(Expr::Int(n), Typ::ISize) => self.b.ins().iconst(self.int, if neg { -*n } else { *n }),
			(Expr::Int(n), Typ::USize) => {
				let n = if neg { -*n } else { *n };
				if n < 0 {
					return Err(oob(n));
				}
				self.b.ins().iconst(self.int, n)
			}
			(Expr::Int(n), Typ::Float(w)) => self.float_lit((if neg { -*n } else { *n }) as f64, *w, value.1)?,
			(Expr::Float(x), Typ::Float(w)) => self.float_lit(if neg { -*x } else { *x }, *w, value.1)?,
			(Expr::Atom(name), Typ::Enum(typ)) => self.construct_variant(typ, name, &[], value.1)?.0,
			(Expr::EnumShorthand { variant, args }, Typ::Enum(typ)) => {
				self.construct_variant(typ, variant, args, value.1)?.0
			}
			(Expr::None, Typ::Option(inner)) => self.make_option(inner, None),
			(Expr::String(s), Typ::CStr) => {
				let mut bytes = s.as_bytes().to_vec();
				bytes.push(0);
				let sym = format!("__str_{}", *self.string_idx);
				*self.string_idx += 1;
				define_data(&mut self.module, &sym, bytes);
				self.data_addr(&sym)
			}
			(Expr::Atom(name), Typ::Sum(..)) => {
				let variants = self.variants_of(target);
				let Some(v) = variants.iter().find(|v| &v.name == name && v.payload.is_empty()) else {
					return Err(
						Diagnostic::new(format!("`{target}` has no atom `:{name}`"), value.1.into_range())
							.with_label("not a member of this sum type"),
					);
				};
				self.make_enum(&variants, v.disc, &[])
			}
			(Expr::EnumShorthand { variant, .. } | Expr::Atom(variant), Typ::Result(ok, err)) => {
				let has =
					|t: &Typ| matches!(t, Typ::Enum(n) if self.enum_variants(n).iter().any(|v| v.name == *variant));
				let Some((disc, side)) = [ok, err].into_iter().enumerate().find(|(_, t)| has(t)) else {
					return Ok(None);
				};
				match self.coerce_lit(value, side)? {
					Some(v) => self.make_enum(&result_variants(ok, err), disc as i64, &[v]),
					None => return Ok(None),
				}
			}
			(_, Typ::Option(inner)) => match self.coerce_lit(value, inner)? {
				Some(v) => self.make_option(inner, Some(v)),
				None => return Ok(None),
			},
			_ => return Ok(None),
		};
		Ok(Some(v))
	}

	// The variant table of a named enum.
	pub(super) fn enum_variants(&self, name: &str) -> Vec<VariantInfo> {
		self.enums
			.get(name)
			.cloned()
			.or_else(|| self.generics.instances.borrow().get(name).cloned())
			.unwrap_or_default()
	}

	// See through `&T`.
	pub(super) fn peeled(&self, typ: &Typ) -> Typ {
		match peel(typ) {
			Typ::Struct(n, f) if f.is_empty() => {
				Typ::Struct(n.clone(), self.structs.get(n).cloned().unwrap_or_default())
			}
			t => t.clone(),
		}
	}

	// Variant table for any type that carries variants.
	pub(super) fn variants_of(&self, typ: &Typ) -> Vec<VariantInfo> {
		match typ {
			Typ::Enum(name) => self.enum_variants(name),
			Typ::Option(inner) => option_variants(inner),
			Typ::Result(ok, err) => result_variants(ok, err),
			Typ::Sum(name, _) if !name.is_empty() => self.types().named_sum(name, Span::default()).unwrap_or_default(),
			Typ::Sum(_, variants) => variants.clone(),
			_ => Vec::new(),
		}
	}

	// Pick the (first) sum member that matches the hint.
	pub(super) fn through_sum(&self, hint: Option<&Typ>, pick: impl Fn(&Typ) -> bool) -> Option<Typ> {
		let Some(sum @ Typ::Sum(..)) = hint else {
			return hint.cloned();
		};
		let hits: Vec<_> = self
			.variants_of(sum)
			.into_iter()
			.filter(|v| v.payload.first().is_some_and(&pick))
			.collect();
		let [v] = &hits[..] else { return None };
		Some(v.payload[0].clone())
	}

	// The tag of an enum value.
	pub(super) fn enum_tag(&mut self, typ: &Typ, val: Value) -> Value {
		if *typ == Typ::Any {
			self.b.ins().load(self.int, MemFlags::new(), val, 0)
		} else if rc::opt_ref(typ) {
			let nz = self.b.ins().icmp_imm(IntCC::NotEqual, val, 0);
			self.b.ins().uextend(self.int, nz)
		} else if enum_boxed(&self.variants_of(typ)) {
			self.b.ins().load(self.int, MemFlags::new(), val, 0)
		} else {
			val
		}
	}

	// Build an Option value.
	pub(super) fn make_option(&mut self, inner: &Typ, some: Option<Value>) -> Value {
		if matches!(inner, Typ::Ref(_)) {
			return some.unwrap_or_else(|| self.b.ins().iconst(self.int, 0));
		}
		let variants = option_variants(inner);
		match some {
			Some(v) => self.make_enum(&variants, 1, &[v]),
			None => self.make_enum(&variants, 0, &[]),
		}
	}

	// A payload slot of a variant.
	pub(super) fn opt_payload(&mut self, val: Value, typ: &Typ, inner: &Typ, off: i32) -> Value {
		if rc::opt_ref(typ) {
			val
		} else {
			self.b.ins().load(cl_type(inner, self.int), MemFlags::new(), val, off)
		}
	}

	// Build a variant value.
	// A bare discriminant for fieldless enums, and a heap where that's not possible.
	pub(super) fn make_enum(&mut self, variants: &[VariantInfo], disc: i64, fields: &[Value]) -> Value {
		let slots = enum_slots(variants);
		if slots == 1 {
			return self.b.ins().iconst(self.int, disc);
		}
		let ptr = self.call_alloc(slots);
		let tag = self.b.ins().iconst(self.int, disc);
		self.b.ins().store(MemFlags::new(), tag, ptr, 0);
		let payload = variants
			.iter()
			.find(|v| v.disc == disc)
			.map(|v| v.payload.as_slice())
			.unwrap_or(&[]);
		for (i, fv) in fields.iter().enumerate() {
			let fv = match payload.get(i) {
				Some(t) => self.copy_in(*fv, t),
				None => *fv,
			};
			self.b.ins().store(MemFlags::new(), fv, ptr, ((i + 1) * 8) as i32);
		}
		ptr
	}

	// A match pattern's discriminant and payload binds.
	pub(super) fn enum_pattern(&self, pat: &Spanned<Expr>, typ: &Typ) -> Result<(i64, Vec<Bind>), Diagnostic> {
		let bad = |msg| Err(Diagnostic::new(msg, pat.1.into_range()).with_label("bad pattern"));
		if let (Typ::Sum(..), Some(te)) = (typ, TypeExpr::from_expr(&pat.0)) {
			let (variants, disp) = (self.variants_of(typ), self.sum_display(&te, pat.1)?);
			return match variants.iter().find(|x| x.name == disp) {
				Some(x) => Ok((x.disc, vec![])),
				None => bad(format!("`{typ}` has no variant `{disp}`")),
			};
		}
		if let (Typ::Any, Expr::Ident(v)) = (typ, &pat.0) {
			let t = self.types().resolve(&TypeExpr::Name(v.clone()), pat.1)?;
			return Ok((typeid(&t), vec![]));
		}
		let (variant, args): (&str, &[Spanned<Expr>]) = match &pat.0 {
			Expr::EnumShorthand { variant, args } => (variant, args),
			Expr::Atom(v) => (v, &[]),
			Expr::Field { tuple, field } if matches!(tuple.0, Expr::Ident(_)) => (field, &[]),
			_ => return bad(format!("`{typ}` is matched by its variants")),
		};
		let variants = self.variants_of(typ);
		let Some(v) = variants.iter().find(|v| v.name == variant) else {
			return bad(format!("`{typ}` has no variant `{variant}`"));
		};
		let binds = if let [(Expr::Record(entries), _)] = args {
			let mut binds = Vec::with_capacity(entries.len());
			for (k, val) in entries {
				let (Expr::Ident(field), Expr::Ident(local)) = (&k.0, &val.0) else {
					return bad("field patterns bind names".into());
				};
				let Some(idx) = v.names.iter().position(|n| n == field) else {
					return bad(format!("`{variant}` has no field `{field}`"));
				};
				binds.push((local.clone(), v.payload[idx].clone(), 8 + idx as i32 * 8));
			}
			binds
		} else {
			field_binds(args.iter().zip(&v.payload), 8, 8)?
		};
		Ok((v.disc, binds))
	}

	// The display name a type pattern refers to.
	// ex: `string` -> `str`.
	fn sum_display(&self, te: &TypeExpr, span: Span) -> Result<String, Diagnostic> {
		match (self.types().resolve(te, span), te) {
			(Ok(t), _) => Ok(t.to_string()),
			(Err(_), TypeExpr::Name(n)) => Ok(n.clone()),
			(Err(e), _) => Err(e),
		}
	}

	// An `n @ int` arm on a sum captures the unwrapped payload at offset 8.
	pub(super) fn sum_capture(&self, arm: &MatchArm, st: &Typ) -> Option<Bind> {
		let name = arm.binding.as_ref()?;
		let [pat] = arm.patterns.as_slice() else { return None };
		let te = TypeExpr::from_expr(&pat.0)?;
		if *st == Typ::Any {
			let t = self.types().resolve(&te, pat.1).ok()?;
			return Some((name.clone(), t, 8));
		}
		let Typ::Sum(..) = st else { return None };
		let (variants, disp) = (self.variants_of(st), self.sum_display(&te, pat.1).ok()?);
		let vi = variants.iter().find(|x| x.name == disp && x.payload.len() == 1)?;
		Some((name.clone(), vi.payload[0].clone(), 8))
	}

	pub(super) fn make_range(
		&mut self,
		start: Value,
		end: Option<Value>,
		step: Value,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let typ = self.types().resolve(&TypeExpr::Name(role::RANGE.into()), span)?;
		let Typ::Struct(_, fields) = &typ else {
			unreachable!("core `Range` is a struct")
		};
		let fields = fields.clone();
		let ptr = self.struct_slot(&fields)?;
		let end = end.map(|v| self.intcast(v, types::I64, true));
		let end = self.make_option(&Typ::Int(64), end);
		let start = self.intcast(start, types::I64, true);
		let step = self.intcast(step, types::I64, true);
		for (i, v) in [start, end, step].into_iter().enumerate() {
			self.b.ins().store(MemFlags::new(), v, ptr, i as i32 * 8);
		}
		self.temp(ptr, &typ);
		Ok((ptr, typ))
	}

	// (start, end, step, open)
	pub(super) fn range_parts(&mut self, range: Value) -> (Value, Value, Value, Value) {
		let cl = cl_int_for_width(64);
		let start = self.b.ins().load(cl, MemFlags::new(), range, 0);
		let opt = self.b.ins().load(self.int, MemFlags::new(), range, 8);
		let step = self.b.ins().load(cl, MemFlags::new(), range, 16);
		let opt_typ = Typ::Option(Box::new(Typ::Int(64)));
		let tag = self.enum_tag(&opt_typ, opt);
		let open = self.b.ins().icmp_imm(IntCC::Equal, tag, 0);
		let end = self.opt_payload(opt, &opt_typ, &Typ::Int(64), 8);
		(start, end, step, open)
	}

	pub(super) fn range_value(
		&mut self,
		start: &Spanned<Expr>,
		end: Option<&Spanned<Expr>>,
		inclusive: bool,
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let bad = |msg: &str, label: &str| Diagnostic::new(msg, start.1.into_range()).with_label(label);
		let (start, second) = match &start.0 {
			Expr::Range {
				start: first,
				end: Some(second),
				inclusive: inner,
			} => {
				if *inner {
					return Err(bad("`..=` goes on the last operator only", "not this one"));
				}
				if matches!(first.0, Expr::Range { .. }) {
					return Err(bad("a range takes one step", "`a..step..b` is as deep as this nests"));
				}
				if let (Expr::Int(a), Expr::Int(b)) = (&first.0, &second.0)
					&& a == b
				{
					return Err(bad("a range step of 0 never advances", "same as the first value"));
				}
				(&**first, Some(&**second))
			}
			_ => (start, None),
		};
		let start_val = self.int_value(start, "range start")?;
		let step = match second {
			Some(second) => {
				let v = self.int_value(second, "range step")?;
				self.b.ins().isub(v, start_val)
			}
			None => self.b.ins().iconst(types::I64, 1),
		};
		let end = end
			.map(|e| {
				let v = self.int_value(e, "range end")?;
				Ok(if inclusive { self.b.ins().iadd(v, step) } else { v })
			})
			.transpose()?;
		self.make_range(start_val, end, step, span)
	}

	pub(super) fn range_pattern(
		&mut self,
		sv: Value,
		st: &Typ,
		(start, end, inclusive): Bounds,
		span: Span,
	) -> Result<Value, Diagnostic> {
		let Typ::Int(_) = st else {
			let msg = format!("range patterns need an integer subject, got {st}");
			return Err(Diagnostic::new(msg, span.into_range()).with_label("not an integer"));
		};
		let zero = (Expr::Int(0), span);
		let (range, _) = self.range_value(start.unwrap_or(&zero), end, inclusive, span)?;
		let sv = self.intcast(sv, types::I64, true);
		let hit = self.range_contains(range, sv, span)?;
		Ok(self.b.ins().icmp_imm(IntCC::NotEqual, hit, 0))
	}

	pub(super) fn range_contains(&mut self, range: Value, n: Value, span: Span) -> Result<Value, Diagnostic> {
		let sig = self.funcs.get(role::RANGE_CONTAINS).cloned().ok_or_else(|| {
			Diagnostic::new(format!("core is missing `{}`", role::RANGE_CONTAINS), span.into_range())
				.with_label("required to test range membership")
		})?;
		Ok(self.emit_call(&sig, &[range, n]).0)
	}

	// Make and check enum variant.
	pub(super) fn construct_variant(
		&mut self,
		name: &str,
		variant: &str,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let variants = self.enum_variants(name);
		let v = variants.iter().find(|v| v.name == variant).ok_or_else(|| {
			Diagnostic::new(format!("enum `{name}` has no variant `{variant}`"), span.into_range())
				.with_label("no such variant")
		})?;
		let (disc, payload, names) = (v.disc, v.payload.clone(), v.names.clone());
		let fields = if !names.is_empty() {
			let [(Expr::Record(entries), _)] = args else {
				let msg = format!("`{name}.{variant}` takes named fields");
				return Err(Diagnostic::new(msg, span.into_range()).with_label("use `{ field: value }`"));
			};
			let mut fields: Vec<Value> = payload.iter().map(|t| self.zero(t)).collect();
			for (k, val) in entries {
				let Expr::Ident(key) = &k.0 else {
					return Err(
						Diagnostic::new("field names must be idents", k.1.into_range()).with_label("not a field name")
					);
				};
				let idx = names.iter().position(|n| n == key).ok_or_else(|| {
					Diagnostic::new(format!("`{name}.{variant}` has no field `{key}`"), k.1.into_range())
						.with_label("no such field")
				})?;
				fields[idx] = self.check_typed(val, &payload[idx], "type mismatch")?;
				self.move_resource(val, &payload[idx])?;
			}
			fields
		} else {
			if args.len() != payload.len() {
				let msg = format!(
					"`{name}.{variant}` takes {} field(s), got {}",
					payload.len(),
					args.len()
				);
				return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of fields"));
			}
			let mut fields = Vec::with_capacity(args.len());
			for (arg, ft) in args.iter().zip(&payload) {
				fields.push(self.check_typed(arg, ft, "type mismatch")?);
				self.move_resource(arg, ft)?;
			}
			fields
		};
		let val = self.make_enum(&variants, disc, &fields);
		let typ = Typ::Enum(name.to_string());
		self.temp(val, &typ);
		Ok((val, typ))
	}

	// Tuple struct constructor.
	pub(super) fn construct_tuple_struct(
		&mut self,
		name: &str,
		args: &[Spanned<Expr>],
		span: Span,
	) -> Result<TypedVal, Diagnostic> {
		let typ = self.types().resolve(&TypeExpr::Name(name.to_string()), span)?;
		let Typ::TupleStruct(_, fields) = typ.clone() else {
			unreachable!("caller checked the alias");
		};
		let mut slots: Vec<Option<&Spanned<Expr>>> = vec![None; fields.len()];
		if let [(Expr::Record(entries), _)] = args
			&& fields.iter().any(|(n, _)| n.is_some())
		{
			for (k, val) in entries {
				let Expr::Ident(key) = &k.0 else {
					return Err(
						Diagnostic::new("field names must be idents", k.1.into_range()).with_label("not a field name")
					);
				};
				let idx = fields.iter().position(|(n, _)| n.as_deref() == Some(key)).ok_or_else(|| {
					Diagnostic::new(format!("`{name}` has no field `{key}`"), k.1.into_range())
						.with_label("no such field")
				})?;
				slots[idx] = Some(val);
			}
		} else if args.len() == fields.len() {
			slots = args.iter().map(Some).collect();
		} else {
			let msg = format!("`{name}` takes {} field(s), got {}", fields.len(), args.len());
			return Err(Diagnostic::new(msg, span.into_range()).with_label("wrong number of fields"));
		}
		let mut vals = Vec::with_capacity(fields.len());
		for ((_, ft), slot) in fields.iter().zip(&slots) {
			vals.push(match *slot {
				Some(arg) if name == role::PTR => self.ptr_arg(arg)?,
				Some(arg) => self.check_typed(arg, ft, "type mismatch")?,
				None => self.zero(ft),
			});
		}
		if let [v] = vals[..] {
			return Ok((v, typ));
		}
		let ptr = self.call_alloc(fields.len());
		for (i, v) in vals.into_iter().enumerate() {
			self.b.ins().store(MemFlags::new(), v, ptr, (i * 8) as i32);
		}
		Ok((ptr, typ))
	}

	// An expr checked against an expected type.
	pub(super) fn check_typed(&mut self, arg: &Spanned<Expr>, ft: &Typ, label: &str) -> Result<Value, Diagnostic> {
		let (fv, at) = self.check_expr(arg, ft)?;
		if at != *ft {
			let msg = format!("expected {ft}, got {at}");
			return Err(Diagnostic::new(msg, arg.1.into_range()).with_label(label));
		}
		Ok(fv)
	}

	// Evaluate `value` against an expected type.
	// Coerces variant shorthands, atoms, and `none`.
	pub(super) fn check_expr(&mut self, value: &Spanned<Expr>, target: &Typ) -> Result<TypedVal, Diagnostic> {
		if let Typ::Annotated(anns, inner) = target
			&& let Typ::Fn(ps, _) = &**inner
		{
			let value = match &value.0 {
				Expr::Annotated(a, v) if ann_names(self.scope, a) == *anns => v,
				_ => value,
			};
			let (val, vt) = self.check_expr(value, inner)?;
			if vt != **inner {
				return Ok((val, vt));
			}
			check_ann_typ(anns, &vt, value.1)?;
			if ps.iter().any(|p| matches!(p.typ, Typ::Fn(..))) && !self.c_callable(Some(value)) {
				let msg = "this fn is called from C";
				return Err(Diagnostic::new(msg, value.1.into_range()).with_label("mark it with `@c`"));
			}
			return Ok((self.b.ins().load(self.int, MemFlags::new(), val, 0), target.clone()));
		}
		let (val, vt) = self.lower(value, Some(target))?;
		self.coerce(val, &vt, target, value.1)
	}

	// Convert an already-lowered value to the type expected, or hand it back unchanged.
	pub(super) fn coerce(&mut self, val: Value, from: &Typ, to: &Typ, span: Span) -> Result<TypedVal, Diagnostic> {
		if from == to {
			return Ok((val, from.clone()));
		}
		if let Typ::Trait(tn) = to {
			return self.make_trait_object(val, from, tn, span);
		}
		if *to == Typ::Error && self.open_error(from) {
			return Ok((self.box_error(val, from), Typ::Error));
		}
		if *to == Typ::Any {
			let id = typeid(from);
			let v = VariantInfo::new(from.key(), id, vec![from.clone()]);
			return Ok((self.make_enum(&[v], id, &[val]), Typ::Any));
		}
		if let Typ::Annotated(_, inner) = from
			&& **inner == *to
		{
			return Ok((self.fn_cell(val), to.clone()));
		}
		// fixed arrays widen to dynamic
		if let (Typ::FixedArray(e, n), Typ::Array(t)) = (from, to)
			&& e == t
		{
			return Ok((self.fixed_to_array(val, e, *n), to.clone()));
		}
		// same member sets with different order
		if let (Typ::Sum(..), Typ::Sum(..)) = (from, to) {
			let (src, dst) = (self.variants_of(from), self.variants_of(to));
			let Some(map) = sum_remap(&src, &dst) else {
				return Ok((val, from.clone()));
			};
			let old = self.enum_tag(from, val);
			let mut tag = self.b.ins().iconst(self.int, map[0].1);
			for &(s, d) in &map[1..] {
				let hit = self.b.ins().icmp_imm(IntCC::Equal, old, s);
				let dv = self.b.ins().iconst(self.int, d);
				tag = self.b.ins().select(hit, dv, tag);
			}
			if !enum_boxed(&dst) {
				return Ok((tag, to.clone()));
			}
			let slots = enum_slots(&dst);
			let ptr = self.call_alloc(slots);
			self.b.ins().store(MemFlags::new(), tag, ptr, 0);
			for i in 1..slots {
				let w = self.b.ins().load(self.int, MemFlags::new(), val, (i * 8) as i32);
				self.b.ins().store(MemFlags::new(), w, ptr, (i * 8) as i32);
			}
			return Ok((ptr, to.clone()));
		}
		// values widens into options
		if let Typ::Option(inner) = to
			&& **inner == *from
		{
			return Ok((self.make_option(inner, Some(val)), to.clone()));
		}
		if let Typ::Sum(..) = to {
			let variants = self.variants_of(to);
			if let Some(v) = variants.iter().find(|v| v.payload == [from.clone()]) {
				return Ok((self.make_enum(&variants, v.disc, &[val]), to.clone()));
			}
		}
		Ok((val, from.clone()))
	}

	// Box a struct behind its vtable.
	fn make_trait_object(&mut self, val: Value, vt: &Typ, tn: &str, span: Span) -> Result<TypedVal, Diagnostic> {
		// already the right trait object, pass it through
		if matches!(vt, Typ::Trait(n) if n == tn) {
			return Ok((val, vt.clone()));
		}
		let Typ::Struct(name, _) = vt else {
			return Err(
				Diagnostic::new("only structs can be trait objects yet", span.into_range())
					.with_label(format!("`{vt}` is not a struct")),
			);
		};
		if !self.trait_impls.contains(&(name.clone(), tn.to_string())) {
			return Err(
				Diagnostic::new(format!("`{name}` doesn't implement `{tn}`"), span.into_range())
					.with_label("no matching impl"),
			);
		}
		Ok((self.box_trait_object(val, name, tn), Typ::Trait(tn.to_string())))
	}

	// Whether `typ` claims std `Error`, boxing into the open `Error` type.
	pub(super) fn open_error(&self, typ: &Typ) -> bool {
		self.trait_impls.contains(&(typ.key(), role::ERROR.to_string()))
	}

	// Box a claimer of `Error` behind its vtable.
	pub(super) fn box_error(&mut self, val: Value, typ: &Typ) -> Value {
		self.box_trait_object(val, &typ.key(), role::ERROR)
	}

	// Box `val` (an instance of `name`) behind its `name`/`tn` vtable.
	pub(super) fn box_trait_object(&mut self, val: Value, name: &str, tn: &str) -> Value {
		let sym = oi_symbol(&format!("vtable_{name}_{tn}"));
		let vtable = self.data_addr(&sym);
		let boxp = self.call_alloc(2);
		self.b.ins().store(MemFlags::new(), vtable, boxp, 0);
		self.b.ins().store(MemFlags::new(), val, boxp, 8);
		boxp
	}

	pub(super) fn float_lit(&mut self, x: f64, w: u16, span: Span) -> Result<Value, Diagnostic> {
		match w {
			32 => Ok(self.b.ins().f32const(x as f32)),
			64 => Ok(self.b.ins().f64const(x)),
			_ => Err(Diagnostic::new(
				format!("f{w} literals aren't supported by the JIT backend yet"),
				span.into_range(),
			)
			.with_label("not yet implemented")),
		}
	}

	// Record literal, lowered to a map.
	pub(super) fn record_lit(
		&mut self,
		entries: &[(Spanned<Expr>, Spanned<Expr>)],
		span: Span,
		target: Option<&Typ>,
	) -> Result<TypedVal, Diagnostic> {
		let entries: Vec<_> = entries
			.iter()
			.map(|(k, v)| match &k.0 {
				Expr::Ident(n) => ((Expr::String(n.clone()), k.1), v.clone()),
				_ => (k.clone(), v.clone()),
			})
			.collect();
		self.map_lit(&entries, span, target)
	}

	pub(super) fn map_lit(
		&mut self,
		entries: &[(Spanned<Expr>, Spanned<Expr>)],
		span: Span,
		target: Option<&Typ>,
	) -> Result<TypedVal, Diagnostic> {
		let (key_typ, mut val_typ, mut first_bits) = match target {
			Some(Typ::Map(k, v)) => ((**k).clone(), Some((**v).clone()), None),
			_ => match entries.first() {
				Some((first_key, _)) => {
					let (kv, kt) = self.expr(first_key)?;
					let tag = map_key_tag(&kt).ok_or_else(|| {
						Diagnostic::new(format!("{kt} cannot be used as a map key"), first_key.1.into_range())
							.with_label("unsupported key type")
					})?;
					(kt, None, Some((tag, self.map_bits(kv))))
				}
				None => {
					return Err(Diagnostic::new("cannot infer the type of `[]` here", span.into_range())
						.with_label("no map type is expected in this position"));
				}
			},
		};
		let map = self.call_map_new();
		for (key, value) in entries {
			let (tag, key_bits) = match first_bits.take() {
				Some(tb) => tb,
				None => self.map_key(key, &key_typ)?,
			};
			let (val, vt) = match &val_typ {
				Some(t) => (self.check_typed(value, t, "type mismatch")?, t.clone()),
				None => self.expr(value)?,
			};
			closure_escape(&vt, value.1.into_range(), "stored in a map")?;
			self.move_resource(value, &vt)?;
			let val = if self.is_copy(&vt) { self.copy_in(val, &vt) } else { val };
			self.untemp(val);
			val_typ.get_or_insert(vt);
			let bits = self.map_bits(val);
			self.call_map_set(map, tag, key_bits, bits);
		}
		let val_typ = val_typ.expect("target or first entry set it");
		let typ = Typ::Map(Box::new(key_typ), Box::new(val_typ));
		self.temp(map, &typ);
		Ok((map, typ))
	}

	// Struct literal.
	// `Name {}`
	pub(super) fn struct_lit(
		&mut self,
		name: &str,
		type_args: &[Spanned<TypeExpr>],
		fields: &[(Option<String>, Spanned<Expr>)],
		span: Span,
		target: Option<&Typ>,
	) -> Result<TypedVal, Diagnostic> {
		for (i, (fname, value)) in fields.iter().enumerate() {
			// ensure no duplicate named fields
			if let Some(fname) = fname
				&& fields[..i].iter().any(|(n, _)| n.as_ref() == Some(fname))
			{
				return Err(
					Diagnostic::new(format!("`{fname}` is repeated"), value.1.into_range()).with_label("repeated")
				);
			}
		}
		// `Self {}` inside a method resolves to the impl's type
		let mut name = match name {
			"" => match target {
				Some(Typ::Struct(n, _)) => n.clone(),
				// anonymous structs
				_ if fields.iter().all(|f| f.0.is_some()) => return self.infer_anon(fields),
				_ => {
					return Err(
						Diagnostic::new("cannot infer the struct type of `.{}` here", span.into_range())
							.with_label("name the literal: `Name.{ ... }`"),
					);
				}
			},
			"Self" => self.self_type.clone().ok_or_else(|| {
				Diagnostic::new("`Self` is only valid in an impl block", span.into_range())
					.with_label("no enclosing impl")
			})?,
			_ => self.qualify(name).to_string(),
		};
		if self.enums.contains_key(name.as_str()) {
			if !fields.is_empty() {
				return Err(Diagnostic::new(
					format!("enum `{name}` only supports `{name}.{{}}` with no fields"),
					span.into_range(),
				)
				.with_label("not a struct"));
			}
			let typ = Typ::Enum(name.clone());
			return Ok((self.zero(&typ), typ));
		}
		// explicit generics
		let mut explicit = None;
		if !type_args.is_empty() {
			let args = type_args.iter().map(|t| t.0.clone()).collect();
			if let Typ::Struct(n, fs) = self.types().resolve(&TypeExpr::Generic(name.clone(), args), span)? {
				(name, explicit) = (n, Some(fs));
			}
		}
		// anonymous structs are named by their shape
		let anon = target.and_then(|t| match t {
			Typ::Struct(n, fs) if *n == name => Some(fs.clone()),
			_ => None,
		});
		let struct_fields = match explicit.or(anon).or_else(|| self.structs.get(name.as_str()).cloned()) {
			Some(fields) => fields,
			None => match self.generics.structs.get(name.as_str()).cloned() {
				Some(def) => return self.generic_struct_lit(&name, def, fields, span, target),
				None => {
					return Err(Diagnostic::new(format!("unknown struct `{name}`"), span.into_range())
						.with_label("not defined"));
				}
			},
		};
		let ptr = self.struct_slot(&struct_fields)?;

		let arity = |got: usize| {
			Diagnostic::new(
				format!(
					"`{name}` has {} fields but {got} values were provided",
					struct_fields.len()
				),
				span.into_range(),
			)
			.with_label("wrong number of fields")
		};
		let mut prefix = 0;
		for (i, (field_name, value)) in fields.iter().enumerate() {
			// struct update
			if let Expr::Spread(src) = &value.0 {
				if prefix > 0 {
					return Err(Diagnostic::new("spread requires named fields", span.into_range())
						.with_label("`..` cannot be mixed with positional values"));
				}
				let (val, typ) = self.expr(src)?;
				if !matches!(&typ, Typ::Struct(n, _) if *n == name) {
					return Err(
						Diagnostic::new(format!("cannot spread {typ} into `{name}`"), src.1.into_range())
							.with_label("type mismatch"),
					);
				}
				self.assign_fields(val, ptr, &struct_fields, false);
				continue;
			}
			let (idx, ftyp, base) = match field_name.as_deref() {
				None if i != prefix => {
					return Err(
						Diagnostic::new("positional fields go before named fields", value.1.into_range())
							.with_label("positional field after a named one"),
					);
				}
				None if i >= struct_fields.len() => return Err(arity(fields.len())),
				None => {
					self.check_member(&name, &struct_fields[i].name, value.1)?;
					prefix += 1;
					(i, struct_fields[i].typ.clone(), ptr)
				}
				Some(fname) => match struct_fields.iter().position(|f| f.name == fname) {
					Some(idx) if idx < prefix => {
						return Err(Diagnostic::new(
							format!("`{fname}` was already set positionally"),
							value.1.into_range(),
						)
						.with_label("set twice"));
					}
					Some(idx) => {
						self.check_member(&name, fname, value.1)?;
						(idx, struct_fields[idx].typ.clone(), ptr)
					}
					None => match self.promoted(&struct_fields, fname, value.1)? {
						Some((path, inner, ftyp)) => (inner, ftyp, self.follow(ptr, &path)),
						None => {
							return Err(Diagnostic::new(
								format!("`{name}` has no field `{fname}`"),
								value.1.into_range(),
							)
							.with_label("no such field"));
						}
					},
				},
			};
			let val = self.check_typed(value, &ftyp, "type mismatch")?;
			self.move_resource(value, &ftyp)?;
			let val = self.copy_in(val, &ftyp);
			self.b.ins().store(MemFlags::new(), val, base, (idx * 8) as i32);
		}
		check_required(&name, &struct_fields, fields, span)?;
		let typ = Typ::Struct(name.clone(), struct_fields);
		self.temp(ptr, &typ);
		Ok((ptr, typ))
	}

	// A self-typed anonymous struct, named by its shape.
	fn infer_anon(&mut self, entries: &[(Option<String>, Spanned<Expr>)]) -> Result<TypedVal, Diagnostic> {
		let ptr = self.stack_slot((entries.len() * 8) as u32);
		let mut fields: Vec<FieldDef> = Vec::with_capacity(entries.len());
		for (i, (name, value)) in entries.iter().enumerate() {
			let (val, typ) = self.expr(value)?;
			let val = self.copy_in(val, &typ);
			self.b.ins().store(MemFlags::new(), val, ptr, (i * 8) as i32);
			fields.push(FieldDef {
				name: name.clone().expect("guarded by the caller"),
				typ,
				default: None,
				embedded: false,
				annotations: vec![],
			});
		}
		let shape: Vec<_> = fields.iter().map(|f| format!("{}: {}", f.name, f.typ)).collect();
		Ok((ptr, Typ::Struct(format!("struct{{{}}}", shape.join(", ")), fields)))
	}

	// Allocate a struct on the stack, initializing each field to its default.
	pub(super) fn struct_slot(&mut self, struct_fields: &[FieldDef]) -> Result<Value, Diagnostic> {
		let ptr = self.stack_slot((struct_fields.len() * 8) as u32);
		for (i, f) in struct_fields.iter().enumerate() {
			let init = if let Some(default_expr) = &f.default {
				let val = self.check_typed(default_expr, &f.typ, "not a valid default for this field")?;
				self.copy_in(val, &f.typ)
			} else if let Typ::Struct(_, inner) = &f.typ {
				// apply field defaults of inner structs
				let inner = inner.clone();
				self.struct_slot(&inner)?
			} else {
				self.zero(&f.typ)
			};
			self.b.ins().store(MemFlags::new(), init, ptr, (i * 8) as i32);
		}
		Ok(ptr)
	}

	// Struct literal for a generic struct.
	fn generic_struct_lit(
		&mut self,
		name: &str,
		def: GenericStructDef,
		fields: &[(Option<String>, Spanned<Expr>)],
		span: Span,
		target: Option<&Typ>,
	) -> Result<TypedVal, Diagnostic> {
		let positional = fields.first().is_some_and(|(n, _)| n.is_none());
		if positional && fields.len() != def.fields.len() {
			return Err(Diagnostic::new(
				format!(
					"`{name}` has {} fields but {} values were provided",
					def.fields.len(),
					fields.len()
				),
				span.into_range(),
			)
			.with_label("wrong number of fields"));
		}
		let mut subst = HashMap::new();
		let mut provided = Vec::with_capacity(fields.len());
		for (i, (field_name, value)) in fields.iter().enumerate() {
			if matches!(value.0, Expr::Spread(_)) {
				return Err(Diagnostic::new(
					"spread in a generic struct literal isn't supported yet",
					value.1.into_range(),
				)
				.with_label("unsupported"));
			}
			let idx = match field_name {
				None if positional => {
					self.check_member(name, &def.fields[i].name, value.1)?;
					i
				}
				None => {
					return Err(
						Diagnostic::new("cannot mix named and positional fields", value.1.into_range())
							.with_label("missing field name"),
					);
				}
				Some(fname) => {
					self.check_member(name, fname, value.1)?;
					def.fields.iter().position(|f| &f.name == fname).ok_or_else(|| {
						Diagnostic::new(format!("`{name}` has no field `{fname}`"), value.1.into_range())
							.with_label("no such field")
					})?
				}
			};
			let fte = &def.fields[idx].typ;
			let (val, vtyp) = match def.type_params.iter().any(|p| mentions(fte, &p.name)) {
				true => self.expr(value)?,
				false => {
					let want = self.types().resolve(fte, value.1)?;
					self.check_expr(value, &want)?
				}
			};
			unify(&def.fields[idx].typ, &vtyp, &def.type_params, &mut subst, self.generics)
				.map_err(|msg| Diagnostic::new(msg, value.1.into_range()).with_label("type mismatch"))?;
			let val = self.copy_in(val, &vtyp);
			provided.push((idx, val, vtyp, value.1));
		}
		// params the field values didn't pin can come from the expected type
		if let Some(Typ::Struct(_, tfields)) = target {
			for (df, tf) in def.fields.iter().zip(tfields) {
				unify(&df.typ, &tf.typ, &def.type_params, &mut subst, self.generics).ok();
			}
		}
		if let Some(missing) = def.type_params.iter().find(|p| !subst.contains_key(&p.name)) {
			return Err(Diagnostic::new(
				format!("cannot infer type parameter `{}`", missing.name),
				span.into_range(),
			)
			.with_label("not determined by any field"));
		}
		let typ = self.types().instantiate(name, &def, &subst, span)?;
		let Typ::Struct(_, struct_fields) = &typ else {
			unreachable!()
		};
		check_required(name, struct_fields, fields, span)?;
		let ptr = self.struct_slot(struct_fields)?;
		for (idx, val, vtyp, vspan) in provided {
			let expected = &struct_fields[idx].typ;
			if &vtyp != expected {
				return Err(
					Diagnostic::new(format!("expected {expected}, got {vtyp}"), vspan.into_range())
						.with_label("type mismatch"),
				);
			}
			closure_escape(&vtyp, vspan.into_range(), "stored in a field")?;
			self.b.ins().store(MemFlags::new(), val, ptr, (idx * 8) as i32);
		}
		Ok((ptr, typ))
	}

	// A stack slot's base address.
	pub(super) fn stack_slot(&mut self, size: u32) -> Value {
		let slot = self
			.b
			.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size, 0));
		self.b.ins().stack_addr(self.int, slot, 0)
	}

	// Copy field slots between structs.
	pub(super) fn assign_fields(&mut self, src: Value, dst: Value, fields: &[FieldDef], release_old: bool) {
		for (i, f) in fields.iter().enumerate() {
			let cl = cl_type(&f.typ, self.int);
			let old = (release_old && rc::releasable(&f.typ))
				.then(|| self.b.ins().load(cl, MemFlags::new(), dst, (i * 8) as i32));
			let fv = self.b.ins().load(cl, MemFlags::new(), src, (i * 8) as i32);
			let fv = self.copy_in(fv, &f.typ);
			self.b.ins().store(MemFlags::new(), fv, dst, (i * 8) as i32);
			if let Some(old) = old {
				self.release_value(old, &f.typ);
			}
		}
	}

	pub(super) fn fixed_copy(&mut self, src: Value, elem: &Typ, n: usize) -> Value {
		let stride = self.elem_stride(elem);
		let dst = self.stack_slot((n as i64 * stride) as u32);
		self.fixed_move(dst, src, elem, n);
		dst
	}

	// Copy n elements between two packed buffers.
	pub(super) fn fixed_move(&mut self, dst: Value, src: Value, elem: &Typ, n: usize) {
		let stride = self.elem_stride(elem);
		for i in 0..n {
			let off = (i as i64 * stride) as i32;
			let v = self.load_elem(src, off, elem);
			self.store_elem(dst, off, elem, v);
		}
	}
}
