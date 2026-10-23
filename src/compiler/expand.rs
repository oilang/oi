// User-defined comptime macros.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ast::{Capture, Child, EnumVariant, Expr, Param, Span, Spanned, TypeExpr, int_word};
use crate::diagnostics::{Diagnostic, arity_err, fail};
use crate::loader::{Program, Publics, Scope, module_of, wrapped_def};
use crate::runtime;

use super::Compiler;

use Child::{List, One};

const BUILTINS: [&str; 6] = ["dbg", "assert", "panic", "todo", "unreachable", "src"];
const MAX_DEPTH: usize = 64;
const MAX_PARAMS: usize = 4;

pub(crate) const RT_QUOTE: &str = "oi_rt_quote";
pub(crate) const RT_AST_LIT: &str = "oi_rt_ast_lit";
pub(crate) const RT_AST_METHOD: &str = "oi_rt_ast_method";
pub(crate) const RT_QUOTE_MATCH: &str = "oi_rt_quote_match";

// A copy of `program` with each module's items filtered by `keep`.
pub(super) fn lend(program: &Program, mut keep: impl FnMut(&str, &mut Vec<Spanned<Expr>>)) -> Program {
	let mut modules = program.modules.clone();
	modules.iter_mut().for_each(|m| keep(&m.name, &mut m.items));
	Program {
		map: program.map.clone(),
		modules,
		publics: program.publics.clone(),
		reexports: program.reexports.clone(),
		consts: program.consts.clone(),
		const_types: program.const_types.clone(),
		annotations: program.annotations.clone(),
		roots: program.roots.clone(),
		core_origin: program.core_origin.clone(),
		selected: vec![],
	}
}

#[derive(Default)]
struct Expander {
	// stage-0 fns, in definition order
	defs: Vec<Spanned<Expr>>,
	// name -> (arity, stage-0 fn pointer)
	macros: HashMap<String, (usize, *const u8)>,
	// qualified names visible outside their own module
	publics: Publics,
	// keeps the stage-0 JIT and its code alive for every call this pass makes
	stage0: Option<Compiler>,
	hoisted: Vec<Spanned<Expr>>,
}

// Every name a pattern binds.
pub(super) fn pat_names(pat: &mut Spanned<Expr>) -> Vec<&mut String> {
	let elems: Vec<&mut Spanned<Expr>> = match &mut pat.0 {
		Expr::Ident(n) => return vec![n],
		Expr::Tuple(fields) | Expr::StructLit { fields, .. } => fields.iter_mut().map(|(_, e)| e).collect(),
		Expr::Array(elems) => elems.iter_mut().collect(),
		_ => return vec![],
	};
	elems
		.into_iter()
		.filter_map(|(e, _)| if let Expr::Ident(n) = e { Some(n) } else { None })
		.collect()
}

// Visit every name-binding site in expr, skipping deliberate `%name` captures.
fn for_binders(e: &mut Expr, f: &mut impl FnMut(&mut String)) {
	match e {
		Expr::Bind { name, .. } if !name.starts_with('%') => f(name),
		Expr::PatBind {
			pat, mutable: Some(_), ..
		}
		| Expr::For { pat, .. } => pat_names(pat).into_iter().filter(|n| !n.starts_with('%')).for_each(f),
		Expr::AnonFn { params, .. } => params
			.iter_mut()
			.filter(|p| !p.name.is_empty() && !p.name.starts_with('%'))
			.for_each(|p| f(&mut p.name)),
		Expr::Match { arms, .. } => arms.iter_mut().flat_map(|a| &mut a.binding).for_each(f),
		_ => {}
	}
}

// The name a `%name` can take.
fn name_slot(e: &mut Expr) -> Option<&mut String> {
	match e {
		Expr::Field { field, .. } | Expr::IndexAssign { field: Some(field), .. } => Some(field),
		Expr::MethodCall { method, .. } => Some(method),
		Expr::Ident(name)
		| Expr::Bind { name, .. }
		| Expr::Assign { name, .. }
		| Expr::Fn { name, .. }
		| Expr::StructDef { name, .. }
		| Expr::EnumDef { name, .. }
		| Expr::TypeAlias { name, .. }
		| Expr::StructLit { name, .. }
		| Expr::Claim { typ: name, .. } => Some(name),
		Expr::For { pat, .. } => match &mut pat.0 {
			Expr::Ident(name) => Some(name),
			_ => None,
		},
		_ => None,
	}
}

fn resolve_bare(name: &str, scope: &Scope) -> String {
	let key = format!("{name}!");
	scope.env.get(&key).cloned().unwrap_or(key)
}

// Get a fn's defining module, encoded in its qualified name.
fn owner(e: &Spanned<Expr>) -> &str {
	match &e.0 {
		Expr::Fn { name, .. } => module_of(name).unwrap_or("main"),
		_ => "main",
	}
}

