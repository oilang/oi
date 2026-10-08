use std::collections::{HashMap, HashSet};
use std::ops::Range;

use cranelift::codegen;
use cranelift::codegen::ir::immediates::{Ieee16, Ieee128};
use cranelift::codegen::ir::{StackSlotData, StackSlotKind};
use cranelift::prelude::*;
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, ModuleError};

use super::{
	Artifacts, CTX, FieldDef, FnParam, FnSig, GenericEnumDef, GenericFnDef, GenericStructDef, Generics, Local,
	LoopFrame, Typ, TypeCtx, VariantInfo, World, access_of, access_peel, access_wrap, ann_names, builtin_claim,
	c_layout, check_ann_typ, check_c_sig, check_reserved, cl_int_for_width, cl_type, display_name, elem_size, embeds,
	enum_boxed, enum_slots, has_ann, is_c_struct, is_range, mentions, oi_symbol, param_cl, role, sugar, sum_remap,
	trait_fns, type_expr, typeid,
};
use crate::ast::{Access, BinOp, Bounds, Expr, MatchArm, Span, Spanned, TypeExpr, place};
use crate::diagnostics::{Diagnostic, arity_err, fail, unknown_member};
use crate::loader::{Scope, module_of};
use crate::runtime;

mod anon;
mod array;
mod builtin;
mod call;
mod control;
mod core;
mod expr;
mod ffi;
mod generic;
mod helpers;
mod macros;
mod op;
mod pipe;
mod print;
mod rc;
mod stmt;
pub(crate) mod value;

use self::anon::AnonSig;
use self::call::Callee;
use self::helpers::*;

pub(super) struct Translator<'a, M: Module> {
	pub int: types::Type,
	pub b: FunctionBuilder<'a>,
	pub vars: Vars,
	pub params: Vec<Local>,
	pub dollar: Option<TypedVal>,
	pub module: &'a mut M,
	pub funcs: &'a HashMap<String, FnSig>,
	pub types: TypeCtx<'a>,
	pub world: &'a World,
	pub out: &'a mut Artifacts,
	pub c_callback: bool,
	pub comptime: bool,
	pub ret: Option<(Typ, Span)>,
	pub loops: Vec<LoopFrame>,
	pub unsafely: usize,
	pub scopes: Vec<Vec<(Variable, Typ)>>,
	pub defers: Vec<Vec<rc::Defer>>,
	pub deferring: bool,
	pub temps: HashMap<Value, Variable>,
	pub self_type: Option<String>,
	pub is_main: bool,
	pub script: bool,
	pub self_name: Option<String>,
	pub pure: bool,
	pub ctx_used: bool,
	pub anon_ctx: Option<String>,
	pub slots: Vec<String>,
	pub addressed: HashSet<String>,
	pub aliases: Vec<Variable>,
	pub flagged: Vec<Variable>,
	pub withs: Vec<String>,
}

// Bindings in scope, with an undo log so `scoped` restores what a child scope changed.
#[derive(Default)]
pub(super) struct Vars {
	map: HashMap<String, Local>,
	log: Vec<(String, Option<Local>)>,
	marks: Vec<usize>,
}

impl std::ops::Deref for Vars {
	type Target = HashMap<String, Local>;
	fn deref(&self) -> &Self::Target {
		&self.map
	}
}

impl From<HashMap<String, Local>> for Vars {
	fn from(map: HashMap<String, Local>) -> Self {
		Vars {
			map,
			..Default::default()
		}
	}
}

impl Vars {
	fn mark(&mut self) {
		self.marks.push(self.log.len());
	}

	fn undo(&mut self) {
		let mark = self.marks.pop().expect("an open mark");
		for (name, old) in self.log.drain(mark..).rev() {
			match old {
				Some(local) => self.map.insert(name, local),
				None => self.map.remove(&name),
			};
		}
	}

	pub(super) fn insert(&mut self, name: String, local: Local) -> Option<Local> {
		let old = self.map.insert(name.clone(), local);
		self.log(name, &old);
		old
	}

	fn remove(&mut self, name: &str) -> Option<Local> {
		let old = self.map.remove(name);
		self.log(name.to_string(), &old);
		old
	}

	fn retain(&mut self, keep: impl Fn(&String, &Local) -> bool) {
		let gone: Vec<_> = self.map.iter().filter(|&(n, l)| !keep(n, l)).map(|(n, _)| n.clone()).collect();
		gone.iter().for_each(|n| _ = self.remove(n));
	}

	fn log(&mut self, name: String, old: &Option<Local>) {
		if !self.marks.is_empty() {
			self.log.push((name, old.clone()));
		}
	}
}

// A statement that writes through an existing, mutable binding.
#[derive(Clone, Copy)]
enum Mutation {
	Assign,      // `x = v`
	IndexAssign, // `x[i] = v`
	Append,      // `x << v`
	FieldAssign, // `x.f = v`
	DerefAssign, // `x^ = v`
}

// A destructured binding.
// `(name, type, offset)`
type Bind = (String, Typ, i32);
pub(super) type TypedVal = (Value, Typ);
