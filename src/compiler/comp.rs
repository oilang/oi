// Compile-time evaluation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ast::{Annotation, Child, Expr, Span, Spanned};
use crate::diagnostics::{Diagnostic, SourceMap};
use crate::loader::{Program, module_of};
use crate::runtime;

use super::{Compiler, role};

pub(crate) const RT_COMP_YIELD: &str = "oi_rt_comp_yield";
pub(crate) const RT_COMP_STRUCT: &str = "oi_rt_comp_struct";
pub(crate) const RT_COMP_PANIC: &str = "oi_rt_comp_panic";
pub(crate) const UNREIFIABLE: &str = "can't use this type in `comp` yet";

pub(crate) const TAG_INT: i64 = 0;
pub(crate) const TAG_FLOAT: i64 = 1;
pub(crate) const TAG_BOOL: i64 = 2;
pub(crate) const TAG_STR: i64 = 3;
pub(crate) const TAG_UNIT: i64 = 4;
pub(crate) const TAG_ARRAY: i64 = 5;
pub(crate) const TAG_AST: i64 = 6;
pub(crate) const TAG_AST_SEQ: i64 = 7;

enum Entry {
	Scalar(i64, i64),
	Struct(String, usize),
}

static SITE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
	static STACK: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
	static MAP: RefCell<SourceMap> = RefCell::default();
}

pub(crate) extern "C" fn rt_comp_yield(tag: i64, val: i64) {
	STACK.with_borrow_mut(|s| s.push(Entry::Scalar(tag, val)));
}

pub(crate) extern "C" fn rt_comp_struct(name: *const runtime::StrHeader, nfields: i64) {
	// SAFETY: `name` is a str constant the JIT just built
	let name = unsafe { runtime::str_string(name) };
	STACK.with_borrow_mut(|s| s.push(Entry::Struct(name, nfields as usize)));
}

