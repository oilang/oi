//! Multi-file module loading.
//! Read imported dirs, parse and validate each file, and qualify definition names as `module::name`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use chumsky::{input::Stream, prelude::*};
use include_dir::{Dir, File, include_dir};

use crate::Reported;
use crate::ast::{Annotation, BinOp, Child, Expr, Span, Spanned, TypeExpr, UseItem, Vis, int_word};
use crate::compiler::{builtin, sized};
use crate::diagnostics::{Diagnostic, SourceMap};
use crate::lexer::{Token, lex_at, prescan, splice_raw};
use crate::parser::parser;

static CORE: Dir = include_dir!("$CARGO_MANIFEST_DIR/core");

/// Source of an embedded core module.
pub fn core_source(module: &str) -> Option<&'static str> {
	let path = module.replace('.', "/");
	let file = CORE
		.get_file(format!("{path}.oi"))
		.or_else(|| CORE.get_file(format!("{path}/mod.oi")));
	file?.contents_utf8()
}

fn core_file(file: &File) -> (String, String) {
	let src = file.contents_utf8().unwrap_or_default();
	(format!("core/{}", file.path().display()), src.into())
}

fn core_files(dir: &Dir) -> Vec<(String, String)> {
	let mut files: Vec<_> = dir
		.files()
		.filter(|f| f.path().extension().is_some_and(|x| x == "oi"))
		.collect();
	files.sort_by_key(|f| f.path());
	files.into_iter().map(core_file).collect()
}

// A parsed module.
#[derive(Clone)]
pub struct Module {
	pub name: String,
	pub items: Vec<Spanned<Expr>>,
	pub scope: Scope,
}

impl Module {
	fn new(name: &str) -> Self {
		Module {
			name: name.into(),
			items: vec![],
			scope: Scope {
				module: if name == "main" { String::new() } else { name.into() },
				..Scope::default()
			},
		}
	}
}

// A module's view of names.
#[derive(Default, Clone)]
pub struct Scope {
	header: Option<String>,
	pub env: HashMap<String, String>,
	pub visible: HashMap<String, Visible>,
	pub module: String,
	pub files: Vec<(Range<usize>, Scope)>,
	pub withs: Vec<String>,
}

// Whether `name` is a reserved hook trait.
pub(crate) fn is_hook_trait(name: &str) -> bool {
	matches!(name, "Drop" | "Copy")
}

// The method a hook trait's fill defines.
pub(crate) fn hook_method(name: &str) -> &'static str {
	if name == "Copy" { "copy" } else { "drop" }
}

impl Scope {
	// The scope of the file holding `span`.
	pub(crate) fn at(&self, span: Span) -> &Scope {
		self.files
			.iter()
			.find(|(r, _)| r.contains(&span.start))
			.map_or(self, |(_, s)| s)
	}

	// Resolve a bare name through this module's env, qualifying a miss into its own module.
	pub(crate) fn qualify_name(&self, name: &str) -> String {
		if let Some((m, t)) = name.split_once('.') {
			return match self.visible.get(m) {
				Some(vis) => {
					let t = vis.only.as_ref().and_then(|o| o.get(t)).map_or(t, String::as_str);
					format!("{}::{t}", vis.module)
				}
				None => name.to_string(),
			};
		}
		match self.env.get(name) {
			Some(q) => q.clone(),
			None if self.module.is_empty() => name.to_string(),
			None => format!("{}::{name}", self.module),
		}
	}

	// Resolve a bare trait name, leaving qualified names and the built-in hooks alone.
	pub(crate) fn qualify_trait(&self, name: &str) -> String {
		if name.contains("::") || is_hook_trait(name) {
			return name.to_string();
		}
		self.qualify_name(name)
	}
}

// A visible module.
#[derive(Clone)]
pub struct Visible {
	pub module: String,
	pub only: Option<HashMap<String, String>>,
}

// Qualified names visible outside their module, and each module's package.
#[derive(Default, Clone)]
pub struct Publics {
	pub vis: HashMap<String, Vis>,
	pub packages: HashMap<String, PathBuf>,
	pub uses: HashMap<String, Vec<String>>,
}

// The module a qualified key belongs to.
pub(crate) fn module_of(key: &str) -> Option<&str> {
	key.split_once("::").map(|(m, _)| m)
}

impl Publics {
	// Whether a key is visible from a given module.
	pub fn is_visible(&self, key: &str, from: &str) -> bool {
		let owner = module_of(key).unwrap_or_default();
		match self.vis.get(key) {
			Some(Vis::Package) => self.packages.get(owner) == self.packages.get(from),
			v => v.is_some(),
		}
	}
}