// Rewrite direct macro calls in stage-0 bodies to their compiled fns, quoted ones expand once spliced.
fn direct_calls(e: &mut Expr, macros: &HashMap<String, (usize, *const u8)>, scope: &Scope, in_quote: bool) {
	if !in_quote && let Expr::MacroCall { name, args } = e {
		let resolved = resolve_bare(name, scope);
		if macros.contains_key(&resolved) {
			let args = std::mem::take(args);
			*e = Expr::Call {
				name: resolved,
				type_args: vec![],
				args,
			};
		}
	}
	if in_quote {
		def_site(e, scope);
	}
	let child_in_quote = match e {
		Expr::Quote(_) => true,
		Expr::UnquoteExpr(_) | Expr::UnquoteSplat(_) => false,
		_ => in_quote,
	};
	e.for_children(|c| match c {
		List(list) => list
			.iter_mut()
			.for_each(|(e, _)| direct_calls(e, macros, scope, child_in_quote)),
		One((e, _)) => direct_calls(e, macros, scope, child_in_quote),
	});
}

// Resolve a template's `mod.fn` calls where the macro is actually defined.
fn def_site(e: &mut Expr, scope: &Scope) {
	let Expr::MethodCall {
		recv,
		method,
		type_args,
		args,
	} = e
	else {
		return;
	};
	let Expr::Ident(m) = &recv.0 else { return };
	let name = match scope.visible.contains_key(m) {
		true => scope.qualify_name(&format!("{m}.{method}")),
		false if scope.module.rsplit('.').next() == Some(m) => scope.qualify_name(method),
		false => return,
	};
	*e = Expr::Call {
		name,
		type_args: std::mem::take(type_args),
		args: std::mem::take(args),
	};
}

impl Expander {
	fn define(
		&mut self,
		name: String,
		mut params: Vec<Param>,
		ret: Option<Spanned<TypeExpr>>,
		body: Vec<Spanned<Expr>>,
		span: Span,
	) -> Result<(), Diagnostic> {
		let ast = |te: &TypeExpr| matches!(te, TypeExpr::Name(n) if n == "Ast");
		let param = |te: &TypeExpr| ast(te) || matches!(te, TypeExpr::Name(n) if n == "Tokens");
		let bare = name.rsplit("::").next().unwrap_or(&name);
		if BUILTINS.contains(&bare.trim_end_matches('!')) {
			return fail(format!("`{name}` is a builtin macro"), span, "reserved name");
		}
		if self.macros.contains_key(&name) {
			return fail(format!("`{name}` is already defined"), span, "duplicate macro");
		}
		if let Some(p) = params.iter().find(|p| !param(&p.typ)) {
			return fail("macro params must be `Ast` or `Tokens`", p.span, "not Ast");
		}
		params.iter_mut().for_each(|p| p.typ = TypeExpr::Name("Ast".into()));
		if params.len() > MAX_PARAMS {
			return fail("macros take at most 4 arguments for now", span, "too many parameters");
		}
		if let Some((te, rspan)) = &ret
			&& !ast(te)
		{
			return fail("macros return `Ast`", *rspan, "not Ast");
		}
		self.macros.insert(name.clone(), (params.len(), std::ptr::null()));
		let f = Expr::Fn {
			name,
			type_params: vec![],
			params_tuple: params.len() != 1,
			params,
			ret: Some((TypeExpr::Name("Ast".into()), Span::default())),
			body,
		};
		self.defs.push((f, span));
		Ok(())
	}

	// Compile every macro body as a real fn, in one synthetic program, and grab their pointers.
	fn compile_stage0(&mut self, program: &Program) -> Result<(), Diagnostic> {
		let scopes: HashMap<&str, &Scope> = program.modules.iter().map(|m| (m.name.as_str(), &m.scope)).collect();
		for e in &mut self.defs {
			let scope = scopes[owner(e)].at(e.1);
			direct_calls(&mut e.0, &self.macros, scope, false);
		}
		let defs = std::mem::take(&mut self.defs);
		let mut synthetic = lend(program, |module, items| {
			let scope = &scopes[module];
			items.retain_mut(|(e, span)| {
				let lent = match e {
					_ if module != "main" => true,
					Expr::Fn { name, .. } => name != "main",
					Expr::Bind { .. } => false,
					_ => super::comp::is_def(e),
				};
				lent && !self.calls_macro(e, scope.at(*span))
			});
			items.extend(defs.iter().filter(|d| owner(d) == module).cloned());
		});
		synthetic.annotations.retain(|k, _| k.contains("::"));
		let compiler = self.stage0.get_or_insert_with(Compiler::default);
		compiler.run_stage0(&synthetic, self.macros.keys().cloned().collect())?;
		for (name, (_, ptr)) in &mut self.macros {
			*ptr = compiler.finalized(name);
		}
		Ok(())
	}

	// Checks whether item calls a user macro.
	fn calls_macro(&self, e: &mut Expr, scope: &Scope) -> bool {
		let mut found = false;
		e.walk(&mut |x| {
			if let Expr::MacroCall { name, .. } = x {
				let key = self.resolve(name, scope, Span::default()).unwrap_or_default();
				found |= self.macros.contains_key(&key);
			}
			x.types()
				.into_iter()
				.for_each(|t| t.walk_mut(&mut |t| found |= self.type_call(t, scope).is_some()));
		});
		found
	}

