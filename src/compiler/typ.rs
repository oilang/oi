//! The Oi type layer.

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use cranelift::prelude::*;

use crate::ast::{Access, Annotation, Expr, Param, Spanned, TypeExpr};
use crate::compiler::role;

#[derive(Clone, Debug)]
pub(crate) enum Typ {
	Int(u16),
	UInt(u16),
	ISize,
	USize,
	Float(u16),
	Bool,
	Str,
	CStr,
	Atom,
	Tuple(Vec<(Option<String>, Typ)>),
	Array(Box<Typ>),
	FixedArray(Box<Typ>, usize),
	Struct(String, Vec<FieldDef>),
	TupleStruct(String, Vec<(Option<String>, Typ)>),
	Enum(String),
	Trait(String),
	Option(Box<Typ>),
	Result(Box<Typ>, Box<Typ>),
	Sum(String, Vec<VariantInfo>),
	Error,
	Fn(Vec<FnParam>, Box<Typ>),
	Annotated(Vec<String>, Box<Typ>),
	Closure(Vec<FnParam>, Box<Typ>, bool),
	Map(Box<Typ>, Box<Typ>),
	Access(Access, Box<Typ>),
	Ref(Box<Typ>),
	Ast,
	Any,
}

// Annotation names.
pub(crate) fn marks(anns: &[String]) -> String {
	anns.iter().map(|a| format!("@{} ", display_name(a))).collect()
}

// One fn parameter slot.
#[derive(Clone, Debug)]
pub(crate) struct FnParam {
	pub name: Option<String>,
	pub typ: Typ,
	pub default: Option<Spanned<Expr>>,
	pub variadic: bool,
}

impl FnParam {
	pub fn new(typ: Typ) -> Self {
		FnParam {
			name: None,
			typ,
			default: None,
			variadic: false,
		}
	}

	pub fn of(p: &Param, typ: Typ) -> Self {
		FnParam {
			name: Some(p.name.clone()),
			typ,
			default: p.default.clone(),
			variadic: matches!(p.typ, TypeExpr::Variadic(_)),
		}
	}
}

impl PartialEq for FnParam {
	fn eq(&self, other: &Self) -> bool {
		self.typ == other.typ
	}
}

// A struct field definition.
#[derive(Clone, Debug)]
pub(crate) struct FieldDef {
	pub name: String,
	pub typ: Typ,
	pub default: Option<Spanned<Expr>>,
	pub embedded: bool,
	pub annotations: Vec<Annotation>,
}

// Check whether a field is embedded.
pub(crate) fn embedded(p: &Param) -> bool {
	matches!(&p.typ, TypeExpr::Name(n) if *n == p.name)
}

// The embedded fields of a struct.
pub(crate) fn embeds(fields: &[FieldDef]) -> impl Iterator<Item = (usize, &str, &[FieldDef])> {
	fields.iter().enumerate().filter_map(|(o, f)| match &f.typ {
		Typ::Struct(sn, inner) if f.embedded => Some((o, sn.as_str(), &inner[..])),
		_ => None,
	})
}

// A field's encoded vtable slot.
pub(crate) fn field_slot<'f>(fields: &'f [FieldDef], name: &str) -> Option<(i64, &'f FieldDef)> {
	let direct = fields.iter().position(|f| f.name == name).map(|i| ((i * 8) as i64, &fields[i]));
	direct.or_else(|| {
		embeds(fields).find_map(|(o, _, inner)| {
			let i = inner.iter().position(|f| f.name == name)?;
			Some((((i * 8) as i64) << 32 | ((o * 8) as i64) | 1, &inner[i]))
		})
	})
}

// A struct's field offsets under the C ABI.
pub(crate) struct CLayout {
	pub offsets: Vec<u32>,
	pub size: u32,
	pub align: u32,
}

