//! The Oi type layer.

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use cranelift::codegen::ir::ArgumentPurpose;
use cranelift::prelude::*;
use target_lexicon::Architecture;

use crate::ast::{Access, Annotation, Expr, Param, Span, Spanned, TypeExpr};
use crate::compiler::resolve::TypeCtx;
use crate::compiler::role;

#[derive(Clone, Debug)]
pub(crate) enum Typ {
	Int(u16),
	UInt(u16),
	ISize,
	USize,
	Float(u16),
	Bool,
	Rune,
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
	TypeId,
	Const(Box<Expr>),
}

// Spell a comptime value back out, for instance names.
fn lit_text(e: &Expr) -> String {
	let list = |es: &mut dyn Iterator<Item = &Spanned<Expr>>| es.map(|e| lit_text(&e.0)).collect::<Vec<_>>().join(", ");
	match e {
		Expr::Int(n) => n.to_string(),
		Expr::Float(x) => x.to_string(),
		Expr::Bool(b) => b.to_string(),
		Expr::String(s) => format!("{s:?}"),
		Expr::Atom(a) => format!(":{a}"),
		Expr::Array(es) => format!("[{}]", list(&mut es.iter())),
		Expr::StructLit { name, fields, .. } => format!("{name}.{{{}}}", list(&mut fields.iter().map(|(_, v)| v))),
		Expr::Cast { args, .. } => list(&mut args.iter()),
		Expr::EnumShorthand { variant, args } if args.is_empty() => format!(":{variant}"),
		Expr::EnumShorthand { variant, args } => format!(".{variant}.({})", list(&mut args.iter())),
		Expr::DotTuple(es) => format!(".({})", list(&mut es.iter())),
		e => format!("{e:?}"),
	}
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
	pub escapes: bool,
	pub copy: bool,
}

impl FnParam {
	pub fn new(typ: Typ) -> Self {
		FnParam {
			name: None,
			typ,
			default: None,
			variadic: false,
			escapes: false,
			copy: false,
		}
	}