	// A `name!(T)` call.
	fn type_call(&self, t: &TypeExpr, scope: &Scope) -> Option<Spanned<Expr>> {
		if let TypeExpr::Annotated(anns, inner) = t
			&& let [(Expr::Ident(name), span)] = &anns[..]
			&& self.macros.contains_key(&resolve_bare(name, scope))
		{
			let (name, args) = (name.clone(), vec![(type_ast(inner), *span)]);
			return Some((Expr::MacroCall { name, args }, *span));
		}
		None
	}

	// Swap each `@name T` type for the one `name!(T)` defines, hoisting its definitions.
	fn expand_types(&mut self, e: &mut Expr, scope: &Scope, depth: usize) -> Result<(), Diagnostic> {
		let mut res = Ok(());
		for t in e.types() {
			t.walk_mut(&mut |t| {
				if res.is_ok()
					&& let Some(call) = self.type_call(t, scope)
				{
					res = self.call(&call, scope, depth).map(|defs| {
						let mut defs = defs.unwrap_or_default();
						defs.iter_mut()
							.filter_map(|d| name_slot(&mut d.0))
							.for_each(|n| *n = scope.qualify_name(n));
						let name = defs.first().and_then(|d| d.0.def_name()).unwrap_or_default().to_string();
						if !self.hoisted.iter().any(|h| h.0.def_name() == Some(&name)) {
							self.hoisted.extend(defs);
						}
						*t = TypeExpr::Name(name);
					});
				}
			});
		}
		res
	}

	// Resolve a macro call against scope, enforcing privacy.
	fn resolve(&self, name: &str, scope: &Scope, span: Span) -> Result<String, Diagnostic> {
		let Some((module, rest)) = name.split_once('.') else {
			return Ok(resolve_bare(name, scope));
		};
		let Some(vis) = scope.visible.get(module) else {
			return fail(format!("cannot find module `{module}`"), span, "no such module");
		};
		let key = format!("{}::{rest}!", vis.module);
		if !self.publics.is_visible(&key, &scope.module) {
			return fail(format!("`{rest}` is private to module `{module}`"), span, "not public");
		}
		Ok(key)
	}

	// Instantiate expr if it calls a user macro.
	fn call(
		&mut self,
		e: &Spanned<Expr>,
		scope: &Scope,
		depth: usize,
	) -> Result<Option<Vec<Spanned<Expr>>>, Diagnostic> {
		let Expr::MacroCall { name, args } = &e.0 else {
			return Ok(None);
		};
		let key = self.resolve(name, scope, e.1)?;
		let Some(&(arity, ptr)) = self.macros.get(&key) else {
			return Ok(None);
		};
		if args.len() != arity {
			return arity_err(&format!("`{name}!`"), arity, args.len(), "argument", e.1);
		}
		if depth >= MAX_DEPTH {
			return fail("macro expansion is too deep", e.1, "recursion limit");
		}
		type Ptr = *mut Spanned<Expr>;
		let boxed: Vec<Ptr> = args
			.iter()
			.map(|a| match &a.0 {
				Expr::Quote(q) => one(q.clone(), a.1),
				_ => a.clone(),
			})
			.map(leak)
			.collect();
		let arg = |i: usize| boxed.get(i).copied().unwrap_or(std::ptr::null_mut());
		// SAFETY: stage-0 fns take at most MAX_PARAMS pointer args, all in registers on the ABIs cranelift targets, so a fixed-shape call just leaves the extras unread.
		let f = unsafe { std::mem::transmute::<*const u8, fn(Ptr, Ptr, Ptr, Ptr) -> Ptr>(ptr) };
		DEFS.with_borrow_mut(|d| d.0 = scope.clone());
		let (e0, span) = unsafe { *Box::from_raw(f(arg(0), arg(1), arg(2), arg(3))) };
		if let Some(msg) = ERROR.take() {
			return fail(msg, e.1, format!("while running `{name}!`"));
		}
		let mut body = match e0 {
			Expr::Block(stmts) => stmts,
			other => vec![(other, span)],
		};
		body.iter_mut().for_each(|s| pin(s, e.1));
		self.expand(List(&mut body), scope, depth + 1)?;
		Ok(Some(body))
	}

	// Walk the tree, expanding macro calls.
	fn expand(&mut self, c: Child, scope: &Scope, depth: usize) -> Result<(), Diagnostic> {
		match c {
			List(body) => {
				let mut i = 0;
				while i < body.len() {
					let (file, span) = (scope.at(body[i].1), body[i].1);
					match self.call(&body[i], file, depth)? {
						Some(mut stmts) => {
							if !scope.files.is_empty() {
								stmts.iter_mut().for_each(|s| s.1 = span);
							}
							let n = stmts.len();
							body.splice(i..i + 1, stmts);
							i += n;
						}
						None => {
							self.expand(One(&mut body[i]), file, depth)?;
							i += 1;
						}
					}
				}
				Ok(())
			}
			One(e) => {
				if let Some(stmts) = self.call(e, scope, depth)? {
					e.0 = one(stmts, e.1).0;
					return Ok(());
				}
				self.expand_types(&mut e.0, scope, depth)?;
				match &e.0 {
					Expr::Fn { name, .. } if name.contains('!') => Ok(()),
					Expr::Quote(_) => Ok(()),
					Expr::Unquote(_) | Expr::UnquoteExpr(_) | Expr::UnquoteSplat(_) | Expr::UnquoteBind(..) => {
						fail("unquotes only make sense inside a macro template", e.1, "stray unquote")
					}
					Expr::MacroDef { .. } => {
						fail("macros can only be defined at the top level", e.1, "nested macro def")
					}
					_ => e.0.try_children(|c| self.expand(c, scope, depth)),
				}
			}
		}
	}
}