// C offset/alignment layout.
pub(crate) fn c_layout(fields: &[FieldDef], is_c: &impl Fn(&str) -> bool) -> Option<CLayout> {
	let (mut offsets, mut size, mut align) = (Vec::with_capacity(fields.len()), 0u32, 1u32);
	for f in fields {
		let (fsize, falign) = f.typ.c_size_align(is_c)?;
		size = size.next_multiple_of(falign);
		offsets.push(size);
		size += fsize;
		align = align.max(falign);
	}
	Some(CLayout {
		offsets,
		size: size.next_multiple_of(align),
		align,
	})
}

impl Typ {
	pub fn unit() -> Typ {
		Typ::Tuple(vec![])
	}

	pub fn is_unit(&self) -> bool {
		matches!(self, Typ::Tuple(f) if f.is_empty())
	}

	pub fn is_enumish(&self) -> bool {
		matches!(
			self,
			Typ::Enum(_) | Typ::Option(_) | Typ::Result(..) | Typ::Sum(..) | Typ::Any
		)
	}

	// Single-field tuple structs are transparent.
	pub fn newtype(&self) -> Option<&Typ> {
		match self {
			Typ::TupleStruct(_, fields) if fields.len() == 1 => Some(&fields[0].1),
			_ => None,
		}
	}

	// Whether a value can cross the C ABI.
	pub fn is_c_repr(&self) -> bool {
		match self.newtype().unwrap_or(self) {
			Typ::Fn(ps, r) => (ps.iter().map(|p| &p.typ))
				.chain((!r.is_unit()).then_some(&**r))
				.all(Typ::is_c_repr),
			Typ::Annotated(_, t) => t.is_c_repr(),
			t => matches!(
				t,
				Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize | Typ::Float(_) | Typ::Bool | Typ::CStr
			),
		}
	}

	// Size and alignment under the C ABI.
	pub fn c_size_align(&self, is_c: &impl Fn(&str) -> bool) -> Option<(u32, u32)> {
		let scalar = |bytes: u32| Some((bytes, bytes));
		match self.newtype().unwrap_or(self) {
			Typ::Int(w) | Typ::UInt(w) => scalar(cl_int_for_width(*w).bytes()),
			Typ::Float(w) => scalar((*w as u32) / 8),
			Typ::Bool => scalar(1),
			Typ::ISize | Typ::USize | Typ::CStr => scalar(8),
			t @ Typ::Fn(..) if t.is_c_repr() => scalar(8),
			Typ::Annotated(_, t) => t.c_size_align(is_c),
			Typ::FixedArray(e, n) => e
				.c_size_align(is_c)
				.filter(|(es, _)| e.is_c_repr() && *es as i64 == elem_size(e))
				.map(|(es, ea)| (es * *n as u32, ea)),
			Typ::Struct(name, fields) if is_c(name) => c_layout(fields, is_c).map(|l| (l.size, l.align)),
			_ => None,
		}
	}

	// 1:1 spelling for identity keys.
	pub fn key(&self) -> String {
		let keys = |ts: &[Typ]| ts.iter().map(Typ::key).collect::<Vec<_>>().join(", ");
		match self {
			Typ::Tuple(fields) => {
				let elems: Vec<_> = fields.iter().map(|(_, t)| t.key()).collect();
				format!("({})", elems.join(", "))
			}
			Typ::Array(e) => format!("[]{}", e.key()),
			Typ::FixedArray(e, n) => format!("[{n}]{}", e.key()),
			Typ::Option(inner) => format!("?{}", inner.key()),
			Typ::Result(ok, err) if **err == Typ::Error => format!("!{}", ok.key()),
			Typ::Result(ok, err) => format!("Result[{}, {}]", ok.key(), err.key()),
			Typ::Map(k, v) => format!("[{}]{}", k.key(), v.key()),
			Typ::Access(a, inner) => format!("{a} {}", inner.key()),
			Typ::Ref(inner) => format!("&{}", inner.key()),
			Typ::Trait(name) => format!("dyn {name}"),
			Typ::Fn(params, ret) | Typ::Closure(params, ret, _) => {
				let ps: Vec<_> = params.iter().map(|p| p.typ.key()).collect();
				format!("fn({}) {}", ps.join(", "), ret.key())
			}
			Typ::Annotated(anns, t) => format!("{}{}", marks(anns), t.key()),
			Typ::Sum(name, _) if !name.is_empty() => name.clone(),
			Typ::Sum(_, variants) => variants
				.iter()
				.map(|v| match v.payload.is_empty() {
					true => format!(":{}", v.name),
					false => format!("{}({})", v.name, keys(&v.payload)),
				})
				.collect::<Vec<_>>()
				.join(" | "),
			_ => self.to_string(),
		}
	}
}

