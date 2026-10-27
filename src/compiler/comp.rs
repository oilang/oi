// Compile-time evaluation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ast::{Annotation, BinOp, Child, Expr, Span, Spanned, TypeExpr, TypeParam};
use crate::diagnostics::{Diagnostic, SourceMap};
use crate::loader::{Program, module_of};
use crate::runtime;

use super::{Compiler, Typ, TypeCtx, role, type_expr, typeid};

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
pub(crate) const TAG_ATOM: i64 = 8;
pub(crate) const TAG_TYPE: i64 = 9;
pub(crate) const TAG_ANON: i64 = 10;

enum Entry {
	Scalar(i64, i64),
	Struct(String, usize),
}

static SITE: AtomicUsize = AtomicUsize::new(0);
static DEPTH: AtomicUsize = AtomicUsize::new(0);

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

// Visit every comp site outside generic fns and claims.
fn sites(items: &mut [Spanned<Expr>], f: &mut impl FnMut(&mut Expr)) {
	for (e, _) in items {
		match e {
			Expr::Fn { type_params, .. } | Expr::Claim { type_params, .. } if !type_params.is_empty() => {}
			Expr::Comp(_) => f(e),
			e => e.for_children(|c| match c {
				Child::List(list) => sites(list, f),
				Child::One(one) => sites(std::slice::from_mut(one), f),
			}),
		}
	}
}

// Find the first unevaluated comp site.
fn first_comp(items: &mut [Spanned<Expr>]) -> Option<Spanned<Expr>> {
	let mut found = None;
	sites(items, &mut |x| {
		if found.is_none()
			&& let Expr::Comp(inner) = x
		{
			found = Some((**inner).clone());
		}
	});
	found
}

// Swap comp site for its folded literal.
fn patch_first(items: &mut [Spanned<Expr>], lit: Expr) {
	let mut lit = Some(lit);
	sites(items, &mut |x| {
		if let Some(lit) = lit.take() {
			*x = lit;
		}
	});
}

pub(crate) fn instantiate(
	body: &mut [Spanned<Expr>],
	types: &TypeCtx,
	target: &str,
	consts: &HashMap<String, Spanned<Expr>>,
) -> Result<(), Diagnostic> {
	let (program, stage0) = types.consts.comp.expect("lowering always has a stage 0 host");
	let as_type = |(e, span): &Spanned<Expr>| match e {
		Expr::Tuple(fs) if fs.is_empty() => Some(Typ::unit()),
		e => types.resolve(&TypeExpr::from_expr(e)?, *span).ok(),
	};
	while let Some(mut inner) = first_comp(body) {
		inner.0.walk(&mut |e| {
			if let Expr::Binary(op @ (BinOp::Eq | BinOp::Ne), l, r) = e
				&& let (Some(lt), Some(rt)) = (as_type(l), as_type(r))
			{
				*e = Expr::Bool((lt == rt) == (*op == BinOp::Eq));
			} else if let Expr::Ident(n) = e
				&& let Some(Typ::Const(lit)) = types.type_params.get(n)
			{
				*e = (**lit).clone();
			} else if let Expr::Ident(n) = e
				&& let Some(TypeExpr::Name(to)) = types.type_params.get(n).and_then(type_expr)
			{
				*n = to;
			}
		});
		let lit = fold(inner, target, consts, program, &mut stage0.borrow_mut())?;
		patch_first(body, lit);
	}
	Ok(())
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
		TAG_INT => Expr::Int(v.into()),
		TAG_FLOAT => Expr::Float(f64::from_bits(v as u64)),
		TAG_BOOL => Expr::Bool(v != 0),
		// SAFETY: `v` is a str handle the runtime just produced
		TAG_STR => Expr::String(unsafe { runtime::str_string(v as *const _) }),
		// SAFETY: `v` is an interned `:name` str handle
		TAG_ATOM => Expr::Atom(unsafe { runtime::str_string(v as *const _) }[1..].into()),
		// SAFETY: `v` is an Ast handle, a box the quote runtime leaked
		TAG_AST => unsafe { (*(v as *mut Spanned<Expr>)).0.clone() },
		_ => Expr::Tuple(vec![]),
	}
}

// Spell a yielded type.
fn spell(typ: &Typ, instances: &HashMap<String, Vec<Typ>>) -> TypeExpr {
	match typ.nominal().and_then(|n| Some((n.split_once('[')?.0, instances.get(n)?))) {
		Some((base, args)) => TypeExpr::Generic(base.into(), args.iter().map(|a| spell(a, instances)).collect()),
		None if let Typ::Tuple(fs) = typ => {
			TypeExpr::Tuple(fs.iter().map(|(n, t)| (n.clone(), spell(t, instances))).collect())
		}
		None => type_expr(typ).unwrap_or_else(|| TypeExpr::Name(typ.to_string())),
	}
}