// Expand all macro calls across a program's modules, keeping the stage-0 JIT that ran them.
pub fn expand(program: &Program) -> Result<(Program, Option<Compiler>), Diagnostic> {
	let mut ex = Expander {
		publics: program.publics.clone(),
		..Default::default()
	};
	let mut out = lend(program, |_, _| {});
	for m in &mut out.modules {
		for item in std::mem::take(&mut m.items) {
			match item.0 {
				Expr::MacroDef {
					name,
					params,
					ret,
					body,
				} => ex.define(name, params, ret, body, item.1)?,
				_ => m.items.push(item),
			}
		}
	}
	let called =
		(out.modules.iter_mut()).any(|m| m.items.iter_mut().any(|it| ex.calls_macro(&mut it.0, m.scope.at(it.1))));
	if called {
		ex.compile_stage0(&out)?;
	}
	let defs = program.items().map(|(_, i)| Expr::peel_meta(i).2);
	DEFS.set((
		Scope::default(),
		defs.filter_map(|d| Some((d.0.def_name()?.to_string(), d.clone()))).collect(),
	));
	for m in &mut out.modules {
		for item in std::mem::take(&mut m.items) {
			let call = m.name != "main" && matches!(item.0, Expr::MacroCall { .. });
			let mut expanded = vec![item];
			ex.expand(List(&mut expanded), &m.scope, 0)?;
			// add module prefixes
			let defs = expanded.iter_mut().filter(|_| call).filter_map(|it| wrapped_def(&mut it.0));
			for (public, name) in defs.filter(|(_, name)| module_of(name).is_none()) {
				*name = format!("{}::{name}", m.name);
				if let Some(vis) = public {
					ex.publics.vis.insert(name.clone(), vis);
				}
			}
			m.items.append(&mut expanded);
		}
		m.items.append(&mut ex.hoisted);
	}
	out.publics = ex.publics;
	out.check_selected(&program.selected, |q| ex.macros.contains_key(q))?;
	Ok((out, ex.stage0))
}

// A quote template.
struct Template {
	stmts: Vec<Spanned<Expr>>,
	names: Vec<String>,
	bound: HashSet<String>,
}

static HYGIENE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
	static ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
	static DEFS: RefCell<(Scope, HashMap<String, Spanned<Expr>>)> = RefCell::default();
}

// Record a macro-run failure, carrying on with `stand_in`.
fn flag<T>(msg: &str, stand_in: T) -> T {
	ERROR.with_borrow_mut(|e| {
		e.get_or_insert_with(|| msg.to_string());
	});
	stand_in
}

// One unquote site in a template.
pub(crate) enum Slot {
	Name(String),
	Expr(Spanned<Expr>),
	Splat(Spanned<Expr>),
}

// An instantiation argument.
// One Ast, or a splat's `[]Ast` elements.
enum Arg<'a> {
	Ast(&'a Spanned<Expr>),
	Seq(Vec<Spanned<Expr>>),
}

fn push_name(slots: &mut Vec<Slot>, n: &str) {
	if !slots.iter().any(|s| matches!(s, Slot::Name(m) if m == n)) {
		slots.push(Slot::Name(n.to_string()));
	}
}

// Collect a template's unquote slots and binders, pulling unquote expressions out into slots.
fn scan(e: &mut Expr, slots: &mut Vec<Slot>, bound: &mut HashSet<String>, nested: &mut bool) {
	match e {
		Expr::Quote(_) => *nested = true,
		Expr::Unquote(n) => push_name(slots, n),
		Expr::UnquoteExpr(inner) | Expr::UnquoteSplat(inner) => {
			let taken = std::mem::replace(inner.as_mut(), (Expr::Unquote(String::new()), Span::default()));
			let (key, slot) = match e {
				Expr::UnquoteSplat(_) => (format!("...{}", slots.len()), Slot::Splat(taken)),
				_ => (slots.len().to_string(), Slot::Expr(taken)),
			};
			slots.push(slot);
			*e = Expr::Unquote(key);
		}
		Expr::UnquoteBind(binder, bind) => {
			let placeholder = (Expr::Unquote(String::new()), Span::default());
			if let Some(name) = name_slot(&mut bind.0) {
				*name = format!("%{}", slots.len());
			}
			slots.push(Slot::Expr(std::mem::replace(binder.as_mut(), placeholder)));
			*e = std::mem::replace(&mut bind.0, Expr::Unquote(String::new()));
		}
		_ => {
			if let Some(n) = name_slot(e).and_then(|n| n.strip_prefix('%')) {
				push_name(slots, n);
			}
			if let Expr::Fn { params, .. } | Expr::AnonFn { params, .. } = e {
				params
					.iter()
					.filter_map(|p| p.name.strip_prefix('%'))
					.for_each(|n| push_name(slots, n));
			}
		}
	}
	for_binders(e, &mut |n| {
		bound.insert(n.clone());
	});
	e.for_children(|c| match c {
		List(list) => list.iter_mut().for_each(|(e, _)| scan(e, slots, bound, nested)),
		One((e, _)) => scan(e, slots, bound, nested),
	});
}