impl fmt::Display for Typ {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Typ::Int(64) => write!(f, "int"),
			Typ::Int(w) => write!(f, "i{w}"),
			Typ::UInt(64) => write!(f, "uint"),
			Typ::UInt(w) => write!(f, "u{w}"),
			Typ::ISize => write!(f, "isize"),
			Typ::USize => write!(f, "usize"),
			Typ::Float(64) => write!(f, "float"),
			Typ::Float(w) => write!(f, "f{w}"),
			Typ::Bool => write!(f, "bool"),
			Typ::Str => write!(f, "string"),
			Typ::CStr => write!(f, "cstr"),
			Typ::Atom => write!(f, "atom"),
			Typ::Tuple(fields) if fields.is_empty() => write!(f, "()"),
			Typ::Tuple(_) => write!(f, "tuple"),
			Typ::Array(e) => write!(f, "[]{e}"),
			Typ::FixedArray(e, n) => write!(f, "[{n}]{e}"),
			Typ::Struct(name, _) => write!(f, "{name}"),
			Typ::TupleStruct(name, _) => write!(f, "{name}"),
			Typ::Enum(name) => write!(f, "{name}"),
			Typ::Trait(name) => write!(f, "{name}"),
			Typ::Option(inner) => write!(f, "?{inner}"),
			Typ::Result(ok, err) if **err == Typ::Error => write!(f, "!{ok}"),
			Typ::Result(ok, err) => write!(f, "Result[{ok}, {err}]"),
			Typ::Sum(name, _) if !name.is_empty() => write!(f, "{name}"),
			Typ::Sum(_, variants) => {
				write!(
					f,
					"{}",
					variants
						.iter()
						.map(|v| if v.payload.is_empty() {
							format!(":{}", v.name)
						} else {
							v.name.clone()
						})
						.collect::<Vec<_>>()
						.join(" | ")
				)
			}
			Typ::Error => write!(f, "Error"),
			Typ::Annotated(anns, t) => write!(f, "{}{t}", marks(anns)),
			Typ::Fn(params, ret) | Typ::Closure(params, ret, _) => {
				write!(f, "fn(")?;
				for (i, p) in params.iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					match (p.variadic, &p.typ) {
						(true, Typ::Array(e)) => write!(f, "..{e}")?,
						_ => write!(f, "{}", p.typ)?,
					}
				}
				write!(f, ") {ret}")
			}
			Typ::Map(k, v) => write!(f, "[{k}]{v}"),
			Typ::Access(a, inner) => write!(f, "{a} {inner}"),
			Typ::Ref(inner) => write!(f, "&{inner}"),
			Typ::Ast => write!(f, "Ast"),
			Typ::Any => write!(f, "any"),
		}
	}
}

impl PartialEq for FieldDef {
	fn eq(&self, other: &Self) -> bool {
		self.name == other.name && self.typ == other.typ
	}
}