// A whole program, with its source files and modules and pubs, oh my.
#[derive(Default)]
pub struct Program {
	pub map: SourceMap,
	pub modules: Vec<Module>,
	pub publics: Publics,
	pub reexports: HashMap<String, String>,
	pub consts: HashMap<String, Spanned<Expr>>,
	pub const_types: HashMap<String, Spanned<TypeExpr>>,
	pub annotations: HashMap<String, Vec<Annotation>>,
	pub roots: Vec<PathBuf>,
	pub core_origin: HashSet<String>,
	pub selected: Vec<(String, String, String, Span)>,
}

impl Program {
	// All items with their file's scope.
	pub fn items(&self) -> impl Iterator<Item = (&Scope, &Spanned<Expr>)> {
		self.modules
			.iter()
			.flat_map(|m| m.items.iter().map(move |i| (m.scope.at(i.1), i)))
	}
}

fn err(msg: impl Into<String>, span: Span, label: &str) -> Diagnostic {
	Diagnostic::new(msg.into(), span.into_range()).with_label(label)
}

// Hint `core.x` when a local `x` shadows it.
pub(crate) fn shadow_note(d: Diagnostic, module: &str, core_origin: &HashSet<String>) -> Diagnostic {
	match CORE.get_dir(module.replace('.', "/")) {
		Some(_) if !core_origin.contains(module) => d.with_note(format!("local `{module}` shadows `core.{module}`")),
		_ => d,
	}
}

// Ensure const values are literals.
pub(crate) fn is_literal(e: &Expr) -> bool {
	match e {
		Expr::Bool(_) | Expr::Int(_) | Expr::Rune(_) | Expr::Float(_) | Expr::String(_) | Expr::Atom(_) => true,
		Expr::Negative(inner) => is_literal(&inner.0),
		_ => false,
	}
}

// Fold a const initializer down to a literal, so simple arithmetic doesn't need `comp`.
pub(crate) fn fold_const(e: &Expr, consts: &HashMap<String, Spanned<Expr>>, scope: &Scope) -> Option<Expr> {
	let fold = |e: &Spanned<Expr>| fold_const(&e.0, consts, scope);
	Some(match e {
		Expr::Negative(v) => match fold(v)? {
			Expr::Int(n) => Expr::Int(n.wrapping_neg()),
			Expr::Float(f) => Expr::Float(-f),
			_ => return None,
		},
		Expr::Not(v) => match fold(v)? {
			Expr::Int(n) => Expr::Int(!n),
			_ => return None,
		},
		Expr::Ident(n) => fold_const(&consts.get(&scope.qualify_name(n))?.0, consts, scope)?,
		Expr::Field { tuple, field } => match (&tuple.0, field.as_str()) {
			(Expr::Ident(name), bound @ ("min" | "max")) => {
				let (lo, hi) = builtin(name).or_else(|| sized(name, Span::default())?.ok())?.int_bounds()?;
				Expr::Int(if bound == "max" { hi } else { lo })
			}
			_ => return None,
		},
		Expr::Binary(op, a, b) => match (fold(a)?, fold(b)?) {
			(Expr::Int(a), Expr::Int(b)) => Expr::Int(match op {
				BinOp::Add => a.wrapping_add(b),
				BinOp::Sub => a.wrapping_sub(b),
				BinOp::Mul => a.wrapping_mul(b),
				BinOp::Div => a.checked_div(b)?,
				BinOp::Mod => a.checked_rem(b)?,
				BinOp::Pow => a.wrapping_pow(b.try_into().ok()?),
				BinOp::BitAnd => a & b,
				BinOp::BitOr => a | b,
				BinOp::BitXor => a ^ b,
				BinOp::Shl => a.wrapping_shl(b.try_into().ok()?),
				BinOp::Shr => a.wrapping_shr(b.try_into().ok()?),
				_ => return None,
			}),
			(Expr::Float(a), Expr::Float(b)) => Expr::Float(match op {
				BinOp::Add => a + b,
				BinOp::Sub => a - b,
				BinOp::Mul => a * b,
				BinOp::Div => a / b,
				BinOp::Pow => a.powf(b),
				_ => return None,
			}),
			(Expr::String(a), Expr::String(b)) if *op == BinOp::Add => Expr::String(a + &b),
			_ => return None,
		},
		_ if is_literal(e) => e.clone(),
		_ => return None,
	})
}

// The int const a sum member names, if any.
fn flag(t: &TypeExpr, consts: &HashMap<String, Spanned<Expr>>, scope: &Scope) -> Option<i64> {
	let TypeExpr::Name(n) = t else { return None };
	match fold_const(&Expr::Ident(n.clone()), consts, scope)? {
		Expr::Int(v) => int_word(v),
		_ => None,
	}
}

type ConstFill<'a> = (&'a String, &'a Option<Spanned<TypeExpr>>, &'a Spanned<Expr>);