// Validate a quote and leak it as a template.
pub(crate) fn register(stmts: &[Spanned<Expr>], span: Span) -> Result<(usize, Vec<Slot>), Diagnostic> {
	let mut stmts = stmts.to_vec();
	let (mut slots, mut bound, mut nested) = (Vec::new(), HashSet::new(), false);
	for (e, _) in &mut stmts {
		scan(e, &mut slots, &mut bound, &mut nested);
	}
	if nested {
		return fail("nested quotes aren't supported yet", span, "nested quote");
	}
	let names = slots
		.iter()
		.enumerate()
		.map(|(i, s)| match s {
			Slot::Name(n) => n.clone(),
			Slot::Expr(_) => i.to_string(),
			Slot::Splat(_) => format!("...{i}"),
		})
		.collect();
	let tpl = Template { names, stmts, bound };
	Ok((leak(tpl) as usize, slots))
}

// Fill a template copy, suffixing its own binders for good hygiene (see what I did there?).
fn fill(e: &mut Spanned<Expr>, bound: &HashSet<String>, args: &HashMap<&str, Arg>, suffix: usize) {
	if let Expr::Unquote(name) = &e.0 {
		match &args[name.as_str()] {
			Arg::Ast(v) => *e = (*v).clone(),
			Arg::Seq(_) => e.0 = flag("%{..} spread needs a sequence position", Expr::Tuple(vec![])),
		}
		return;
	}
	let rename = &mut |n: &mut String| {
		if bound.contains(n.as_str()) {
			*n = format!("{n}#{suffix}");
		}
	};
	for_binders(&mut e.0, rename);
	match &mut e.0 {
		Expr::Ident(n)
		| Expr::Assign { name: n, .. }
		| Expr::Call { name: n, .. }
		| Expr::FieldAssign { name: n, .. }
		| Expr::DerefAssign { name: n, .. }
		| Expr::IndexAssign { name: n, .. }
		| Expr::Append { name: n, .. }
		| Expr::MapDelete { name: n, .. } => rename(n),
		Expr::PatBind { pat, mutable: None, .. } => pat_names(pat).into_iter().for_each(rename),
		Expr::AnonFn {
			captures: Some(list), ..
		} => {
			for c in list {
				let (Capture::ReadOnly(n) | Capture::Mut(n) | Capture::Move(n)) = c;
				rename(n);
			}
		}
		_ => {}
	}
	if let Some(name) = name_slot(&mut e.0)
		&& let Some(param) = name.strip_prefix('%')
	{
		match &args[param] {
			Arg::Ast((Expr::Ident(n), _)) => *name = n.clone(),
			_ => flag("a `%name` binder needs a plain name argument", ()),
		}
	}
	match &mut e.0 {
		Expr::Fn {
			params,
			params_tuple,
			ret,
			..
		}
		| Expr::AnonFn {
			params,
			params_tuple,
			ret,
			..
		} => {
			fill_sig(params, ret.as_mut(), args);
			*params_tuple |= params.len() != 1;
		}
		Expr::StructDef { fields, .. } => fill_sig(fields, None, args),
		_ => e.0.types().into_iter().for_each(|t| fill_type(t, args)),
	}
	match &mut e.0 {
		Expr::Call { args: list, .. }
		| Expr::MacroCall { args: list, .. }
		| Expr::EnumShorthand { args: list, .. }
		| Expr::Array(list)
		| Expr::DotArray(_, list)
		| Expr::DotTuple(list) => splice(list, bound, args, suffix),
		Expr::MethodCall { recv, args: list, .. } => {
			fill(recv, bound, args, suffix);
			splice(list, bound, args, suffix);
		}
		Expr::StructLit { fields, .. } if fields.iter().all(|(n, _)| n.is_none()) => {
			let mut list = fields.drain(..).map(|(_, v)| v).collect();
			splice(&mut list, bound, args, suffix);
			*fields = list.into_iter().map(|v| (None, v)).collect();
		}
		_ => e.0.for_children(|c| match c {
			List(list) => splice(list, bound, args, suffix),
			One(one) => fill(one, bound, args, suffix),
		}),
	}
	if let Expr::Match { arms, .. } = &mut e.0 {
		*arms = std::mem::take(arms)
			.into_iter()
			.flat_map(|a| match a.patterns.is_empty() {
				false => vec![a],
				true => (a.body.into_iter())
					.filter_map(|(e, _)| match e {
						Expr::Arm(arm) => Some(arm),
						_ => flag("a match arm spread needs `pattern => body` Asts", None),
					})
					.collect(),
			})
			.collect();
	}
	if let Expr::EnumDef { variants, fills, .. } = &mut e.0 {
		let spliced = fills.extract_if(.., |f| matches!(f.0, Expr::Ident(_) | Expr::Call { .. }));
		variants.extend(spliced.map(to_variant));
	}
}