// Ignore tuple field names for equality.
impl PartialEq for Typ {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Typ::Tuple(a), Typ::Tuple(b)) => a.len() == b.len() && a.iter().zip(b).all(|((_, x), (_, y))| x == y),
			(Typ::TupleStruct(a, _), Typ::TupleStruct(b, _)) => a == b,
			(Typ::Int(a), Typ::Int(b)) | (Typ::UInt(a), Typ::UInt(b)) | (Typ::Float(a), Typ::Float(b)) => a == b,
			(Typ::Struct(n, a), Typ::Struct(m, b)) => n == m && a == b,
			(Typ::Enum(a), Typ::Enum(b)) => a == b,
			(Typ::Trait(a), Typ::Trait(b)) => a == b,
			(Typ::Option(a), Typ::Option(b)) | (Typ::Array(a), Typ::Array(b)) => a == b,
			(Typ::Result(a, e), Typ::Result(b, f)) => a == b && e == f,
			(Typ::FixedArray(a, n), Typ::FixedArray(b, m)) => a == b && n == m,
			(Typ::Sum(n, a), Typ::Sum(m, b)) => n == m && a == b,
			(Typ::Fn(p, r) | Typ::Closure(p, r, _), Typ::Fn(q, s) | Typ::Closure(q, s, _)) => p == q && r == s,
			(Typ::Annotated(a, x), Typ::Annotated(b, y)) => a == b && x == y,
			(Typ::Map(k, v), Typ::Map(l, w)) => k == l && v == w,
			(Typ::Access(a, x), Typ::Access(b, y)) => a == b && x == y,
			(Typ::Ref(a), Typ::Ref(b)) => match (&**a, &**b) {
				(Typ::Struct(n, _), Typ::Struct(m, _)) => n == m,
				_ => a == b,
			},
			_ => std::mem::discriminant(self) == std::mem::discriminant(other),
		}
	}
}

// Spell a type back out as source, for signatures created by the compiler.
pub(crate) fn type_expr(typ: &Typ) -> Option<TypeExpr> {
	let named = |n: &String| (!n.contains('[')).then(|| TypeExpr::Name(n.clone()));
	Some(match typ {
		Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize | Typ::Float(_) | Typ::Bool | Typ::Str | Typ::CStr => {
			TypeExpr::Name(typ.to_string())
		}
		Typ::Struct(n, _) | Typ::TupleStruct(n, _) | Typ::Enum(n) => named(n)?,
		Typ::Array(e) => TypeExpr::Array(Box::new(type_expr(e)?)),
		Typ::FixedArray(e, n) => {
			TypeExpr::FixedArray(Box::new(type_expr(e)?), Box::new((Expr::Int(*n as i64), (0..0).into())))
		}
		Typ::Option(e) => TypeExpr::Option(Box::new(type_expr(e)?)),
		Typ::Result(ok, err) => {
			let err = if **err == Typ::Error {
				None
			} else {
				Some(Box::new(type_expr(err)?))
			};
			TypeExpr::Result(Box::new(type_expr(ok)?), err)
		}
		Typ::Map(k, v) => TypeExpr::Map(Box::new(type_expr(k)?), Box::new(type_expr(v)?)),
		Typ::Tuple(fs) => TypeExpr::Tuple(
			fs.iter()
				.map(|(n, t)| Some((n.clone(), type_expr(t)?)))
				.collect::<Option<_>>()?,
		),
		Typ::Fn(ps, r) => TypeExpr::Fn(
			ps.iter()
				.map(|p| Some((p.name.clone(), access_of(&p.typ), type_expr(access_peel(&p.typ))?)))
				.collect::<Option<_>>()?,
			Box::new(type_expr(r)?),
		),
		_ => return None,
	})
}

// The access mod a param carries.
pub(crate) fn access_of(typ: &Typ) -> Access {
	if let Typ::Access(a, _) = typ { *a } else { Access::Read }
}

// The type under an access mod.
pub(crate) fn access_peel(typ: &Typ) -> &Typ {
	if let Typ::Access(_, inner) = typ { inner } else { typ }
}

// Wrap an Access into a Typ.
pub(crate) fn access_wrap(access: Access, typ: Typ) -> Typ {
	if access == Access::Read {
		typ
	} else {
		Typ::Access(access, Box::new(typ))
	}
}