// Fill an associated const.
fn const_fill(e: &Expr) -> Option<ConstFill<'_>> {
	match e {
		Expr::Pub(_, b) => const_fill(&b.0),
		Expr::Bind {
			name,
			typ,
			value: Some(v),
			..
		} => Some((name, typ, v)),
		_ => None,
	}
}

// The definition an attribute macro wraps, and whether it is public.
pub(crate) fn wrapped_def(e: &mut Expr) -> Option<(Option<Vis>, &mut String)> {
	match e {
		Expr::Pub(v, b) => wrapped_def(&mut b.0).map(|(_, n)| (Some(*v), n)),
		Expr::Annotated(_, b) => wrapped_def(&mut b.0),
		Expr::Fn { name, .. }
		| Expr::StructDef { name, .. }
		| Expr::EnumDef { name, .. }
		| Expr::TypeAlias { name, .. }
		| Expr::TraitDef { name, .. } => Some((None, name)),
		_ => None,
	}
}

// Unit and struct literals also count as const values.
fn is_const_value(e: &Expr) -> bool {
	match e {
		Expr::Tuple(fields) => fields.is_empty(),
		Expr::Array(items) => items.iter().all(|(v, _)| is_literal(v)),
		Expr::StructLit { fields, .. } => fields.iter().all(|(_, v)| is_literal(&v.0)),
		Expr::Field { tuple, .. } => {
			matches!(&tuple.0, Expr::Ident(n) if n.rsplit("::").next().is_some_and(|t| t.starts_with(char::is_uppercase)))
		}
		_ => is_literal(e),
	}
}

/// Read a source file, reporting failure.
pub fn read_src(path: &Path) -> Result<(String, String), Reported> {
	let src =
		fs::read_to_string(path).map_err(|e| crate::driver::fail(format!("cannot read {}: {e}", path.display())))?;
	Ok((path.display().to_string(), src))
}

fn walk_oi(dir: &Path) -> Vec<PathBuf> {
	let mut files = vec![];
	for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
		let path = entry.path();
		if path.extension().is_some_and(|x| x == "oi") {
			files.push(path);
		}
	}
	files
}

// Lex and parse one file's source at its base offset.
pub fn parse_file(
	file: &str,
	src: &str,
	base: usize,
	raw: &HashSet<String>,
) -> Result<Vec<Spanned<Expr>>, Vec<Diagnostic>> {
	let file = std::path::absolute(file).unwrap_or_default();
	let toks = splice_raw(lex_at(src, base), raw, src, base)
		.map_err(|span| vec![err("unterminated raw macro body", span, "this `{` never closes")])?
		.into_iter()
		.map(|(tok, span)| match tok {
			Token::PathString(p) => (Token::String(file.with_file_name(p).display().to_string()), span),
			tok => (tok, span),
		});
	let eoi = (base + src.len()..base + src.len()).into();
	parser(src, base)
		.parse(Stream::from_iter(toks).map(eoi, |t| t))
		.into_result()
		.map_err(|errs| errs.iter().map(Diagnostic::from_rich).collect())
}

#[derive(Default)]
struct Loader {
	program: Program,
	entry_paths: Vec<PathBuf>,
	reexported_modules: HashMap<String, Visible>,
	loading: Vec<String>,
	raw_macros: HashSet<String>,
	prescanned: HashSet<String>,
}

impl Loader {
	fn report(&self, diags: impl IntoIterator<Item = Diagnostic>) -> Reported {
		diags
			.into_iter()
			.map(|d| d.report_mapped(&self.program.map))
			.last()
			.unwrap_or(Reported)
	}

	// Record a definition.
	fn define(
		&mut self,
		m: &mut Module,
		name: &mut String,
		qualify: bool,
		public: Option<Vis>,
		span: Span,
	) -> Result<(), Diagnostic> {
		let bare = name.clone();
		if qualify {
			*name = format!("{}::{bare}", m.name);
		}
		if m.scope.env.insert(bare.clone(), name.clone()).is_some() {
			let msg = format!("`{bare}` is defined twice in module `{}`", m.name);
			return Err(err(msg, span, "duplicate definition"));
		}
		if let Some(v) = public {
			self.program.publics.vis.insert(name.clone(), v);
		}
		Ok(())
	}

	// Fold a const initializer to a literal, qualifying any type name it carries.
	fn const_value(&self, v: &Spanned<Expr>, scope: &Scope) -> Option<Spanned<Expr>> {
		let mut e =
			fold_const(&v.0, &self.program.consts, scope).or_else(|| is_const_value(&v.0).then(|| v.0.clone()))?;
		match &mut e {
			Expr::StructLit { name, .. } if !name.is_empty() => *name = scope.qualify_name(name),
			Expr::Field { tuple, .. } => {
				if let Expr::Ident(n) = &mut tuple.0 {
					*n = scope.qualify_name(n);
				}
			}
			_ => {}
		}
		Some((e, v.1))
	}

