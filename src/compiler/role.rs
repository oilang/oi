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
pub(crate) const ERROR: &str = "core::Error";

// types
pub(crate) const PTR: &str = "core::ptr";
pub(crate) const RANGE: &str = "core::Range";

// fns
pub(crate) const ZERO: &str = "core::zero";
pub(crate) const STR_CONTAINS: &str = "string.contains";
pub(crate) const RANGE_CONTAINS: &str = "core::Range.contains";
pub(crate) const PARSE_INT: &str = "core::parse_int";
pub(crate) const PARSE_FLOAT: &str = "core::parse_float";

// annotations
pub(crate) const PARAMS: &str = "core::params";
pub(crate) const REQUIRED: &str = "core::required";
pub(crate) const TEST: &str = "core::test";
pub(crate) const LINK: &str = "core::link";
pub(crate) const EXPORT: &str = "core::export";
pub(crate) const C: &str = "core::c";
pub(crate) const IMPLICIT: &str = "core::implicit";
pub(crate) const PURE: &str = "core::pure";