	pub fn of(p: &Param, typ: Typ) -> Self {
		FnParam {
			name: Some(p.name.clone()),
			typ,
			default: p.default.clone(),
			variadic: matches!(p.typ, TypeExpr::Variadic(_)),
			escapes: false,
			copy: p.mutable,
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

// Whether a field is embedded.
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

// The name a field routes claims through, embedded or stated by a `via`.
pub(crate) fn lends(f: &FieldDef) -> String {
	match &f.typ {
		Typ::Error => role::ERROR.into(),
		t => t.to_string(),
	}
}

// A struct field's encoded vtable slot and type.
pub(crate) fn field_slot(types: &TypeCtx, sname: &str, name: &str) -> Option<(i64, Typ)> {
	let fields = types.structs.get(sname)?;
	let outer = Typ::Struct(sname.into(), fields.clone());
	let slot = |typ: &Typ, i: usize| types.layout(typ).map(|l| (l.fields[i].off as i64, l.fields[i].typ.clone()));
	match fields.iter().position(|f| f.name == name) {
		Some(i) => slot(&outer, i),
		None => embeds(fields).find_map(|(o, _, inner)| {
			let (off, typ) = slot(&fields[o].typ, inner.iter().position(|f| f.name == name)?)?;
			Some((off << 32 | slot(&outer, o)?.0 | 1, typ))
		}),
	}
}

// A struct or tuple's fields at their byte offsets, and its size.
#[derive(Default)]
pub(crate) struct Layout {
	pub fields: Vec<Field>,
	pub size: u32,
}

pub(crate) struct Field {
	pub off: i32,
	pub words: i32,
	pub typ: Typ,
}

// A struct's field offsets under the C ABI.
pub(crate) struct CLayout {
	pub offsets: Vec<u32>,
	pub size: u32,
	pub align: u32,
}

// C offset/alignment layout.
pub(crate) fn c_layout(fields: &[FieldDef], types: &TypeCtx) -> Option<CLayout> {
	let (mut offsets, mut size, mut align) = (Vec::with_capacity(fields.len()), 0u32, 1u32);
	for f in fields {
		let (fsize, falign) = f.typ.c_size_align(types)?;
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

// Each scalar of a C layout at its offset, as its register type.
fn c_scalars(typ: &Typ, at: i32, types: &TypeCtx) -> Vec<(i32, types::Type)> {
	match typ.newtype().unwrap_or(typ) {
		Typ::Struct(_, fields) => (fields.iter().zip(c_layout(fields, types).expect("validated").offsets))
			.flat_map(|(f, off)| c_scalars(&f.typ, at + off as i32, types))
			.collect(),
		Typ::FixedArray(e, n) => {
			let size = e.c_size_align(types).expect("validated").0 as i32;
			(0..*n as i32).flat_map(|i| c_scalars(e, at + i * size, types)).collect()
		}
		Typ::Float(32) => vec![(at, types::F32)],
		Typ::Float(_) => vec![(at, types::F64)],
		_ => vec![(at, types::I64)],
	}
}

impl Typ {
	pub fn unit() -> Typ {
		Typ::Tuple(vec![])
	}

	// The embed offsets from this type down to `want`, if it is or embeds one.
	pub(crate) fn ctx_path(&self, want: &str) -> Option<Vec<usize>> {
		let Typ::Struct(n, f) = self else { return None };
		if n == want {
			return Some(vec![]);
		}
		embeds(f).find_map(|(o, ..)| Some([vec![o], f[o].typ.ctx_path(want)?].concat()))
	}

	pub fn is_unit(&self) -> bool {
		matches!(self, Typ::Tuple(f) if f.is_empty())
	}

	// The word slots of a struct or tuple held inline.
	pub fn slots(&self) -> Option<Vec<Typ>> {
		match self {
			Typ::Struct(_, fs) => Some(fs.iter().map(|f| f.typ.clone()).collect()),
			Typ::Tuple(fs) | Typ::TupleStruct(_, fs) if self.newtype().is_none() && !fs.is_empty() => {
				Some(fs.iter().map(|(_, t)| t.clone()).collect())
			}
			_ => None,
		}
	}

	pub fn bool_bytes(&self) -> Option<u16> {
		match self {
			Typ::Bool => Some(1),
			Typ::Annotated(a, t) if matches!(**t, Typ::Bool) => a[0][1..].parse().ok().map(|w: u16| w / 8),
			_ => None,
		}
	}

	// Bounds of an int type, as i128 bits.
	pub fn int_bounds(&self) -> Option<(i128, i128)> {
		match self {
			Typ::Int(width) => Some((int_min(*width), int_max(*width))),
			Typ::UInt(width) => Some((0, uint_max(*width))),
			Typ::ISize => Some((int_min(64), int_max(64))),
			Typ::USize => Some((0, uint_max(64))),
			_ => None,
		}
	}

	pub fn is_enumish(&self) -> bool {
		matches!(self, Typ::Enum(_) | Typ::Sum(..) | Typ::Any)
	}

	// Single-field tuple structs are transparent.
	pub fn newtype(&self) -> Option<&Typ> {
		match self {
			Typ::TupleStruct(_, fields) if fields.len() == 1 => Some(&fields[0].1),
			Typ::Annotated(_, t) if self.keyed().is_some() => Some(t),
			_ => None,
		}
	}

	// An enum-indexed array's key enum and fixed array.
	pub fn keyed(&self) -> Option<(Typ, &Typ)> {
		let Typ::Annotated(anns, t) = self else { return None };
		let name = anns[0].strip_prefix('[')?.strip_suffix(']')?;
		Some((Typ::Enum(name.into()), t))
	}

	pub fn unkeyed(&self) -> &Typ {
		self.keyed().map_or(self, |(_, t)| t)
	}

	// A payload variant as a refinement of its enum.
	pub fn refined(&self, variant: &str) -> Typ {
		Typ::Annotated(vec![format!(".{variant}")], Box::new(self.clone()))
	}

	// A refined type's variant and enum.
	pub fn variant(&self) -> Option<(&str, &Typ)> {
		let Typ::Annotated(anns, t) = self else { return None };
		Some((anns[0].strip_prefix('.')?, t))
	}

	// The declared name of a struct or enum.
	pub fn nominal(&self) -> Option<&str> {
		match self {
			Typ::Struct(n, _) | Typ::TupleStruct(n, _) | Typ::Enum(n) => Some(n),
			_ => None,
		}
	}

	// Whether a value can cross the C ABI.
	pub fn is_c_repr(&self, types: &TypeCtx) -> bool {
		match self.newtype().unwrap_or(self) {
			Typ::Fn(ps, r) => ps.iter().all(|p| p.typ.is_c_param(types)) && (r.is_unit() || r.is_c_param(types)),
			Typ::Annotated(_, t) => t.is_c_repr(types),
			Typ::Enum(name) => types.is_c(name),
			t => matches!(
				t,
				Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize | Typ::Float(_) | Typ::Bool | Typ::CStr
			),
		}
	}

	// Whether a value can be a C fn's param, `@c` structs by value.
	pub fn is_c_param(&self, types: &TypeCtx) -> bool {
		self.is_c_repr(types) || matches!(self, Typ::Struct(n, _) if types.is_c(n))
	}

	// A `@c` struct's by-value ABI params, at their offsets or `None` for a pointer to its copy.
	pub fn c_by_val(&self, types: &TypeCtx, arch: Architecture) -> Option<CRegs> {
		let size = self.c_size_align(types).filter(|_| matches!(self, Typ::Struct(..)))?.0;
		let (x86, scalars) = (arch == Architecture::X86_64, c_scalars(self, 0, types));
		let sse = |at: i32| x86 && scalars.iter().filter(|s| s.0 / 8 == at / 8).all(|s| s.1.is_float());
		let word = |at| (at, if sse(at) { types::F64 } else { types::I64 });
		let hfa = scalars.len() <= 4 && scalars.iter().all(|s| s.1.is_float() && s.1 == scalars[0].1);
		let regs: Vec<_> = if matches!(arch, Architecture::Aarch64(_)) && hfa {
			scalars
		} else if size <= 16 {
			(0..size as i32).step_by(8).map(word).collect()
		} else {
			let sarg = AbiParam::special(types::I64, ArgumentPurpose::StructArgument(size.next_multiple_of(8)));
			return Some(vec![(None, if x86 { sarg } else { AbiParam::new(types::I64) })]);
		};
		Some(regs.into_iter().map(|(at, t)| (Some(at), AbiParam::new(t))).collect())
	}

	// Properly size a C enum.
	pub fn c_enum(&self, types: &TypeCtx) -> Option<Typ> {
		let Typ::Enum(name) = self.newtype().unwrap_or(self) else {
			return None;
		};
		let b = types.enums.borrow().get(name)?.first()?.backing.clone();
		b.filter(|b| matches!(b, Typ::Int(w) | Typ::UInt(w) if *w < 64))
	}

	// Size and alignment under the C ABI.
	pub fn c_size_align(&self, types: &TypeCtx) -> Option<(u32, u32)> {
		let scalar = |bytes: u32| Some((bytes, bytes));
		match self.newtype().unwrap_or(self) {
			Typ::Int(w) | Typ::UInt(w) => scalar(cl_int_for_width(*w).bytes()),
			Typ::Float(w) => scalar((*w as u32) / 8),
			t if let Some(n) = t.bool_bytes() => scalar(n.into()),
			Typ::ISize | Typ::USize | Typ::CStr => scalar(8),
			t @ Typ::Fn(..) if t.is_c_repr(types) => scalar(8),
			Typ::Enum(name) if types.is_c(name) => self.c_enum(types).map_or(scalar(8), |b| b.c_size_align(types)),
			Typ::Annotated(_, t) => t.c_size_align(types),
			Typ::FixedArray(e, n) => e
				.c_size_align(types)
				.filter(|(es, _)| e.is_c_repr(types) && *es as i64 == elem_size(e))
				.map(|(es, ea)| (es * *n as u32, ea)),
			Typ::Struct(name, fields) if types.is_c(name) => c_layout(fields, types).map(|l| (l.size, l.align)),
			_ => None,
		}
	}

	// 1:1 spelling for identity keys.
	pub fn key(&self) -> String {
		let keys = |ts: &mut dyn Iterator<Item = &Typ>| ts.map(Typ::key).collect::<Vec<_>>().join(", ");
		match self {
			Typ::Tuple(fields) => format!("({})", keys(&mut fields.iter().map(|(_, t)| t))),
			Typ::Array(e) => format!("[]{}", e.key()),
			Typ::FixedArray(e, n) => format!("[{n}]{}", e.key()),
			Typ::Map(k, v) => format!("[{}]{}", k.key(), v.key()),
			Typ::Access(a, inner) => format!("{a} {}", inner.key()),
			Typ::Ref(inner) => format!("^{}", inner.key()),
			Typ::Trait(name) => format!("dyn {name}"),
			Typ::Fn(params, ret) | Typ::Closure(params, ret, _) => {
				format!("fn({}) {}", keys(&mut params.iter().map(|p| &p.typ)), ret.key())
			}
			Typ::Annotated(_, t) if let (Some((k, _)), Typ::FixedArray(e, _)) = (self.keyed(), &**t) => {
				format!("[{}]{}", k.key(), e.key())
			}
			Typ::Annotated(anns, t) => format!("{}{}", marks(anns), t.key()),
			Typ::Struct(name, _) => name.clone(),
			Typ::Sum(name, _) if !name.is_empty() => name.clone(),
			Typ::Sum(_, variants) => variants
				.iter()
				.map(|v| match v.payload.is_empty() {
					true => format!(":{}", v.name),
					false => format!("{}({})", v.name, keys(&mut v.payload.iter())),
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
			Typ::Rune => write!(f, "rune"),
			Typ::Str => write!(f, "string"),
			Typ::CStr => write!(f, "cstr"),
			Typ::Atom => write!(f, "atom"),
			Typ::Tuple(fields) if fields.is_empty() => write!(f, "()"),
			Typ::Tuple(fields)
				if let [lead @ .., (Some(n), slot)] = &fields[..]
					&& [role::PACK_OK, role::PACK_ERR].contains(&&**n) =>
			{
				let ok = lead_typ(lead);
				match slot.to_string().strip_prefix('?') {
					Some(err) if err != "Error" => write!(f, "Result[{ok}, {err}]"),
					_ => write!(f, "{}{ok}", if n == role::PACK_OK { "?" } else { "!" }),
				}
			}
			Typ::Tuple(fields) => {
				let show = |(n, t): &(Option<String>, Typ)| n.as_ref().map_or(t.to_string(), |n| format!("{n}: {t}"));
				write!(f, "({})", fields.iter().map(show).collect::<Vec<_>>().join(", "))
			}
			Typ::Array(e) => write!(f, "[]{e}"),
			Typ::FixedArray(e, n) => write!(f, "[{n}]{e}"),
			Typ::Enum(name) => write!(f, "{}", sugar(name).as_deref().unwrap_or(name)),
			Typ::Struct(name, _) => write!(f, "{}", sugar(name).as_deref().unwrap_or(name)),
			Typ::TupleStruct(name, _) | Typ::Trait(name) => write!(f, "{name}"),
			Typ::Sum(name, _) if !name.is_empty() => write!(f, "{name}"),
			Typ::Sum(_, variants) => {
				let members: Vec<_> = (variants.iter())
					.map(|v| {
						if v.payload.is_empty() {
							format!(":{}", v.name)
						} else {
							v.name.clone()
						}
					})
					.collect();
				write!(f, "{}", members.join(" | "))
			}
			Typ::Error => write!(f, "Error"),
			Typ::Annotated(anns, t) if matches!(**t, Typ::Bool) => write!(f, "{}", anns[0]),
			Typ::Annotated(anns, _) if let Some((_, Typ::FixedArray(e, _))) = self.keyed() => {
				write!(f, "{}{e}", anns[0])
			}
			Typ::Annotated(anns, t) if self.variant().is_some() => write!(f, "{t}{}", anns[0]),
			Typ::Annotated(anns, t) => write!(f, "{}{t}", marks(anns)),
			Typ::Fn(params, ret) | Typ::Closure(params, ret, _) => {
				let shown: Vec<_> = (params.iter())
					.map(|p| match (p.variadic, &p.typ) {
						(true, Typ::Array(elem)) => format!("..{elem}"),
						(_, typ) => typ.to_string(),
					})
					.collect();
				write!(f, "fn({}) {ret}", shown.join(", "))
			}
			Typ::Map(k, v) => write!(f, "[{k}]{v}"),
			Typ::Access(a, inner) => write!(f, "{a} {inner}"),
			Typ::Ref(inner) => write!(f, "^{inner}"),
			Typ::Ast => write!(f, "Ast"),
			Typ::Any => write!(f, "any"),
			Typ::TypeId => write!(f, "typeid"),
			Typ::Const(lit) => f.write_str(&lit_text(lit)),
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
			_ if self.bool_bytes().zip(other.bool_bytes()).is_some() => true,
			(Typ::Tuple(a), Typ::Tuple(b)) => a.len() == b.len() && a.iter().zip(b).all(|((_, x), (_, y))| x == y),
			(Typ::TupleStruct(a, _), Typ::TupleStruct(b, _)) => a == b,
			(Typ::Int(a), Typ::Int(b)) | (Typ::UInt(a), Typ::UInt(b)) | (Typ::Float(a), Typ::Float(b)) => a == b,
			(Typ::Struct(n, a), Typ::Struct(m, b)) => n == m && a == b,
			(Typ::Enum(a), Typ::Enum(b)) => a == b,
			(Typ::Trait(a), Typ::Trait(b)) => a == b,
			(Typ::Array(a), Typ::Array(b)) => a == b,
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
	let named = |n: &String| (!n.contains('[')).then(|| TypeExpr::Resolved(n.clone()));
	Some(match typ {
		Typ::Int(_) | Typ::UInt(_) | Typ::ISize | Typ::USize | Typ::Float(_) | Typ::Bool | Typ::Str | Typ::CStr => {
			TypeExpr::Name(typ.to_string())
		}
		Typ::Struct(n, fs) if n.starts_with("struct{") => TypeExpr::AnonStruct(
			fs.iter()
				.map(|f| Some(Param::new(f.name.clone(), type_expr(&f.typ)?, Span::default())))
				.collect::<Option<_>>()?,
		),
		Typ::Struct(n, _) | Typ::TupleStruct(n, _) | Typ::Enum(n) => named(n)?,
		Typ::Array(e) => TypeExpr::Array(Box::new(type_expr(e)?)),
		Typ::FixedArray(e, n) => TypeExpr::FixedArray(
			Box::new(type_expr(e)?),
			Box::new((Expr::Int(*n as i128), Span::default())),
		),
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
		Typ::Const(lit) => TypeExpr::Value(Box::new(((**lit).clone(), Span::default()))),
		_ => return None,
	})
}

// A pack's leading slots as the one value they carry.
pub(crate) fn lead_typ(lead: &[(Option<String>, Typ)]) -> Typ {
	match lead {
		[(_, one)] => one.clone(),
		many => Typ::Tuple(many.to_vec()),
	}
}

// A param's ABI type.
pub(crate) fn param_cl(typ: &Typ, access: Access, int: types::Type) -> types::Type {
	if access == Access::Mut { int } else { cl_type(typ, int) }
}

// A fn's ABI params, then trailing pointers for its ctx and env.
pub(crate) fn abi_params<'t>(
	params: impl Iterator<Item = (&'t Typ, Access)>,
	ptrs: usize,
	int: types::Type,
) -> impl Iterator<Item = AbiParam> {
	let params = params.map(move |(t, a)| AbiParam::new(param_cl(t, a, int)));
	params.chain(std::iter::repeat_n(AbiParam::new(int), ptrs))
}

// A fn's ABI returns, one per tuple or `!T` pack slot.
pub(crate) fn abi_returns(ret: &Typ, types: &TypeCtx, int: types::Type) -> Vec<AbiParam> {
	match types.ret_pack(ret).as_ref().unwrap_or(ret) {
		Typ::Tuple(fields) => fields.iter().map(|(_, t)| AbiParam::new(cl_type(t, int))).collect(),
		ret => vec![AbiParam::new(cl_type(ret, int))],
	}
}

// A C ABI fn's params and returns, `@c` structs spread per the target.
pub(crate) fn c_abi_sig<'t>(
	sig: &mut Signature,
	params: impl Iterator<Item = (&'t Typ, Access)> + Clone,
	ret: &Typ,
	types: &TypeCtx,
	arch: Architecture,
	int: types::Type,
) {
	let spread = c_spread(params.clone().map(|(t, _)| t), ret, types, arch);
	sig.params.extend(params.zip(spread).flat_map(|((t, a), regs)| match regs {
		Some(regs) => regs.into_iter().map(|(_, p)| p).collect(),
		None => vec![AbiParam::new(param_cl(t, a, int))],
	}));
	match ret.c_by_val(types, arch).as_deref() {
		Some([(None, _)]) => sig.params.insert(0, AbiParam::special(int, ArgumentPurpose::StructReturn)),
		Some(regs) => sig.returns.extend(regs.iter().map(|(_, p)| *p)),
		None => sig.returns.extend(abi_returns(ret, types, int)),
	}
}

pub(crate) type CRegs = Vec<(Option<i32>, AbiParam)>;

// Each param's `@c` struct ABI params, whole on the stack once SysV runs out of int or SSE regs for it.
pub(crate) fn c_spread<'t>(
	params: impl Iterator<Item = &'t Typ>,
	ret: &Typ,
	types: &TypeCtx,
	arch: Architecture,
) -> Vec<Option<CRegs>> {
	let sret = matches!(ret.c_by_val(types, arch).as_deref(), Some([(None, _)]));
	let mut free = [6 - sret as usize, 8];
	let spread = |t: &Typ| {
		let regs = t.c_by_val(types, arch);
		let mut need = [0, 0];
		for (_, p) in regs.iter().flatten().filter(|r| r.0.is_some()) {
			need[p.value_type.is_float() as usize] += 1;
		}
		if regs.is_none() {
			need[matches!(t.newtype().unwrap_or(t), Typ::Float(_)) as usize] = 1;
		}
		if regs.is_some() && arch == Architecture::X86_64 && (need[0] > free[0] || need[1] > free[1]) {
			let sarg = ArgumentPurpose::StructArgument(t.c_size_align(types)?.0.next_multiple_of(8));
			return Some(vec![(None, AbiParam::special(types::I64, sarg))]);
		}
		free = [free[0].saturating_sub(need[0]), free[1].saturating_sub(need[1])];
		regs
	};
	params.map(spread).collect()
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

// Bounds of a `width`-bit int, as i128 bits.
pub(crate) fn int_min(width: u16) -> i128 {
	i128::MIN >> (128 - width)
}

pub(crate) fn int_max(width: u16) -> i128 {
	i128::MAX >> (128 - width)
}

pub(crate) fn uint_max(width: u16) -> i128 {
	(u128::MAX >> (128 - width)) as i128
}

pub(crate) fn oi_symbol(name: &str) -> String {
	format!("oi_{}", name.replace('.', "__").replace("::", "$"))
}

// Spell the core Option/Result instances back as `?T` / `!T` where possible.
pub(crate) fn sugar(name: &str) -> Option<String> {
	let args = |base: &str| name.strip_prefix(base)?.strip_prefix('[')?.strip_suffix(']');
	if let Some(inner) = args(role::OPTION) {
		return Some(format!("?{inner}"));
	}
	if let Some(inner) = args(role::BITS) {
		return Some(format!("[{inner}]()"));
	}
	let parts = args(role::RESULT)?;
	Some(match parts.strip_suffix(", Error") {
		Some(ok) => format!("!{ok}"),
		None => format!("Result[{parts}]"),
	})
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
		33..=64 => types::I64,
		_ => types::I128,
	}
}

pub(crate) fn cl_type(typ: &Typ, int: types::Type) -> types::Type {
	match typ.newtype().unwrap_or(typ) {
		Typ::Int(w) | Typ::UInt(w) => cl_int_for_width(*w),
		Typ::Rune => types::I32,
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
		Typ::Rune => 4,
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

// Tag permutation to coerce a sum into a superset sum.
pub(crate) fn sum_remap(src: &[VariantInfo], dst: &[VariantInfo]) -> Option<Vec<(i64, i64)>> {
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

// The tag an `any` box carries for a type.
pub(crate) fn typeid(t: &Typ) -> i64 {
	let mut h = DefaultHasher::new();
	(t.key(), t.to_string()).hash(&mut h);
	h.finish() as i64
}