	// Pull const fills out as associated consts keyed `Type::name`, returning type fills as aliases.
	fn assoc_fills(
		&mut self,
		typ: &str,
		fills: &mut Vec<Spanned<Expr>>,
		scope: &Scope,
	) -> Result<Vec<Spanned<Expr>>, Diagnostic> {
		let mut aliases = vec![];
		for (n, t, v) in fills.iter().filter_map(|f| const_fill(&f.0)) {
			let key = format!("{typ}::{n}");
			if let Some(lit) = self.const_value(v, scope) {
				if let Some(t) = t {
					self.program.const_types.insert(key.clone(), t.clone());
				}
				self.program.consts.insert(key, lit);
			} else if let Some(typ) = TypeExpr::from_expr(&v.0) {
				let alias = Expr::TypeAlias {
					name: key,
					type_params: vec![],
					typ,
				};
				aliases.push((alias, v.1));
			} else {
				return Err(err(
					"cannot evaluate this at compile time",
					v.1,
					"not a const expression",
				));
			}
		}

		fills.retain(|f| const_fill(&f.0).is_none_or(|(.., v)| self.const_value(v, scope).is_none()));
		Ok(aliases)
	}

	// Validate a file and fold its items into the module, qualifying names as they land.
	fn add_file(
		&mut self,
		m: &mut Module,
		imports: &mut Vec<(String, Span)>,
		mut file: Vec<Spanned<Expr>>,
	) -> Result<Scope, Diagnostic> {
		let main = m.name == "main";
		let file_start = m.items.len();
		let mut locals = vec![];
		let mut withs = vec![];
		if let Some((Expr::Module(name), span)) = file.first() {
			if m.scope.header.get_or_insert_with(|| name.clone()) != name {
				return Err(err(
					"files in this module disagree",
					*span,
					"module name differs from the first file",
				));
			}
			file.remove(0);
		}
		for item in file {
			// peel off annotations
			let (anns, item) = match item {
				(Expr::Annotated(anns, inner), _) => (anns, *inner),
				item => (Vec::new(), item),
			};
			if matches!(item.0, Expr::Module(_)) {
				return Err(err("`module` must come first", item.1, "move it to the top"));
			}
			// peel off `pub` wrapper
			let (public, mut item) = match item {
				(Expr::Pub(v, inner), _) => (Some(v), *inner),
				item => (None, item),
			};
			if !anns.is_empty()
				&& item.0.def_name().is_none()
				&& !matches!(&item.0, Expr::Bind { value: Some(v), .. } if matches!(v.0, Expr::Foreign))
			{
				return Err(err(
					"annotations only attach to definitions",
					item.1,
					"not a definition",
				));
			}
			if let Expr::Use {
				name,
				path,
				group,
				with,
			} = &item.0
			{
				// `a.b.c` is module `a.b.c` if it's a dir, otherwise item `c` of `a.b`
				let dotted = |n: usize| path[..n].iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(".");
				let whole = path.len() == 1 || group.is_some() || self.is_module(&dotted(path.len()));
				let depth = path.len() - usize::from(!whole);
				let module = &self.canon(&dotted(depth));
				// a `.item` import tail acts as a one-item group
				let items: Vec<UseItem> = match (path.get(depth), group) {
					(Some(it), _) => vec![UseItem {
						local: name.clone().unwrap_or_else(|| it.clone()),
						rename_of: Some(it.clone()),
					}],
					(None, Some(items)) => items.clone(),
					(None, None) => vec![],
				};
				// ensure every imported item is public in its module
				for it in &items {
					let (remote, span) = it.remote();
					self.program
						.selected
						.push((module.clone(), remote.clone(), m.scope.module.clone(), *span));
				}
				let narrows = name.is_some() && group.is_some();
				if public == Some(Vis::Package) {
					return Err(err(
						"only `pub` imports can be re-exported",
						item.1,
						"`pub(package) use` is not a thing yet",
					));
				}
				if narrows || items.is_empty() {
					// bind the module itself, or narrowed to its specified items
					let local = name.as_ref().unwrap_or(&path[depth - 1]).0.clone();
					if *with {
						withs.push(local.clone());
					}
					let only =
						narrows.then(|| items.iter().map(|it| (it.local.0.clone(), it.remote().0.clone())).collect());
					let vis = Visible {
						module: module.clone(),
						only,
					};
					if public.is_some() {
						self.reexported_modules.insert(format!("{}::{local}", m.name), vis.clone());
						if !narrows {
							self.program
								.publics
								.uses
								.entry(m.name.clone())
								.or_default()
								.push(module.clone());
						}
					}
					// handle re-importing
					if let Some(prev) = m.scope.visible.insert(local.clone(), vis)
						&& (narrows || prev.only.is_some() || prev.module != *module)
					{
						return Err(err(
							format!("`{local}` already names module `{}`", prev.module),
							item.1,
							"conflicting import",
						));
					}
				} else if *with {
					return Err(err(
						"`with` needs a module import",
						item.1,
						"these are already bare names",
					));
				} else {
					// bind the items
					for it in &items {
						let (local, span) = &it.local;
						locals.push(local.clone());
						let target = format!("{module}::{}", it.remote().0);
						if public.is_some() {
							self.program.reexports.insert(format!("{}::{local}", m.name), target.clone());
						}
						m.scope.env.insert(format!("{local}!"), format!("{target}!"));
						if m.scope.env.insert(local.clone(), target).is_some() {
							let msg = format!("`{local}` is already defined in module `{}`", m.name);
							return Err(err(msg, *span, "conflicting import"));
						}
					}
				}
				imports.push((module.clone(), item.1));
				continue;
			}
			let span = item.1;
			// make fn bindings shadow
			if let Expr::Fn { name, .. } = &item.0
				&& let Some(prev) = m.scope.env.get(name.as_str()).cloned()
				&& let Some(i) = m.items[file_start..]
					.iter()
					.position(|it| matches!(&it.0, Expr::Fn { name: n, .. } if *n == prev))
			{
				m.items.remove(file_start + i);
				m.scope.env.remove(name.as_str());
				self.program.annotations.remove(&prev);
				self.program.publics.vis.remove(&prev);
			}
			if let Expr::MacroCall { args, .. } = &mut item.0
				&& let Some((pubbed, name)) = args.first_mut().and_then(|a| wrapped_def(&mut a.0))
			{
				self.define(m, name, !main, public.or(pubbed), span)?;
				m.items.push(item);
				continue;
			}
			if let Expr::TypeAlias {
				name,
				typ: TypeExpr::Sum(members),
				..
			} = &mut item.0
				&& let Some(v) = members
					.iter()
					.try_fold(0, |acc, t| Some(acc | flag(t, &self.program.consts, &m.scope)?))
			{
				self.define(m, name, true, public, span)?;
				self.program.consts.insert(name.clone(), (Expr::Int(v.into()), span));
				continue;
			}

			let mut assoc_types = vec![];
			match &mut item.0 {
				Expr::Fn { name, .. }
				| Expr::StructDef { name, .. }
				| Expr::EnumDef { name, .. }
				| Expr::TypeAlias { name, .. }
				| Expr::TraitDef { name, .. } => {
					self.define(m, name, !main, public, span)?;
					if !anns.is_empty() {
						self.program.annotations.entry(name.clone()).or_default().extend(anns);
					}
					match &mut item.0 {
						Expr::StructDef { name, fills, .. } => assoc_types = self.assoc_fills(name, fills, &m.scope)?,
						Expr::TraitDef { fields, .. } => {
							for f in fields {
								if let TypeExpr::Infer(v) = &f.typ
									&& self.const_value(v, &m.scope).is_none()
									&& TypeExpr::from_expr(&v.0).is_some()
								{
									f.typ = TypeExpr::Name("type".into());
								}
							}
						}
						_ => {}
					}
				}
				Expr::MacroDef { name, .. } => {
					name.push('!');
					self.define(m, name, !main, public, span)?;
				}
				Expr::Bind {
					mutable,
					name,
					typ,
					value,
				} if !main || matches!(value.as_deref(), Some((Expr::Foreign, _))) => {
					if let Some(v) = value.as_deref_mut()
						&& let Some(c) = self.const_value(v, &m.scope)
					{
						*v = c;
					}
					let bad = match (*mutable, typ.is_some(), value.as_deref()) {
						(true, _, Some(v)) if is_const_value(&v.0) || matches!(v.0, Expr::Comp(_)) => {
							self.define(m, name, !main, public, span)?;
							None
						}
						(true, ..) => Some(("a static needs a comptime initializer", "not a const expression")),
						(_, _, Some(v)) if matches!(v.0, Expr::Foreign) => match typ {
							Some((TypeExpr::Fn(..), _)) => {
								self.define(m, name, !main, public, span)?;
								if !anns.is_empty() {
									self.program.annotations.entry(name.clone()).or_default().extend(anns);
								}
								None
							}
							_ => Some(("foreign globals aren't supported yet", "only foreign fns are supported")),
						},
						(_, true, _) => {
							Some(("type annotations on consts aren't supported yet", "drop the annotation"))
						}
						(_, _, Some(v)) if is_const_value(&v.0) || matches!(v.0, Expr::Comp(_)) => {
							let v = v.clone();
							self.define(m, name, true, public, span)?;
							self.program.consts.insert(name.clone(), v);
							continue;
						}
						(_, _, Some(v)) if TypeExpr::from_expr(&v.0).is_some() => {
							self.define(m, name, true, public, span)?;
							None
						}
						_ => Some(("cannot evaluate this at compile time", "not a const expression")),
					};
					if let Some((msg, label)) = bad {
						return Err(err(msg, span, label));
					}
				}
				Expr::Bind {
					mutable: false,
					name,
					typ: None,
					value: Some(v),
				} => {
					if let Some(c) = self.const_value(v, &m.scope) {
						self.program.consts.insert(name.clone(), c);
					}
				}
				Expr::Claim {
					typ,
					type_params,
					fills,
					..
				} => {
					// `u64` is keyed as `uint`
					if let Some(Ok(sized)) = sized(typ, span) {
						*typ = sized.key();
					}
					if !main
						&& !crate::compiler::TypeCtx::builtin_type(typ)
						&& type_params.iter().all(|p| p.name != *typ)
					{
						*typ = format!("{}::{typ}", m.name);
					}
					assoc_types = self.assoc_fills(typ, fills, &m.scope)?;
				}
				Expr::Doc(_) | Expr::MacroCall { .. } => {}
				_ if !main => {
					return Err(err(
						"top-level statements aren't allowed in a module",
						span,
						"only definitions and imports",
					));
				}
				_ => {}
			}

			m.items.push(item);
			m.items.append(&mut assoc_types);
		}

		// imports stay with the file that wrote them
		let env = locals.into_iter().flat_map(|l| [format!("{l}!"), l]);
		let core = Visible {
			module: "core".into(),
			only: None,
		};
		m.scope.visible.entry("core".into()).or_insert(core);
		Ok(Scope {
			env: env.filter_map(|k| m.scope.env.remove_entry(&k)).collect(),
			visible: std::mem::take(&mut m.scope.visible),
			module: m.scope.module.clone(),
			withs,
			..Scope::default()
		})
	}