// A `name: Type` param Ast.
fn to_param(hole: &Param, a: &Spanned<Expr>) -> Param {
	let mut p = hole.clone();
	let (notes, _, (e, _)) = Expr::peel_meta(a);
	if let Expr::Bind {
		name,
		typ: Some(t),
		value,
		..
	} = e
	{
		p.name = name.split('#').next().unwrap_or(name).into();
		(p.typ, p.default) = (t.0.clone(), value.as_deref().cloned());
		p.annotations.extend_from_slice(notes);
	} else {
		flag("a param hole needs a `name: Type` Ast", ());
	}
	p
}

// The slot a type hole names.
fn hole_key(t: &TypeExpr) -> Option<&str> {
	match t {
		TypeExpr::Unquote(e) if let Expr::Unquote(k) = &e.0 => Some(k),
		_ => None,
	}
}

// Fill every type hole with the type named by its argument.
fn fill_type(t: &mut TypeExpr, args: &HashMap<&str, Arg>) {
	t.walk_mut(&mut |t| {
		let Some(k) = hole_key(t) else { return };
		match &args[k] {
			Arg::Ast(a) if let Some(named) = TypeExpr::from_expr(&a.0) => *t = named,
			_ => flag("a type hole needs an Ast naming a type", ()),
		}
	});
}

// Fill a signature.
fn fill_sig(params: &mut Vec<Param>, ret: Option<&mut Spanned<TypeExpr>>, args: &HashMap<&str, Arg>) {
	let mut out = Vec::with_capacity(params.len());
	for mut p in params.drain(..) {
		match hole_key(&p.typ).filter(|_| p.name.is_empty()) {
			Some(k) => match &args[k] {
				Arg::Ast(a) => out.push(to_param(&p, a)),
				Arg::Seq(v) => out.extend(v.iter().map(|a| to_param(&p, a))),
			},
			None => {
				if let Some(k) = p.name.strip_prefix('%') {
					match &args[k] {
						Arg::Ast((Expr::Ident(n), _)) => p.name = n.clone(),
						_ => flag("a param name hole needs a plain name argument", ()),
					}
				}
				fill_type(&mut p.typ, args);
				out.push(p);
			}
		}
	}
	*params = out;
	ret.into_iter().for_each(|(t, _)| fill_type(t, args));
}

// A `Name` or `Name(Type)` Ast spliced into an enum body.
fn to_variant((e, span): Spanned<Expr>) -> EnumVariant {
	let (name, args) = match e {
		Expr::Call { name, args, .. } => (name, args),
		Expr::Ident(name) => (name, vec![]),
		_ => unreachable!(),
	};
	let payload: Vec<_> = args.iter().filter_map(|a| Some((TypeExpr::from_expr(&a.0)?, a.1))).collect();
	if payload.len() != args.len() {
		flag("an enum variant payload needs Asts naming types", ());
	}
	EnumVariant {
		name,
		span,
		payload,
		..Default::default()
	}
}

// Walk a sequence position, splicing `%{..expr}` slots in verbatim and filling everything else.
fn splice(list: &mut Vec<Spanned<Expr>>, bound: &HashSet<String>, args: &HashMap<&str, Arg>, suffix: usize) {
	let mut i = 0;
	while i < list.len() {
		if let Expr::Unquote(n) = &list[i].0
			&& let Arg::Seq(items) = &args[n.as_str()]
		{
			let items = items.clone();
			let n = items.len();
			list.splice(i..i + 1, items);
			i += n;
		} else {
			fill(&mut list[i], bound, args, suffix);
			i += 1;
		}
	}
}

// Instantiate the template at `tpl` with borrowed `args`, by position against its unquote names.
pub(crate) extern "C" fn rt_quote(tpl: usize, args: *const *mut Spanned<Expr>, len: usize) -> *mut Spanned<Expr> {
	// SAFETY: `tpl` was leaked by `register`.
	let tpl = unsafe { &*(tpl as *const Template) };
	let args = if len == 0 {
		&[]
	} else {
		unsafe { std::slice::from_raw_parts(args, len) }
	};
	let suffix = HYGIENE.fetch_add(1, Ordering::Relaxed) + 1;
	let map: HashMap<&str, Arg> = tpl
		.names
		.iter()
		.zip(args)
		.map(|(n, &p)| {
			let arg = if n.starts_with("...") {
				// SAFETY: the lowerer passes a `[]Ast` header for splat slots
				let elems = unsafe { runtime::array_elems(p.cast()) };
				// flatten multi-stmt quotes, which arrive as a block
				Arg::Seq(
					elems
						.iter()
						.flat_map(|&q| match unsafe { (*(q as *mut Spanned<Expr>)).clone() } {
							(Expr::Block(v), _) => v,
							e => vec![e],
						})
						.collect(),
				)
			} else {
				Arg::Ast(unsafe { &*p })
			};
			(n.as_str(), arg)
		})
		.collect();
	let mut stmts = tpl.stmts.clone();
	splice(&mut stmts, &tpl.bound, &map, suffix);
	let span = stmts.first().map_or(Span::default(), |s| s.1);
	leak(one(stmts, span))
}

// Move template fn literals to the call site.
fn pin(e: &mut Spanned<Expr>, call: Span) {
	if matches!(e.0, Expr::AnonFn { .. }) && (e.1.start < call.start || e.1.end > call.end) {
		e.1 = call;
	}
	e.0.for_children(|c| match c {
		List(list) => list.iter_mut().for_each(|e| pin(e, call)),
		One(e) => pin(e, call),
	});
}