pub(crate) fn oi_symbol(name: &str) -> String {
	format!("oi_{}", name.replace('.', "__").replace("::", "$"))
}

// Strip qualifiers like `module::`.
pub(crate) fn display_name(name: &str) -> &str {
	name.split_once("::").map_or(name, |(_, rest)| rest)
}

pub(crate) fn cl_int_for_width(w: u16) -> types::Type {
	match w {
		1..=8 => types::I8,
		9..=16 => types::I16,
		17..=32 => types::I32,
		_ => types::I64,
	}
}

pub(crate) fn cl_type(typ: &Typ, int: types::Type) -> types::Type {
	match typ.newtype().unwrap_or(typ) {
		Typ::Int(w) | Typ::UInt(w) => cl_int_for_width(*w),
		Typ::ISize | Typ::USize => int,
		Typ::Float(w) => match w {
			16 => types::F16,
			32 => types::F32,
			64 => types::F64,
			128 => types::F128,
			w => panic!("unsupported float width f{w}"),
		},
		_ => int,
	}
}

pub(crate) fn elem_size(typ: &Typ) -> i64 {
	match typ.newtype().unwrap_or(typ) {
		Typ::Int(w) | Typ::UInt(w) => cl_int_for_width(*w).bytes() as i64,
		Typ::Float(w) => (*w as i64) / 8,
		_ => 8,
	}
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) struct VariantInfo {
	pub name: String,
	pub disc: i64,
	pub raw: Option<String>,
	pub payload: Vec<Typ>,
	pub names: Vec<String>,
	pub backing: Option<Typ>,
}

impl VariantInfo {
	pub fn new(name: impl Into<String>, disc: i64, payload: Vec<Typ>) -> Self {
		VariantInfo {
			name: name.into(),
			disc,
			raw: None,
			payload,
			names: vec![],
			backing: None,
		}
	}
}

pub(crate) fn is_range(t: &Typ) -> bool {
	matches!(t, Typ::Struct(n, _) if n == role::RANGE)
}

// An enum is a tagged union if any variant has fields.
pub(crate) fn enum_boxed(variants: &[VariantInfo]) -> bool {
	variants.iter().any(|v| !v.payload.is_empty())
}

// Tag permutation to coerce a sum across definitions with the same member set.
pub(crate) fn sum_remap(src: &[VariantInfo], dst: &[VariantInfo]) -> Option<Vec<(i64, i64)>> {
	if src.len() != dst.len() {
		return None;
	}
	src.iter()
		.map(|s| {
			dst.iter()
				.find(|d| d.name == s.name && d.payload == s.payload)
				.map(|d| (s.disc, d.disc))
		})
		.collect()
}

// Slot count of a boxed enum.
pub(crate) fn enum_slots(variants: &[VariantInfo]) -> usize {
	// the tag plus the widest variant's fields
	1 + variants.iter().map(|v| v.payload.len()).max().unwrap_or(0)
}

pub(crate) fn option_variants(inner: &Typ) -> Vec<VariantInfo> {
	vec![
		VariantInfo::new("none", 0, vec![]),
		VariantInfo::new("some", 1, vec![inner.clone()]),
	]
}

pub(crate) fn result_variants(ok: &Typ, err: &Typ) -> Vec<VariantInfo> {
	vec![
		VariantInfo::new("ok", 0, vec![ok.clone()]),
		VariantInfo::new("err", 1, vec![err.clone()]),
	]
}

// The tag an `any` box carries for a type.
pub(crate) fn typeid(t: &Typ) -> i64 {
	let mut h = DefaultHasher::new();
	t.key().hash(&mut h);
	h.finish() as i64
}

// An atom sum type desugars to a bare enum.
pub(crate) fn atom_sum_variants(names: &[String]) -> Vec<VariantInfo> {
	names
		.iter()
		.enumerate()
		.map(|(disc, name)| VariantInfo::new(name.clone(), disc as i64, vec![]))
		.collect()
}