	// Seal a module, then load its imports.
	fn seal(&mut self, mut module: Module, imports: Vec<(String, Span)>) -> Result<(), Reported> {
		self.loading.push(module.name.clone());
		let at = self.program.modules.len();
		self.program.modules.push(Module::new(&module.name));
		for (name, span) in imports {
			self.load_module(&name, span)?;
		}
		// narrow after imports, so their re-exports are known
		let mut reached = vec![];
		for (range, scope) in &mut module.scope.files {
			for item in module.items.iter_mut().filter(|i| range.contains(&i.1.start)) {
				self.reach_submodules(item, scope, &mut reached);
			}
		}
		self.program.modules[at] = module;
		for (name, span) in reached {
			self.load_module(&name, span)?;
		}
		self.loading.pop();
		Ok(())
	}

	// Parse a set of files into one module.
	fn load_files(&mut self, name: &str, files: Vec<(String, String)>) -> Result<(), Reported> {
		let mut module = Module::new(name);
		let mut imports = vec![];
		self.prescan_module(name, &files);
		let bases: Vec<usize> = files.into_iter().map(|(file, src)| self.program.map.push(file, src)).collect();
		let (map, raw) = (&self.program.map, &self.raw_macros);
		let parsed: Vec<_> = std::thread::scope(|s| {
			let jobs: Vec<_> = bases
				.iter()
				.map(|&b| s.spawn(move || parse_file(map.locate_span(b..b).0, map.src(b), b, raw)))
				.collect();
			jobs.into_iter().map(|j| j.join().unwrap()).collect()
		});
		for (result, base) in parsed.into_iter().zip(bases) {
			let items = result.map_err(|ds| self.report(ds))?;
			let scope = self.add_file(&mut module, &mut imports, items).map_err(|d| self.report([d]))?;
			module.scope.files.push((base..base + self.program.map.src(base).len(), scope));
		}
		self.seal(module, imports)
	}

