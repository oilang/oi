// The compiler resolves in `core`.

use std::sync::OnceLock;

use rustc_hash::FxHashMap;

use super::ann;
use crate::ast::Expr;
use crate::loader::Program;

// A key core binds to a def with `@lang`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Role(pub(crate) &'static str);

// Role keys to def paths, and back.
static TABLE: OnceLock<FxHashMap<String, String>> = OnceLock::new();

// Bind each role to the core def its `@lang` marks.
pub(crate) fn fill(core: &Program) {
	TABLE.get_or_init(|| {
		let mut table = FxHashMap::default();
		for (def, anns) in &core.annotations {
			let key = match anns.iter().find_map(|note| ann(note, "lang")) {
				Some([]) => def.rsplit("::").next().unwrap(),
				Some([(None, (Expr::String(key), _))]) => key,
				_ => continue,
			};
			let dup = table.insert(key.to_string(), def.clone());
			assert!(dup.is_none(), "`@lang` binds `{key}` twice");
			table.insert(def.clone(), key.to_string());
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
pub(crate) const EQ: Role = Role("Eq");
pub(crate) const PARTIAL_CMP: Role = Role("PartialCmp");
pub(crate) const CMP: Role = Role("Cmp");
pub(crate) const NEG: Role = Role("Neg");
pub(crate) const ADD: Role = Role("Add");
pub(crate) const SUB: Role = Role("Sub");
pub(crate) const MUL: Role = Role("Mul");
pub(crate) const DIV: Role = Role("Div");
pub(crate) const MOD: Role = Role("Mod");
pub(crate) const POW: Role = Role("Pow");
pub(crate) const NOT: Role = Role("Not");
pub(crate) const BIT_AND: Role = Role("BitAnd");
pub(crate) const BIT_OR: Role = Role("BitOr");
pub(crate) const BIT_XOR: Role = Role("BitXor");
pub(crate) const SHL: Role = Role("Shl");
pub(crate) const SHR: Role = Role("Shr");
pub(crate) const BITWISE: [Role; 6] = [NOT, BIT_AND, BIT_OR, BIT_XOR, SHL, SHR];
pub(crate) const CONTAINS: Role = Role("Contains");
pub(crate) const SEND: Role = Role("Send");
pub(crate) const RECV: Role = Role("Recv");
pub(crate) const INDEX: Role = Role("Index");
pub(crate) const INDEX_ASSIGN: Role = Role("IndexAssign");
pub(crate) const ITERATOR: Role = Role("Iterator");
pub(crate) const KEYED: Role = Role("Keyed");
pub(crate) const ERROR: Role = Role("Error");
pub(crate) const ALLOCATOR: Role = Role("Allocator");
pub(crate) const DISPLAY: Role = Role("Display");
pub(crate) const DEBUG: Role = Role("Debug");
pub(crate) const NUMBER: Role = Role("Number");
pub(crate) const INTEGER: Role = Role("Integer");
pub(crate) const LENGTH: Role = Role("Length");
pub(crate) const WRITE: Role = Role("Write");
pub(crate) const ORDINAL: Role = Role("Ordinal");
pub(crate) const ISOLATED_TRAIT: Role = Role("Isolated");
pub(crate) const LINEAR: Role = Role("Linear");

// types
pub(crate) const OPTION: Role = Role("Option");
pub(crate) const RESULT: Role = Role("Result");
pub(crate) const PACK_ERR: &str = "$err";
pub(crate) const PACK_OK: &str = "$ok";
pub(crate) const PTR: Role = Role("ptr");
pub(crate) const RANGE: Role = Role("Range");
pub(crate) const BITS: Role = Role("Bits");
pub(crate) const SRC: Role = Role("Src");
pub(crate) const ALLOC: Role = Role("Alloc");
pub(crate) const SINK: Role = Role("Sink");

// fns
pub(crate) const ZERO: Role = Role("zero");
pub(crate) const ALLOC_SHIM: Role = Role("alloc_shim");
pub(crate) const STR_CONTAINS: &str = "string.contains";
pub(crate) const ORIGIN: Role = Role("origin");
pub(crate) const RAISE: Role = Role("raise");

// annotations
pub(crate) const PARAMS: Role = Role("params");
pub(crate) const REQUIRED: Role = Role("required");
pub(crate) const TEST: Role = Role("test");
pub(crate) const LINK: Role = Role("link");
pub(crate) const EXPORT: Role = Role("export");
pub(crate) const C: Role = Role("c");
pub(crate) const IMPLICIT: Role = Role("implicit");
pub(crate) const PURE: Role = Role("pure");
pub(crate) const CTX: &str = "core::ctx";
pub(crate) const NOZERO: Role = Role("nozero");
pub(crate) const NOINIT: Role = Role("noinit");
pub(crate) const OPEN: Role = Role("open");
pub(crate) const UNOWNED: &str = "core::unowned";
pub(crate) const ISOLATED: Role = Role("isolated");
pub(crate) const INTRINSIC: Role = Role("intrinsic");

// Annotation markers resolve to core unless an import shadows them.
pub(crate) fn marker(name: &str) -> Option<&'static str> {
	let markers = [
		PARAMS, REQUIRED, TEST, LINK, EXPORT, C, IMPLICIT, PURE, NOZERO, NOINIT, OPEN, ISOLATED,
	];
	match name {
		"ctx" => Some(CTX),
		_ => markers.iter().find(|role| role.0 == name).map(|role| role.path()),
	}
}
