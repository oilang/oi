// The compiler resolves in `core`.

// traits
pub(crate) const EQ: &str = "core::Eq";
pub(crate) const ORD: &str = "core::Ord";
pub(crate) const NEG: &str = "core::Neg";
pub(crate) const ADD: &str = "core::Add";
pub(crate) const SUB: &str = "core::Sub";
pub(crate) const MUL: &str = "core::Mul";
pub(crate) const DIV: &str = "core::Div";
pub(crate) const MOD: &str = "core::Mod";
pub(crate) const POW: &str = "core::Pow";
pub(crate) const NOT: &str = "core::Not";
pub(crate) const BIT_AND: &str = "core::BitAnd";
pub(crate) const BIT_OR: &str = "core::BitOr";
pub(crate) const BIT_XOR: &str = "core::BitXor";
pub(crate) const SHL: &str = "core::Shl";
pub(crate) const SHR: &str = "core::Shr";
pub(crate) const BITWISE: [&str; 6] = [NOT, BIT_AND, BIT_OR, BIT_XOR, SHL, SHR];
pub(crate) const CONTAINS: &str = "core::Contains";
pub(crate) const ITERATOR: &str = "core::Iterator";
pub(crate) const ITERABLE: &str = "core::Iterable";
pub(crate) const ERROR: &str = "core::Error";

// types
pub(crate) const OPTION: &str = "core::Option";
pub(crate) const RESULT: &str = "core::Result";
pub(crate) const PTR: &str = "core::ptr";
pub(crate) const RANGE: &str = "core::Range";
pub(crate) const SRC: &str = "core::Src";

// fns
pub(crate) const ZERO: &str = "core::zero";
pub(crate) const STR_CONTAINS: &str = "string.contains";
pub(crate) const ORIGIN: &str = "core::origin";
pub(crate) const RAISE: &str = "core::raise";

// annotations
pub(crate) const PARAMS: &str = "core::params";
pub(crate) const REQUIRED: &str = "core::required";
pub(crate) const TEST: &str = "core::test";
pub(crate) const LINK: &str = "core::link";
pub(crate) const EXPORT: &str = "core::export";
pub(crate) const C: &str = "core::c";
pub(crate) const IMPLICIT: &str = "core::implicit";
pub(crate) const PURE: &str = "core::pure";
pub(crate) const NOZERO: &str = "core::nozero";

// Annotation markers resolve to core even where a local name shadows them.
pub(crate) fn marker(name: &str) -> Option<&'static str> {
	[PARAMS, REQUIRED, TEST, LINK, EXPORT, C, IMPLICIT, PURE, NOZERO]
		.into_iter()
		.find(|m| m.strip_prefix("core::") == Some(name))
}