	// Whether `name` is a module dir.
	fn is_module(&self, name: &str) -> bool {
		let path = name.replace('.', "/");
		self.program.roots.iter().any(|r| r.join(&path).is_dir())
			|| CORE.get_dir(path.strip_prefix("core/").unwrap_or(&path)).is_some()
	}

	// Whether root `r` has module `dir`.
	fn on_disk(&self, r: &Path, dir: &str) -> bool {
		let file = r.join(format!("{dir}.oi"));
		r.join(dir).is_dir() || (file.is_file() && !self.entry_paths.contains(&file))
	}

	// `core.x` is `x` unless shadowed.
	fn canon(&self, name: &str) -> String {
		match name.strip_prefix("core.") {
			Some(x) if !self.program.roots.iter().any(|r| self.on_disk(r, &x.replace('.', "/"))) => x.into(),
			_ => name.into(),
		}
	}

	// Rewrite `m.sub` to an import of submodule `sub`.
	fn reach_submodules(&self, e: &mut Spanned<Expr>, scope: &mut Scope, imports: &mut Vec<(String, Span)>) {
		e.0.for_children(|c| match c {
			Child::List(list) => list.iter_mut().for_each(|e| self.reach_submodules(e, scope, imports)),
			Child::One(e) => self.reach_submodules(e, scope, imports),
		});
		if let Expr::Field { tuple, field } = &e.0
			&& let Expr::Ident(head) = &tuple.0
			&& let Some(Visible { module, only }) = scope.visible.get(head)
			&& let Some(vis) = self.reexported_modules.get(&format!("{module}::{field}")).cloned().or_else(|| {
				let module = self.canon(&format!("{module}.{field}"));
				(only.is_none() && self.is_module(&module)).then(|| {
					imports.push((module.clone(), e.1));
					Visible { module, only: None }
				})
			}) {
			let local = format!("{head}.{field}");
			scope.visible.insert(local.clone(), vis);
			e.0 = Expr::Ident(local);
		}
	}