fn reify(span: Span, any_types: &[Typ], instances: &HashMap<String, Vec<Typ>>) -> Expr {
	let of = |id| any_types.iter().find(|t| typeid(t) == id).expect("a yielded type has an id");
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
					Entry::Scalar(TAG_TYPE, id) => Expr::TypePat(spell(of(id), instances)),
					Entry::Scalar(TAG_ANON, id) => {
						let Some(Expr::StructLit { name, fields, .. }) = exprs.pop() else {
							unreachable!("fields come first")
						};
						let args = fields.into_iter().map(|(_, f)| f).collect();
						let anon = match name.is_empty() {
							true => Expr::DotTuple(args),
							false => Expr::EnumShorthand { variant: name, args },
						};
						let target = (spell(of(id), instances), span);
						Expr::Cast {
							target,
							args: vec![(anon, span)],
						}
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

// Point a comptime call at a generic-equivalent copy.
fn comp_copy(call: &mut Spanned<Expr>, target: &str, program: &Program) -> Option<(String, Spanned<Expr>)> {
	let Expr::Call { name, type_args, args } = &mut call.0 else {
		return None;
	};
	let scope = &program.modules.iter().find(|m| m.name == target)?.scope;
	let key = scope.at(call.1).qualify_name(name);
	super::has_ann(&program.annotations, &key, "comp").then_some(())?;
	let module = module_of(&key).unwrap_or("main");
	let items = &program.modules.iter().find(|m| m.name == module)?.items;
	let (Expr::Fn { params, ret, body, .. }, span) = items.iter().find(|i| i.0.def_name() == Some(&key))? else {
		return None;
	};
	let type_params = (params.iter())
		.map(|p| TypeParam {
			name: p.name.clone(),
			value_type: (!matches!(&p.typ, TypeExpr::Name(t) if t == "type")).then(|| p.typ.clone()),
			..TypeParam::default()
		})
		.collect();
	*type_args = std::mem::take(args)
		.into_iter()
		.map(|(arg, span)| (TypeExpr::Value(Box::new((arg, span))), span))
		.collect();
	*name = format!("{key}#comp{}", SITE.fetch_add(1, Ordering::Relaxed));
	let copy = Expr::Fn {
		name: name.clone(),
		type_params,
		params: vec![],
		params_tuple: true,
		ret: ret.clone(),
		body: body.clone(),
	};
	Some((module.into(), (copy, *span)))
}

// Compile and run a single comp site, folding it to its reified literal.
pub(crate) fn fold(
	mut inner: Spanned<Expr>,
	target: &str,
	consts: &HashMap<String, Spanned<Expr>>,
	program: &Program,
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
	if let Expr::Bool(_) = inner.0 {
		return Ok(inner.0);
	}
	let span = inner.1;
	// comp loop
	if let Expr::For { pat, body, .. } = &mut inner.0 {
		let vars = super::expand::pat_names(pat);
		if !vars.is_empty() {
			vars.iter().for_each(|var| body.iter_mut().for_each(|e| unquote(e, var)));
			*body = vec![(Expr::Quote(std::mem::take(body)), span)];
		}
	}
	let copy = comp_copy(&mut inner, target, program);
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
		items.retain_mut(|item| match &item.0 {
			Expr::Fn { name, .. } => name != "main",
			Expr::Bind { mutable: true, .. } => has_main || module != "main",
			e => is_def(e) && first_comp(std::slice::from_mut(item)).is_none(),
		});
		if module == target {
			items.push(thunk.clone());
		}
		items.extend(copy.iter().filter(|(m, _)| m == module).map(|(_, f)| f.clone()));
	});
	(synthetic.annotations.values_mut()).for_each(|v| v.retain(|a| !matches!(a.0, Expr::Call { .. })));
	synthetic.consts = (consts.iter())
		.filter(|(_, v)| !matches!(v.0, Expr::Comp(_)))
		.map(|(k, v)| (k.clone(), v.clone()))
		.collect();
	let mut compiler = stage0.take().unwrap_or_default();
	let ran = match DEPTH.fetch_add(1, Ordering::Relaxed) {
		..8 => compiler.run_stage0(&synthetic, vec![name.clone()]),
		_ => Err(Diagnostic::new("`comp` recurses without end", span.into_range())),
	};
	DEPTH.fetch_sub(1, Ordering::Relaxed);
	ran?;
	// SAFETY: fn takes no args and returns unit
	unsafe { std::mem::transmute::<*const u8, fn()>(compiler.finalized(&name))() };
	let lit = reify(span, &compiler.out.any_types, &compiler.out.instances);
	*stage0 = Some(compiler);
	Ok(lit)
}

// Fold every `comp` site, and every call in annotation position, to a literal.
pub(crate) fn eval(
	program: &mut Program,
	annotations: &mut HashMap<String, Vec<Annotation>>,
	consts: &mut HashMap<String, Spanned<Expr>>,
	stage0: &mut Option<Compiler>,
) -> Result<(), Diagnostic> {
	MAP.set(program.map.clone());

	// consts reading from comp
	for m in program.modules.iter_mut().rev() {
		for (item, span) in &mut m.items {
			let Expr::Bind {
				mutable: false,
				typ: None,
				name,
				value: Some(v),
			} = item
			else {
				continue;
			};
			let mut reads = false;
			v.0.clone().walk(&mut |e| {
				let path = match e {
					Expr::Field { tuple, field } if let Expr::Ident(module) = &tuple.0 => format!("{module}.{field}"),
					Expr::Ident(n) => n.clone(),
					_ => return,
				};
				let key = m.scope.at(*span).qualify_name(&path);
				reads |= matches!(consts.get(&key), Some((Expr::Comp(_), _)));
			});
			if reads && !consts.contains_key(name.as_str()) {
				**v = (Expr::Comp(v.clone()), v.1);
				consts.insert(name.clone(), (**v).clone());
			}
		}
	}

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

		// const binds
		for (item, _) in &mut program.modules[i].items {
			if let Expr::Bind {
				name, value: Some(v), ..
			} = item && let (Expr::Comp(_), Some(lit)) = (&v.0, consts.get(name.as_str()))
			{
				**v = lit.clone();
			}
		}

		for j in 0..program.modules[i].items.len() {
			// comp fn
			let comp_fn = program.modules[i].items[j]
				.0
				.def_name()
				.is_some_and(|n| super::has_ann(annotations, n, "comp"));
			while !comp_fn && let Some(inner) = first_comp(std::slice::from_mut(&mut program.modules[i].items[j])) {
				let lit = fold(inner, &module, consts, program, stage0)?;
				patch_first(std::slice::from_mut(&mut program.modules[i].items[j]), lit);
			}
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
