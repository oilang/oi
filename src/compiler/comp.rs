// Compile-time evaluation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ast::{Annotation, Expr, Span, Spanned};
use crate::diagnostics::Diagnostic;
use crate::loader::{Module, Program};
use crate::runtime;

use super::Compiler;

pub(crate) const RT_COMP_YIELD: &str = "oi_rt_comp_yield";
pub(crate) const RT_COMP_STRUCT: &str = "oi_rt_comp_struct";
pub(crate) const UNREIFIABLE: &str = "can't use this type in `comp` yet";

pub(crate) const TAG_INT: i64 = 0;
pub(crate) const TAG_FLOAT: i64 = 1;
pub(crate) const TAG_BOOL: i64 = 2;
pub(crate) const TAG_STR: i64 = 3;
pub(crate) const TAG_UNIT: i64 = 4;
pub(crate) const TAG_ARRAY: i64 = 5;

enum Entry {
	Scalar(i64, i64),
	Struct(String, usize),
}

static SITE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
	static STACK: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

pub(crate) extern "C" fn rt_comp_yield(tag: i64, val: i64) {
	STACK.with_borrow_mut(|s| s.push(Entry::Scalar(tag, val)));
}

pub(crate) extern "C" fn rt_comp_struct(name: *const runtime::StrHeader, nfields: i64) {
	// SAFETY: `name` is a str constant the JIT just built
	let name = String::from_utf8_lossy(unsafe { runtime::str_bytes(name) }).into_owned();
	STACK.with_borrow_mut(|s| s.push(Entry::Struct(name, nfields as usize)));
}

// Find the first unevaluated comp site.
fn first_comp(items: &mut [Spanned<Expr>]) -> Option<Spanned<Expr>> {
	let mut found = None;
	for (e, _) in items.iter_mut() {
		e.walk(&mut |x| {
			if found.is_none()
				&& let Expr::Comp(inner) = x
			{
				found = Some((**inner).clone());
			}
		});
	}
	found
}

// Swap comp site for its folded literal.
fn patch_first(items: &mut [Spanned<Expr>], lit: Expr) {
	let mut lit = Some(lit);
	for (e, _) in items.iter_mut() {
		e.walk(&mut |x| {
			if matches!(x, Expr::Comp(_))
				&& let Some(lit) = lit.take()
			{
				*x = lit;
			}
		});
	}
}

fn has_comp(e: &mut Expr) -> bool {
	let mut found = false;
	e.walk(&mut |x| found |= matches!(x, Expr::Comp(_)));
	found
}

fn is_def(e: &Expr) -> bool {
	matches!(
		e,
		Expr::Fn { .. }
			| Expr::StructDef { .. }
			| Expr::EnumDef { .. }
			| Expr::TraitDef { .. }
			| Expr::TypeAlias { .. }
			| Expr::Claim { .. }
			| Expr::Bind { .. }
	)
}

// Decode one scalar into its literal.
pub(crate) fn scalar(tag: i64, v: i64) -> Expr {
	match tag {
		TAG_INT => Expr::Int(v),
		TAG_FLOAT => Expr::Float(f64::from_bits(v as u64)),
		TAG_BOOL => Expr::Bool(v != 0),
		// SAFETY: `v` is a str handle the runtime just produced
		TAG_STR => Expr::String(String::from_utf8_lossy(unsafe { runtime::str_bytes(v as *const _) }).into_owned()),
		_ => Expr::Tuple(vec![]),
	}
}

// Decode the stack.
fn reify(span: Span) -> Expr {
	STACK
		.with_borrow_mut(|stack| {
			let mut exprs: Vec<Expr> = Vec::new();
			for entry in stack.drain(..) {
				let e = match entry {
					Entry::Scalar(TAG_ARRAY, n) => {
						let at = exprs.len() - n as usize;
						Expr::Array(exprs.split_off(at).into_iter().map(|e| (e, span)).collect())
					}
					Entry::Scalar(tag, v) => scalar(tag, v),
					Entry::Struct(name, nfields) => {
						let at = exprs.len() - nfields;
						let fields = exprs.split_off(at).into_iter().map(|e| (None, (e, span))).collect();
						Expr::StructLit {
							name,
							type_args: vec![],
							fields,
						}
					}
				};
				exprs.push(e);
			}
			exprs.pop()
		})
		.expect("a comp site always yields exactly one value")
}