	// Load a dir as one module.
	fn load_module(&mut self, name: &str, span: Span) -> Result<(), Reported> {
		if self.loading.iter().any(|m| m == name) {
			let msg = format!("import cycle: {} -> {name}", self.loading.join(" -> "));
			return Err(self.report([err(msg, span, "closes a cycle")]));
		}
		if name == "rt" && !self.loading.last().is_some_and(|m| self.program.core_origin.contains(m)) {
			return Err(self.report([err("`rt` is internal to core", span, "not importable here")]));
		}
		if self.program.modules.iter().any(|m| m.name == name) {
			return Ok(());
		}
		// resolve core's imports from internal files
		let from_core = self.loading.last().is_some_and(|m| self.program.core_origin.contains(m));
		let files = self.module_files(name, from_core)?;
		if files.is_empty() {
			return Err(self.report([err(format!("cannot find module `{name}`"), span, "no such module")]));
		}
		self.load_files(name, files)
	}

	// Find a module's files on disk, falling back to the embedded core tree.
	fn module_files(&mut self, name: &str, from_core: bool) -> Result<Vec<(String, String)>, Reported> {
		// `core.x` skips fs
		let from_core = from_core || name.starts_with("core.");
		let dir = &name.strip_prefix("core.").unwrap_or(name).replace('.', "/");
		let file = format!("{dir}.oi");
		let root = self
			.program
			.roots
			.iter()
			.find(|r| self.on_disk(r, dir))
			.unwrap_or(&self.program.roots[0]);
		let mut disk = if !from_core {
			walk_oi(&root.join(dir))
		} else {
			Vec::default()
		};
		disk.sort();
		let candidate = root.join(&file);
		let mut files: Vec<(String, String)> = if !disk.is_empty() {
			disk.iter().map(|path| read_src(path)).collect::<Result<_, _>>()?
		} else if !from_core && candidate.is_file() && !self.entry_paths.contains(&candidate) {
			vec![read_src(&candidate)?]
		} else {
			vec![]
		};
		if files.is_empty() {
			files = match (CORE.get_dir(dir), CORE.get_file(&file)) {
				(Some(d), _) => core_files(d),
				(_, Some(f)) => vec![core_file(f)],
				_ => vec![],
			};
			if !files.is_empty() {
				self.program.core_origin.insert(name.to_string());
			}
		} else {
			let lib = self.program.roots.last() == Some(root);
			let pkg = if lib {
				root.join(dir.split('/').next().unwrap())
			} else {
				root.clone()
			};
			self.program.publics.packages.insert(name.to_string(), pkg);
		}
		Ok(files)
	}

	// Find raw-stream macros in a module and everything it imports.
	fn prescan_module(&mut self, name: &str, files: &[(String, String)]) {
		if !self.prescanned.insert(name.to_string()) {
			return;
		}
		let mut imports = vec![];
		for (_, src) in files {
			let (raw, used) = prescan(&lex_at(src, 0));
			self.raw_macros.extend(raw);
			imports.extend(used);
		}
		let from_core = self.program.core_origin.contains(name);
		for m in imports {
			if !self.prescanned.contains(&m) {
				let inner = self.module_files(&m, from_core).unwrap_or_default();
				self.prescan_module(&m, &inner);
			}
		}
	}

	// Collapse `pub use` chains to their final targets, then point every binding at them.
	fn resolve_reexports(&mut self) {
		let resolved: HashMap<_, _> = self
			.program
			.reexports
			.keys()
			.map(|alias| {
				let mut target = &self.program.reexports[alias];
				while let Some(next) = self.program.reexports.get(target) {
					target = next;
				}
				(alias.clone(), target.clone())
			})
			.collect();
		for (_, s) in self.program.modules.iter_mut().flat_map(|m| &mut m.scope.files) {
			for target in s.env.values_mut() {
				if let Some(t) = resolved.get(target) {
					*target = t.clone();
				}
			}
		}
		self.program.reexports = resolved;
	}