// The unit Ast handed back by a failed macro op.
fn empty() -> Spanned<Expr> {
	(Expr::Tuple(vec![]), Span::default())
}

// Box a value for compiled code to hold.
fn leak<T>(v: T) -> *mut T {
	Box::into_raw(Box::new(v))
}

// Wrap loose stmts into a block when more than one.
pub(crate) fn one(mut stmts: Vec<Spanned<Expr>>, span: Span) -> Spanned<Expr> {
	match stmts.len() {
		1 => stmts.pop().unwrap(),
		_ => (Expr::Block(stmts), span),
	}
}

// Match a template's pattern against `subject`, leaking each `%name` capture into `outs`.
pub(crate) extern "C" fn rt_quote_match(tpl: usize, subject: *mut Spanned<Expr>, outs: *mut *mut Spanned<Expr>) -> i64 {
	// SAFETY: `tpl` was leaked by `register`, `subject` by the caller's Ast value.
	let (tpl, subj) = unsafe { (&*(tpl as *const Template), &*subject) };
	let mut binds = Vec::new();
	if !unify(&tpl.stmts[0], subj, &mut binds) {
		return 0;
	}
	for (i, b) in binds.into_iter().enumerate() {
		unsafe { *outs.add(i) = leak(b) };
	}
	1
}

// Take `e`'s child subtrees, leaving a placeholder.
fn split(e: &Expr) -> (String, Vec<Spanned<Expr>>) {
	let (mut e, mut kids) = (e.clone(), Vec::new());
	e.for_children(|c| {
		let list = match c {
			List(list) => list.as_mut_slice(),
			One(one) => std::slice::from_mut(one),
		};
		for one in list {
			kids.push(std::mem::replace(one, empty()));
		}
	});
	(format!("{e:?}"), kids)
}

// Structurally unify a quote pattern against a runtime Ast, capturing `%name` holes.
fn unify(pat: &Spanned<Expr>, subj: &Spanned<Expr>, binds: &mut Vec<Spanned<Expr>>) -> bool {
	if matches!(pat.0, Expr::Unquote(_)) {
		binds.push(subj.clone());
		return true;
	}
	let (mut pat_expr, mut subj_expr) = (pat.0.clone(), subj.0.clone());
	if let Some(hole) = name_slot(&mut pat_expr).filter(|n| n.starts_with('%'))
		&& let Some(name) = name_slot(&mut subj_expr)
	{
		binds.push((Expr::Ident(std::mem::take(name)), subj.1));
		hole.clear();
	}
	let ((ph, pk), (sh, sk)) = (split(&pat_expr), split(&subj_expr));
	ph == sh && pk.iter().zip(&sk).all(|(p, s)| unify(p, s, binds))
}

pub(crate) extern "C" fn rt_ast_lit(tag: i64, bits: i64) -> *mut Spanned<Expr> {
	leak((super::comp::scalar(tag, bits), Span::default()))
}

#[unsafe(export_name = "oi_ast_ident")]
pub(crate) extern "C" fn rt_ast_ident(s: *const runtime::StrHeader) -> *mut Spanned<Expr> {
	let name = unsafe { runtime::str_string(s) };
	leak((Expr::Ident(name), Span::default()))
}
#[unsafe(export_name = "oi_ast_gensym")]
pub(crate) extern "C" fn rt_ast_gensym(s: *const runtime::StrHeader) -> *mut Spanned<Expr> {
	let prefix = unsafe { runtime::str_string(s) };
	let n = HYGIENE.fetch_add(1, Ordering::Relaxed) + 1;
	leak((Expr::Ident(format!("{prefix}#{n}")), Span::default()))
}
#[unsafe(export_name = "oi_ast_parse")]
pub(crate) extern "C" fn rt_ast_parse(s: *const runtime::StrHeader) -> *mut Spanned<Expr> {
	let src = unsafe { runtime::str_string(s) };
	let node = match crate::loader::parse_file("", &src, 0, &HashSet::new()) {
		Ok(stmts) => one(stmts, Span::default()),
		Err(ds) => flag(ds.first().map_or("parse failed", Diagnostic::message), empty()),
	};
	leak(node)
}
#[unsafe(export_name = "oi_ast_def")]
pub(crate) extern "C" fn rt_ast_def(a: *mut Spanned<Expr>) -> *mut Spanned<Expr> {
	let name = match unsafe { &(*a).0 } {
		Expr::Ident(n) => n.clone(),
		Expr::Field { tuple, field } if let Expr::Ident(m) = &tuple.0 => format!("{m}.{field}"),
		_ => String::new(),
	};
	let def = DEFS.with_borrow(|(scope, defs)| defs.get(&scope.qualify_name(&name)).cloned());
	leak(def.unwrap_or_else(|| flag("`def` needs the name of a definition", empty())))
}

#[unsafe(export_name = "oi_ast_int")]
pub(crate) extern "C" fn rt_ast_int(a: *mut Spanned<Expr>) -> i64 {
	match unsafe { &(*a).0 } {
		Expr::Int(n) if let Some(n) = int_word(*n) => n,
		_ => flag("`.int()` needs an Ast holding an Int literal", 0),
	}
}