// Compile and run a single comp site, folding it to its reified literal.
fn fold(
	mut inner: Spanned<Expr>,
	target: &str,
	expanded: &mut HashMap<String, Vec<Spanned<Expr>>>,
	consts: &HashMap<String, Spanned<Expr>>,
	program: &Program,
	stage0: &mut Option<Compiler>,
) -> Result<Expr, Diagnostic> {
	// conditional compilation
	while let Expr::If { cond, then, els } = inner.0 {
		let span = cond.1;
		let mut arm = match fold(*cond, target, expanded, consts, program, stage0)? {
			Expr::Bool(true) => then,
			Expr::Bool(false) if matches!(els.as_deref(), Some([(Expr::If { .. }, _)])) => {
				inner = els.expect("guard matched").remove(0);
				continue;
			}
			Expr::Bool(false) => els.unwrap_or_default(),
			_ => return Err(Diagnostic::new("`comp if` needs a bool condition", span.into_range())),
		};
		return Ok(if arm.len() == 1 {
			arm.remove(0).0
		} else {
			Expr::Block(arm)
		});
	}
	let span = inner.1;
	// every needs its own name
	let name = format!("__comp{}", SITE.fetch_add(1, Ordering::Relaxed));
	let thunk = (
		Expr::Fn {
			name: name.clone(),
			type_params: vec![],
			params: vec![],
			params_tuple: true,
			ret: None,
			body: vec![(
				Expr::Call {
					name: "__comp_yield".into(),
					type_args: vec![],
					args: vec![inner],
				},
				span,
			)],
		},
		span,
	);
	// enable comp code to call imports
	let modules: Vec<Module> = program
		.modules
		.iter()
		.map(|m| {
			let mut items: Vec<Spanned<Expr>> = expanded
				.get_mut(&m.name)
				.expect("every module is expanded")
				.iter_mut()
				.filter_map(|it| (is_def(&it.0) && !has_comp(&mut it.0)).then(|| it.clone()))
				.collect();
			if m.name == target {
				items.push(thunk.clone());
			}
			Module {
				name: m.name.clone(),
				items,
				scope: m.scope.clone(),
			}
		})
		.collect();
	let mut annotations = program.annotations.clone();
	annotations
		.values_mut()
		.for_each(|v| v.retain(|a| !matches!(a.0, Expr::Call { .. })));
	let synthetic = Program {
		map: program.map.clone(),
		modules,
		publics: program.publics.clone(),
		reexports: program.reexports.clone(),
		consts: consts
			.iter()
			.filter(|(_, v)| !matches!(v.0, Expr::Comp(_)))
			.map(|(k, v)| (k.clone(), v.clone()))
			.collect(),
		annotations,
		roots: program.roots.clone(),
		core_origin: program.core_origin.clone(),
	};
	let compiler = stage0.get_or_insert_with(Compiler::default);
	compiler.roots = vec![name.clone()];
	compiler.compile(&synthetic)?;
	let f = compiler.module.get_finalized_function(compiler.hoisted[&name].id);
	// SAFETY: fn takes no args and returns unit
	unsafe { std::mem::transmute::<*const u8, fn()>(f)() };
	Ok(reify(span))
}

// Fold every `comp` site, and every call in annotation position, to a literal.
pub(crate) fn eval(
	expanded: &mut HashMap<String, Vec<Spanned<Expr>>>,
	annotations: &mut HashMap<String, Vec<Annotation>>,
	consts: &mut HashMap<String, Spanned<Expr>>,
	program: &Program,
	stage0: &mut Option<Compiler>,
) -> Result<(), Diagnostic> {
	// push order is importer-first, so folding in reverse handles dependencies first
	for m in program.modules.iter().rev() {
		let mut comps: Vec<String> = consts
			.iter()
			.filter(|(k, v)| {
				k.split_once("::").map_or("main", |(owner, _)| owner) == m.name && matches!(v.0, Expr::Comp(_))
			})
			.map(|(k, _)| k.clone())
			.collect();
		// comp expressions may reference siblings, so retry until a pass gets stuck
		while !comps.is_empty() {
			let (n, mut err) = (comps.len(), None);
			comps.retain(|k| {
				let (Expr::Comp(inner), span) = consts[k].clone() else {
					unreachable!()
				};
				match fold(*inner, &m.name, expanded, consts, program, stage0) {
					Ok(lit) => {
						consts.insert(k.clone(), (lit, span));
						false
					}
					Err(d) => {
						err = Some(d);
						true
					}
				}
			});
			if comps.len() == n {
				return Err(err.expect("a stuck pass saw an error"));
			}
		}
		while let Some(inner) = first_comp(expanded.get_mut(&m.name).expect("every module is expanded")) {
			let lit = fold(inner, &m.name, expanded, consts, program, stage0)?;
			patch_first(expanded.get_mut(&m.name).expect("every module is expanded"), lit);
		}
	}
	// an annotation call is implicitly comptime
	for a in annotations.iter_mut().filter(|(k, _)| !k.contains("::")).flat_map(|(_, v)| v) {
		if matches!(a.0, Expr::Call { .. }) {
			a.0 = fold(a.clone(), "main", expanded, consts, program, stage0)?;
		}
	}
	Ok(())
}