	// Every module implicitly uses core.
	fn seed_prelude(&mut self) {
		let core_pub: Vec<&String> = self.program.publics.vis.keys().filter(|q| q.starts_with("core::")).collect();
		for m in self.program.modules.iter_mut().filter(|m| m.name != "core") {
			let core = Visible {
				module: "core".into(),
				only: None,
			};
			m.scope.visible.entry("core".into()).or_insert(core);
			for q in &core_pub {
				m.scope
					.env
					.entry(q.strip_prefix("core::").unwrap().to_string())
					.or_insert_with(|| (*q).clone());
			}
		}

		// files see their module's names under their own imports
		for m in &mut self.program.modules {
			for (_, f) in &mut m.scope.files {
				for (k, v) in &m.scope.env {
					f.env.entry(k.clone()).or_insert_with(|| v.clone());
				}
				for (k, v) in &m.scope.visible {
					f.visible.entry(k.clone()).or_insert_with(|| v.clone());
				}
			}
		}
	}

	// Re-key claims on imported types to their owning module, once the prelude is seeded.
	fn resolve_claims(&mut self) {
		for m in &mut self.program.modules {
			let own = format!("{}::", m.scope.module);
			for item in &mut m.items {
				if let Expr::Claim { typ, type_params, .. } = &mut item.0
					&& !crate::compiler::TypeCtx::builtin_type(typ)
					&& type_params.iter().all(|p| p.name != *typ)
				{
					*typ = m.scope.at(item.1).qualify_name(typ.strip_prefix(&own).unwrap_or(typ));
				}
			}
		}
	}
}

impl Program {
	// Ensure selected names are public within their module, once macros have expanded.
	pub(crate) fn check_selected(
		&self,
		selected: &[(String, String, String, Span)],
		is_macro: impl Fn(&str) -> bool,
	) -> Result<(), Diagnostic> {
		for (module, name, from, span) in selected {
			let m = self.modules.iter().find(|m| &m.name == module).unwrap();
			let is_def = |q: &str| {
				self.consts.contains_key(q)
					|| is_macro(q) || self.items().any(|(_, i)| {
					let i = &Expr::peel_meta(i).2.0;
					i.def_name() == Some(q) || matches!(i, Expr::Bind { name, .. } if name == q)
				})
			};
			let qualified = format!("{module}::{name}");
			let found = (m.scope.env.get(name).or_else(|| m.scope.env.get(&format!("{name}!"))))
				.or_else(|| self.reexports.get(&qualified))
				.or_else(|| is_def(&qualified).then_some(&qualified));
			let (msg, label) = match found {
				None => (format!("module `{module}` has no `{name}`"), "no such name"),
				Some(q) if !is_def(q) => (
					format!("`{name}` cannot be imported"),
					"only fns and types can be imported for now",
				),
				Some(q) if !self.publics.is_visible(q, from) => {
					(format!("`{name}` is private to module `{module}`"), "not public")
				}
				_ => continue,
			};
			return Err(shadow_note(err(msg, *span, label), module, &self.core_origin));
		}
		Ok(())
	}
}

/// The Oi home dir.
pub fn home() -> PathBuf {
	std::env::var_os("OI_HOME")
		.map(PathBuf::from)
		.unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".oi"))
}

// Entry files.
pub type Entry = Vec<(String, String)>;

// Load the whole program from the entry's files, with `root` anchoring module lookups.
pub fn load(entry: Entry, root: &Path) -> Result<Program, Reported> {
	let deps = std::env::var_os("OI_PATH").unwrap_or_default();
	let deps = std::env::split_paths(&deps).filter(|p| !p.as_os_str().is_empty());
	let program = Program {
		roots: std::iter::once(root.to_path_buf())
			.chain(deps)
			.chain(std::iter::once(home().join("lib")))
			.collect(),
		core_origin: HashSet::from(["core".to_string()]),
		..Program::default()
	};
	let mut loader = Loader {
		program,
		entry_paths: entry
			.iter()
			.map(|(n, _)| root.join(Path::new(n).file_name().unwrap_or_default()))
			.collect(),
		..Loader::default()
	};
	// import core implicitly
	loader.load_files("core", core_files(&CORE))?;
	loader.program.publics.packages.insert(String::new(), root.to_path_buf());
	loader.load_files("main", entry)?;
	loader.resolve_reexports();
	loader.seed_prelude();
	loader.resolve_claims();
	Ok(loader.program)
}