// A type as an Ast.
fn type_ast(t: &TypeExpr) -> Expr {
	match t {
		TypeExpr::Name(n) => Expr::Ident(n.clone()),
		t => Expr::TypePat(t.clone()),
	}
}

// A field as the Ast a param hole takes.
fn field_ast(f: &Param) -> Expr {
	let bind = Expr::Bind {
		mutable: false,
		name: f.name.clone(),
		typ: Some((f.typ.clone(), f.span)),
		value: f.default.clone().map(Box::new),
	};
	match f.annotations.is_empty() {
		true => bind,
		false => Expr::Annotated(f.annotations.clone(), Box::new((bind, f.span))),
	}
}

// Ast dispatch for the lowerer.
pub(crate) extern "C" fn rt_ast_method(a: *mut Spanned<Expr>, m: *const runtime::StrHeader, arg: i64) -> i64 {
	let ast = |e: Expr| leak((e, Span::default())) as i64;
	let list = |ptrs: Vec<i64>| runtime::array_of(&ptrs, 8) as i64;
	let asts = |es: &mut dyn Iterator<Item = &Spanned<Expr>>| list(es.map(|e| leak(e.clone()) as i64).collect());
	let m = unsafe { runtime::str_bytes(m) };
	let (notes, _, (subject, span)) = Expr::peel_meta(unsafe { &*a });
	match (m, subject) {
		(b"notes", _) => asts(&mut notes.iter()),
		(b"typ", Expr::Bind { typ: Some((t, _)), .. } | Expr::Fn { ret: Some((t, _)), .. }) => ast(type_ast(t)),
		(b"typ", Expr::TypePat(TypeExpr::Array(t) | TypeExpr::FixedArray(t, _))) => ast(type_ast(t)),
		(b"typ", Expr::Fn { ret: None, .. }) => ast(Expr::TypePat(TypeExpr::Tuple(vec![]))),
		(b"typ", _) => flag("this Ast has no type", leak(empty()) as i64),
		(b"len", Expr::TypePat(TypeExpr::FixedArray(_, n))) => ast(n.0.clone()),
		// the variant name, lowercased
		(b"kind", e) => {
			let name = match e {
				Expr::TypePat(t) => format!("{t:?}"),
				e => format!("{e:?}"),
			};
			ast(Expr::Ident(name.split(['(', ' ']).next().unwrap().to_lowercase()))
		}
		(b"str", e) => {
			let s = match e {
				Expr::Ident(s) | Expr::String(s) => s.clone(),
				Expr::Int(n) => n.to_string(),
				Expr::Float(n) => n.to_string(),
				Expr::Bool(b) => b.to_string(),
				_ => flag("`.str()` needs an Ast holding a name or literal", String::new()),
			};
			runtime::str_new(runtime::system_allocator(), s.as_bytes()) as i64
		}
		(
			b"name",
			Expr::StructDef { name, .. }
			| Expr::EnumDef { name, .. }
			| Expr::Call { name, .. }
			| Expr::MacroCall { name, .. }
			| Expr::Bind { name, .. }
			| Expr::Fn { name, .. }
			| Expr::Claim { typ: name, .. }
			| Expr::StructLit { name, .. },
		) => ast(Expr::Ident(name.clone())),
		(b"name", ident @ Expr::Ident(_)) => ast(ident.clone()),
		(b"name", _) => flag("this Ast has no name", leak(empty()) as i64),
		(
			b"items",
			Expr::Array(v)
			| Expr::DotArray(_, v)
			| Expr::Block(v)
			| Expr::Call { args: v, .. }
			| Expr::MacroCall { args: v, .. }
			| Expr::Claim { fills: v, .. },
		) => asts(&mut v.iter()),
		(b"items", Expr::StructDef { fields, .. } | Expr::Fn { params: fields, .. }) => {
			list(fields.iter().map(|f| ast(field_ast(f))).collect())
		}
		(b"items", Expr::StructLit { fields, .. }) => asts(&mut fields.iter().map(|(_, v)| v)),
		(b"items", Expr::Bind { value: Some(v), .. }) => asts(&mut std::iter::once(&**v)),
		(b"items", Expr::Arm(arm)) => list(vec![
			leak(arm.patterns[0].clone()) as i64,
			leak((Expr::Block(arm.body.clone()), *span)) as i64,
		]),
		(b"items", Expr::EnumDef { variants, .. }) => {
			list(variants.iter().map(|v| ast(Expr::Ident(v.name.clone()))).collect())
		}
		(b"items", _) => flag("this Ast has no items", list(vec![])),
		(b"fills", Expr::StructDef { fills, .. } | Expr::EnumDef { fills, .. } | Expr::Claim { fills, .. }) => {
			asts(&mut fills.iter())
		}
		(b"fills", _) => flag("this Ast has no fills", list(vec![])),
		(b"==", Expr::Ident(n)) => {
			let bytes = unsafe { runtime::str_bytes(arg as *const runtime::StrHeader) };
			(n.as_bytes() == bytes) as i64
		}
		(b"==", _) => 0,
		_ => flag("unknown Ast method", leak(empty()) as i64),
	}
}
