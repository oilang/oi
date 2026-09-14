use std::collections::{HashMap, HashSet};
use std::ops::Range;

use cranelift::codegen;
use cranelift::codegen::ir::immediates::{Ieee16, Ieee128};
use cranelift::codegen::ir::{StackSlotData, StackSlotKind};
use cranelift::prelude::*;
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};

use super::{
	Consts, FieldDef, FnParam, FnSig, GenericFnDef, GenericStructDef, Generics, Local, LoopFrame, Pending, TraitItem,
	Typ, TypeCtx, VariantInfo, access_of, access_peel, access_wrap, ann_names, builtin_claim, c_layout, check_ann_typ,
	check_c_sig, cl_int_for_width, cl_type, display_name, elem_size, embeds, enum_boxed, enum_slots, is_c_struct,
	is_range, mentions, oi_symbol, option_variants, result_variants, sum_remap, trait_fns, type_expr, typeid,
};
use crate::ast::{Access, Annotation, BinOp, Bounds, Expr, MatchArm, Span, Spanned, TypeExpr};
use crate::diagnostics::{Diagnostic, SourceMap};
use crate::loader::Scope;
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
use self::core::ambiguous;
use self::helpers::*;

pub(super) struct Translator<'a, M: Module> {
	pub int: types::Type,
	pub b: FunctionBuilder<'a>,
	pub vars: HashMap<String, Local>,
	pub params: Vec<Local>,
	pub dollar: Option<TypedVal>,
	pub module: &'a mut M,
	pub funcs: &'a HashMap<String, FnSig>,
	pub structs: &'a HashMap<String, Vec<FieldDef>>,
	pub enums: &'a HashMap<String, Vec<VariantInfo>>,
	pub aliases: &'a HashMap<String, TypeExpr>,
	pub type_params: &'a HashMap<String, Typ>,
	pub generics: &'a Generics,
	pub traits: &'a HashMap<&'a str, TraitItem<'a>>,
	pub generic_fns: &'a HashMap<String, GenericFnDef>,
	pub trait_impls: &'a HashSet<(String, String)>,
	pub core_traits: &'a HashSet<String>,
	pub scope: &'a Scope,
	pub module_scopes: &'a HashMap<String, Scope>,
	pub map: &'a SourceMap,
	pub publics: &'a HashSet<String>,
	pub privates: &'a HashMap<String, HashSet<String>>,
	pub reexports: &'a HashMap<String, String>,
	pub consts: &'a HashMap<String, Spanned<Expr>>,
	pub annotations: &'a HashMap<String, Vec<Annotation>>,
	pub mono: &'a mut HashMap<String, FnSig>,
	pub pending: &'a mut Vec<Pending>,
	pub printers: &'a mut Vec<(String, Typ, bool, runtime::Sink)>,
	pub descs: &'a mut HashMap<String, DataId>,
	pub string_idx: &'a mut usize,
	pub atoms: &'a mut HashSet<String>,
	pub ret: Option<(Typ, Span)>,
	pub loops: Vec<LoopFrame>,
	pub unsafely: usize,
	pub scopes: Vec<Vec<(Variable, Typ)>>,
	pub temps: HashMap<Value, Variable>,
	pub self_type: Option<String>,
	pub is_main: bool,
	pub self_name: Option<String>,
	pub pure: bool,
}

// A statement that writes through an existing, mutable binding.
#[derive(Clone, Copy)]
enum Mutation {
	Assign,      // `x = v`
	IndexAssign, // `x[i] = v`
	Append,      // `x << v`
	FieldAssign, // `x.f = v`
}

// A destructured binding.
// `(name, type, offset)`
type Bind = (String, Typ, i32);
pub(super) type TypedVal = (Value, Typ);