// Stage 0 report and exit.
pub(crate) extern "C" fn rt_comp_panic(assert: i64, msg: *const runtime::StrHeader, at: i64, end: i64) {
	let what = if assert != 0 { "assertion failed" } else { "panic" };
	// SAFETY: `msg` is a str handle the JIT just built
	let msg = format!("{what}: {}", unsafe { runtime::str_string(msg) });
	MAP.with_borrow(|map| Diagnostic::new(msg, at as usize..end as usize).report_mapped(map));
	std::process::exit(1);
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

// Unquote every path rooted at `var`.
fn unquote(e: &mut Spanned<Expr>, var: &str) {
	fn rooted(e: &Expr, var: &str) -> bool {
		match e {
			Expr::Ident(n) => n == var,
			Expr::Field { tuple: r, .. } | Expr::MethodCall { recv: r, .. } | Expr::Index { collection: r, .. } => {
				rooted(&r.0, var)
			}
			_ => false,
		}
	}
	if rooted(&e.0, var) {
		e.0 = Expr::UnquoteExpr(Box::new(e.clone()));
		return;
	}
	e.0.for_children(|c| match c {
		Child::List(list) => list.iter_mut().for_each(|e| unquote(e, var)),
		Child::One(e) => unquote(e, var),
	});
}

fn has_comp(e: &mut Expr) -> bool {
	let mut found = false;
	e.walk(&mut |x| found |= matches!(x, Expr::Comp(_)));
	found
}

pub(super) fn is_def(e: &Expr) -> bool {
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

pub(crate) fn scalar(tag: i64, v: i64) -> Expr {
	match tag {
		TAG_INT => Expr::Int(v),
		TAG_FLOAT => Expr::Float(f64::from_bits(v as u64)),
		TAG_BOOL => Expr::Bool(v != 0),
		// SAFETY: `v` is a str handle the runtime just produced
		TAG_STR => Expr::String(unsafe { runtime::str_string(v as *const _) }),
		// SAFETY: `v` is an Ast handle, a box the quote runtime leaked
		TAG_AST => unsafe { (*(v as *mut Spanned<Expr>)).0.clone() },
		_ => Expr::Tuple(vec![]),
	}
}

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
					Entry::Scalar(TAG_AST_SEQ, n) => {
						let at = exprs.len() - n as usize;
						super::expand::one(exprs.split_off(at).into_iter().map(|e| (e, span)).collect(), span).0
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
	consts: &HashMap<String, Spanned<Expr>>,
	program: &mut Program,
	stage0: &mut Option<Compiler>,
) -> Result<Expr, Diagnostic> {
	// conditional compilation
	while let Expr::If { cond, then, els } = inner.0 {
		let span = cond.1;
		let arm = match fold(*cond, target, consts, program, stage0)? {
			Expr::Bool(true) => then,
			Expr::Bool(false) if matches!(els.as_deref(), Some([(Expr::If { .. }, _)])) => {
				inner = els.expect("guard matched").remove(0);
				continue;
			}
			Expr::Bool(false) => els.unwrap_or_default(),
			_ => return Err(Diagnostic::new("`comp if` needs a bool condition", span.into_range())),
		};
		return Ok(super::expand::one(arm, span).0);
	}
	let span = inner.1;
	// comp loop
	if let Expr::For { pat, body, .. } = &mut inner.0
		&& let Expr::Ident(var) = &pat.0
	{
		body.iter_mut().for_each(|e| unquote(e, var));
		*body = vec![(Expr::Quote(std::mem::take(body)), span)];
	}
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
	let mut synthetic = super::expand::lend(program, |module, items| {
		let has_main = items.iter().any(|it| matches!(&it.0, Expr::Fn { name, .. } if name == "main"));
		items.retain_mut(|(e, _)| match e {
			Expr::Fn { name, .. } => name != "main",
			Expr::Bind { mutable: true, .. } => has_main || module != "main",
			_ => is_def(e) && !has_comp(e),
		});
		if module == target {
			items.push(thunk.clone());
		}
	});
	(synthetic.annotations.values_mut()).for_each(|v| v.retain(|a| !matches!(a.0, Expr::Call { .. })));
	synthetic.consts = (consts.iter())
		.filter(|(_, v)| !matches!(v.0, Expr::Comp(_)))
		.map(|(k, v)| (k.clone(), v.clone()))
		.collect();
	let mut compiler = stage0.take().unwrap_or_default();
	compiler.run_stage0(&synthetic, vec![name.clone()])?;
	// SAFETY: fn takes no args and returns unit
	unsafe { std::mem::transmute::<*const u8, fn()>(compiler.finalized(&name))() };
	*stage0 = Some(compiler);
	Ok(reify(span))
}

// Fold every `comp` site, and every call in annotation position, to a literal.
pub(crate) fn eval(
	program: &mut Program,
	annotations: &mut HashMap<String, Vec<Annotation>>,
	consts: &mut HashMap<String, Spanned<Expr>>,
	stage0: &mut Option<Compiler>,
) -> Result<(), Diagnostic> {
	MAP.set(program.map.clone());
	// push order is importer-first, so folding in reverse handles dependencies first
	for i in (0..program.modules.len()).rev() {
		let module = program.modules[i].name.clone();
		let mut comps: Vec<String> = consts
			.iter()
			.filter(|(k, v)| module_of(k).unwrap_or("main") == module && matches!(v.0, Expr::Comp(_)))
			.map(|(k, _)| k.clone())
			.collect();
		// comp expressions may reference siblings, so retry until a pass gets stuck
		while !comps.is_empty() {
			let (n, mut err) = (comps.len(), None);
			comps.retain(|k| {
				let (Expr::Comp(inner), span) = consts[k].clone() else {
					unreachable!()
				};
				match fold(*inner, &module, consts, program, stage0) {
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
		while let Some(inner) = first_comp(&mut program.modules[i].items) {
			let lit = fold(inner, &module, consts, program, stage0)?;
			patch_first(&mut program.modules[i].items, lit);
		}
	}
	// an annotation call is implicitly comptime
	for a in annotations.iter_mut().filter(|(k, _)| !k.contains("::")).flat_map(|(_, v)| v) {
		if matches!(&a.0, Expr::Call { name, .. } if name != role::CTX) {
			a.0 = fold(a.clone(), "main", consts, program, stage0)?;
		}
	}
	Ok(())
}
