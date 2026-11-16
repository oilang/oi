// The compiler resolves in `core`.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::ann;
use crate::ast::Expr;
use crate::loader::Program;

// A key core binds to a def with `@lang`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Role(pub(crate) &'static str);

// Role keys to def paths, and back.
static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();

// Bind each role to the core def its `@lang` marks.
pub(crate) fn fill(core: &Program) {
	TABLE.get_or_init(|| {
		let mut table = HashMap::new();
		for (def, anns) in &core.annotations {
			if let Some([(None, (Expr::String(key), _))]) = anns.iter().find_map(|note| ann(note, "lang")) {
				let dup = table.insert(key.clone(), def.clone());
				assert!(dup.is_none(), "`@lang` binds `{key}` twice");
				table.insert(def.clone(), key.clone());
			}
		}
		table
	});
}

// The role a def binds.
pub(crate) fn key(path: &str) -> Option<Role> {
	Some(Role(TABLE.get()?.get(path)?))
}

impl Role {
	// The def a role binds, empty while core lacks it.
	pub(crate) fn path(self) -> &'static str {
		TABLE.get().and_then(|table| table.get(self.0)).map_or("", String::as_str)
	}
}

// traits
pub(crate) const EQ: Role = Role("eq");
pub(crate) const PARTIAL_CMP: Role = Role("partial_cmp");
pub(crate) const CMP: Role = Role("cmp");
pub(crate) const NEG: Role = Role("neg");
pub(crate) const ADD: Role = Role("add");
pub(crate) const SUB: Role = Role("sub");
pub(crate) const MUL: Role = Role("mul");
pub(crate) const DIV: Role = Role("div");
pub(crate) const MOD: Role = Role("mod");
pub(crate) const POW: Role = Role("pow");
pub(crate) const NOT: Role = Role("not");
pub(crate) const BIT_AND: Role = Role("bitand");
pub(crate) const BIT_OR: Role = Role("bitor");
pub(crate) const BIT_XOR: Role = Role("bitxor");
pub(crate) const SHL: Role = Role("shl");
pub(crate) const SHR: Role = Role("shr");
pub(crate) const BITWISE: [Role; 6] = [NOT, BIT_AND, BIT_OR, BIT_XOR, SHL, SHR];
pub(crate) const CONTAINS: Role = Role("contains");
pub(crate) const RECV: Role = Role("recv");
pub(crate) const INDEX: Role = Role("index");
pub(crate) const INDEX_ASSIGN: Role = Role("index_assign");
pub(crate) const ITERATOR: Role = Role("iterator");
pub(crate) const KEYED: Role = Role("keyed");
pub(crate) const ERROR: Role = Role("error");
pub(crate) const ALLOCATOR: Role = Role("allocator");
pub(crate) const DISPLAY: Role = Role("display");
pub(crate) const DEBUG: Role = Role("debug");
pub(crate) const NUMBER: Role = Role("number");
pub(crate) const INTEGER: Role = Role("integer");
pub(crate) const LENGTH: Role = Role("length");
pub(crate) const WRITE: Role = Role("write");
pub(crate) const ORDINAL: Role = Role("ordinal");
pub(crate) const ISOLATED_TRAIT: Role = Role("isolated_trait");
pub(crate) const LINEAR: Role = Role("linear");

// types
pub(crate) const OPTION: &str = "core::Option";
pub(crate) const RESULT: &str = "core::Result";
pub(crate) const PACK_ERR: &str = "$err";
pub(crate) const PACK_OK: &str = "$ok";
pub(crate) const PTR: &str = "core::ptr";
pub(crate) const RANGE: &str = "core::Range";
pub(crate) const BITS: &str = "core::Bits";
pub(crate) const SRC: &str = "core::Src";
pub(crate) const ALLOC: &str = "core::Alloc";
pub(crate) const SINK: &str = "core::Sink";

// fns
pub(crate) const ZERO: &str = "core::zero";
pub(crate) const ALLOC_SHIM: &str = "core::alloc_shim";
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
pub(crate) const CTX: &str = "core::ctx";
pub(crate) const NOZERO: &str = "core::nozero";
pub(crate) const NOINIT: &str = "core::noinit";
pub(crate) const OPEN: &str = "core::open";
pub(crate) const UNOWNED: &str = "core::unowned";
pub(crate) const ISOLATED: &str = "core::isolated";

// Annotation markers resolve to core even where a local name shadows them.
pub(crate) fn marker(name: &str) -> Option<&'static str> {
	[
		PARAMS, REQUIRED, TEST, LINK, EXPORT, C, IMPLICIT, PURE, CTX, NOZERO, NOINIT, OPEN, ISOLATED,
	]
	.into_iter()
	.find(|m| m.strip_prefix("core::") == Some(name))
}
