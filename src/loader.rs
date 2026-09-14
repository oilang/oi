//! Multi-file module loading.
//! Read imported dirs, parse and validate each file, and qualify definition names as `module::name`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chumsky::{input::Stream, prelude::*};
use include_dir::{Dir, include_dir};

use crate::Reported;
use crate::ast::{Annotation, BinOp, Expr, Span, Spanned, TypeExpr, UseItem};
use crate::diagnostics::{Diagnostic, SourceMap};
use crate::lexer::lex_at;
use crate::parser::parser;

static CORE: Dir = include_dir!("$CARGO_MANIFEST_DIR/core");

fn core_files(dir: &Dir) -> Vec<(String, String)> {
	let mut files: Vec<_> = dir
		.files()
		.filter(|f| f.path().extension().is_some_and(|x| x == "oi"))
		.collect();
	files.sort_by_key(|f| f.path());
	files
		.into_iter()
		.map(|f| {
			(
				format!("core/{}", f.path().display()),
				f.contents_utf8().unwrap_or_default().to_string(),
			)
		})
		.collect()
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
	pub env: HashMap<String, String>,
	pub visible: HashMap<String, Visible>,
	pub module: String,
}

impl Scope {
	// Resolve a bare name through this module's env, qualifying a miss into its own module.
	pub(crate) fn qualify_name(&self, name: &str) -> String {
		match self.env.get(name) {
			Some(q) => q.clone(),
			None if self.module.is_empty() => name.to_string(),
			None => format!("{}::{name}", self.module),
		}
	}

	// Resolve a bare trait name, leaving qualified names and `Drop` alone.
	pub(crate) fn qualify_trait(&self, name: &str) -> String {
		if name.contains("::") || name == "Drop" {
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

// A whole program, with its source files and modules and pubs, oh my.
#[derive(Default)]
pub struct Program {
	pub map: SourceMap,
	pub modules: Vec<Module>,
	pub publics: HashSet<String>,
	pub reexports: HashMap<String, String>,
	pub consts: HashMap<String, Spanned<Expr>>,
	pub annotations: HashMap<String, Vec<Annotation>>,
	pub roots: Vec<PathBuf>,
}

impl Program {
	// All items with their module's scope.
	pub fn items(&self) -> impl Iterator<Item = (&Scope, &Spanned<Expr>)> {
		self.modules.iter().flat_map(|m| m.items.iter().map(move |i| (&m.scope, i)))
	}
}

fn err(msg: impl Into<String>, span: Span, label: &str) -> Diagnostic {
	Diagnostic::new(msg.into(), span.into_range()).with_label(label)
}

// Ensure const values are literals.
pub(crate) fn is_literal(e: &Expr) -> bool {
	match e {
		Expr::Bool(_) | Expr::Int(_) | Expr::Float(_) | Expr::String(_) | Expr::Atom(_) => true,
		Expr::Negative(inner) => is_literal(&inner.0),
		_ => false,
	}
}

// Fold a const initializer down to a literal, so simple arithmetic doesn't need `comp`.
pub(crate) fn fold_const(e: &Expr, consts: &HashMap<String, Spanned<Expr>>, scope: &Scope) -> Option<Expr> {
	let fold = |e: &Spanned<Expr>| fold_const(&e.0, consts, scope);
	Some(match e {
		Expr::Negative(v) => match fold(v)? {
			Expr::Int(n) => Expr::Int(n.checked_neg()?),
			Expr::Float(f) => Expr::Float(-f),
			_ => return None,
		},
		Expr::Ident(n) => fold_const(&consts.get(&scope.qualify_name(n))?.0, consts, scope)?,
		Expr::Binary(op, a, b) => match (op, fold(a)?, fold(b)?) {
			(BinOp::Add, Expr::Int(a), Expr::Int(b)) => Expr::Int(a.checked_add(b)?),
			(BinOp::Sub, Expr::Int(a), Expr::Int(b)) => Expr::Int(a.checked_sub(b)?),
			(BinOp::Mul, Expr::Int(a), Expr::Int(b)) => Expr::Int(a.checked_mul(b)?),
			(BinOp::Div, Expr::Int(a), Expr::Int(b)) => Expr::Int(a.checked_div(b)?),
			(BinOp::Add, Expr::Float(a), Expr::Float(b)) => Expr::Float(a + b),
			(BinOp::Sub, Expr::Float(a), Expr::Float(b)) => Expr::Float(a - b),
			(BinOp::Mul, Expr::Float(a), Expr::Float(b)) => Expr::Float(a * b),
			(BinOp::Div, Expr::Float(a), Expr::Float(b)) => Expr::Float(a / b),
			(BinOp::Pow, Expr::Int(a), Expr::Int(b)) => Expr::Int(a.checked_pow(u32::try_from(b).ok()?)?),
			(BinOp::Pow, Expr::Float(a), Expr::Float(b)) => Expr::Float(a.powf(b)),
			_ => return None,
		},
		_ if is_literal(e) => e.clone(),
		_ => return None,
	})
}

// Fill an associated const fill.
fn const_fill(e: &Expr) -> Option<(&String, &Spanned<Expr>)> {
	match e {
		Expr::Pub(b) => const_fill(&b.0),
		Expr::Bind {
			name, value: Some(v), ..
		} => Some((name, v)),
		_ => None,
	}
}

// The definition an attribute macro wraps, and whether it is public.
fn wrapped_def(e: &mut Expr) -> Option<(bool, &mut String)> {
	match e {
		Expr::Pub(b) => wrapped_def(&mut b.0).map(|(_, n)| (true, n)),
		Expr::Annotated(_, b) => wrapped_def(&mut b.0),
		Expr::Fn { name, .. }
		| Expr::StructDef { name, .. }
		| Expr::EnumDef { name, .. }
		| Expr::TypeAlias { name, .. }
		| Expr::TraitDef { name, .. } => Some((false, name)),
		_ => None,
	}
}

// Unit and struct literals also count as const values.
fn is_const_value(e: &Expr) -> bool {
	match e {
		Expr::Tuple(fields) => fields.is_empty(),
		Expr::StructLit { fields, .. } => fields.iter().all(|(_, v)| is_literal(&v.0)),
		_ => is_literal(e),
	}
}

// Lex and parse the file just pushed onto the map.
fn parse_file(map: &SourceMap, base: usize) -> Result<Vec<Spanned<Expr>>, Reported> {
	let src = map.last_src();
	let toks = lex_at(src, base);
	let eoi = (base + src.len()..base + src.len()).into();
	parser(src, base)
		.parse(Stream::from_iter(toks).map(eoi, |t| t))
		.into_result()
		.map_err(|errs| {
			for e in &errs {
				Diagnostic::from_rich(e).report_mapped(map);
			}
			Reported
		})
}

struct Loader {
	// entry dir first, then `OI_PATH`
	roots: Vec<PathBuf>,
	entry_paths: Vec<PathBuf>,
	map: SourceMap,
	modules: Vec<Module>,
	publics: HashSet<String>,
	reexports: HashMap<String, String>,
	consts: HashMap<String, Spanned<Expr>>,
	annotations: HashMap<String, Vec<Annotation>>,
	// import stack
	loading: Vec<String>,
	selected: Vec<(String, String, Span)>,
	// modules loaded from the embedded core tree, allowed to import internal mods
	core_origin: HashSet<String>,
}

impl Loader {
	fn report(&self, diag: Diagnostic) -> Reported {
		diag.report_mapped(&self.map);
		Reported
	}

	// Record a definition.
	fn define(
		&mut self,
		m: &mut Module,
		name: &mut String,
		qualify: bool,
		public: bool,
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
		if public {
			self.publics.insert(name.clone());
		}
		Ok(())
	}

	// Validate a file and fold its items into the module, qualifying names as they land.
	fn add_file(
		&mut self,
		m: &mut Module,
		imports: &mut Vec<(String, Span)>,
		mut file: Vec<Spanned<Expr>>,
	) -> Result<(), Diagnostic> {
		let main = m.name == "main";
		let file_start = m.items.len();
		// enforce V-like module declaration rules (for now, as a pretty sane starting point)
		match file.first() {
			Some((Expr::Module(name), span)) if main && name != "main" => {
				return Err(err("the entry file is module `main`", *span, "rename it to `main`"));
			}
			Some((Expr::Module(name), span)) if *name != m.name => {
				return Err(err(
					format!("this file must declare `module {}`", m.name),
					*span,
					"wrong module name",
				));
			}
			Some((Expr::Module(_), _)) => {
				file.remove(0);
			}
			Some((_, span)) if !main => {
				return Err(err(
					format!("this file must declare `module {}`", m.name),
					*span,
					"add it as the first line",
				));
			}
			_ => {}
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
			let public = matches!(item.0, Expr::Pub(_));
			let mut item = match item {
				(Expr::Pub(inner), _) => *inner,
				item => item,
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
			if let Expr::Use { name, path, group } = &item.0 {
				let (module, _) = &path[0];
				if path.len() > 2 || (path.len() == 2 && group.is_some()) {
					return Err(err(
						"nested module paths aren't supported yet",
						item.1,
						"flatten the path",
					));
				}
				// a `.item` import tail acts as a one-item group
				let items: Vec<UseItem> = match (path.get(1), group) {
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
					self.selected.push((module.clone(), remote.clone(), *span));
				}
				let narrows = name.is_some() && group.is_some();
				if public && (items.is_empty() || narrows) {
					return Err(err(
						"only item imports can be re-exported yet",
						item.1,
						"import it privately instead",
					));
				}
				if narrows || items.is_empty() {
					// bind the module itself, or narrowed to its specified items
					let local = name.as_ref().map_or(module, |(n, _)| n).clone();
					let only =
						narrows.then(|| items.iter().map(|it| (it.local.0.clone(), it.remote().0.clone())).collect());
					let vis = Visible {
						module: module.clone(),
						only,
					};
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
				} else {
					// bind the items
					for it in &items {
						let (local, span) = &it.local;
						let target = format!("{module}::{}", it.remote().0);
						if public {
							self.reexports.insert(format!("{}::{local}", m.name), target.clone());
						}
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
				self.annotations.remove(&prev);
				self.publics.remove(&prev);
			}
			if let Expr::MacroCall { args, .. } = &mut item.0
				&& let Some((pubbed, name)) = args.first_mut().and_then(|a| wrapped_def(&mut a.0))
			{
				self.define(m, name, !main, public || pubbed, span)?;
				m.items.push(item);
				continue;
			}
			match &mut item.0 {
				Expr::Fn { name, .. }
				| Expr::StructDef { name, .. }
				| Expr::EnumDef { name, .. }
				| Expr::TypeAlias { name, .. }
				| Expr::TraitDef { name, .. } => {
					self.define(m, name, !main, public, span)?;
					if !anns.is_empty() {
						self.annotations.entry(name.clone()).or_default().extend(anns);
					}
				}
				Expr::MacroDef { name, .. } => self.define(m, name, !main, public, span)?,
				Expr::Bind {
					mutable,
					name,
					typ,
					value,
				} if !main || matches!(value.as_deref(), Some((Expr::Foreign, _))) => {
					if let Some(v) = value.as_deref_mut()
						&& let Some(f) = fold_const(&v.0, &self.consts, &m.scope)
					{
						v.0 = f;
					}
					let bad = match (*mutable, typ.is_some(), value.as_deref()) {
						(true, ..) => Some(("a module-level binding must be a const", "use `::`")),
						(_, _, Some(v)) if matches!(v.0, Expr::Foreign) => match typ {
							Some((TypeExpr::Fn(..), _)) => {
								self.define(m, name, !main, public, span)?;
								if !anns.is_empty() {
									self.annotations.entry(name.clone()).or_default().extend(anns);
								}
								None
							}
							_ => Some(("foreign globals aren't supported yet", "only foreign fns are supported")),
						},
						(_, true, _) => {
							Some(("type annotations on consts aren't supported yet", "drop the annotation"))
						}
						(_, _, Some(v)) if is_const_value(&v.0) || matches!(v.0, Expr::Comp(_)) => {
							self.define(m, name, true, public, span)?;
							let mut v = v.clone();
							if let Expr::StructLit { name: n, .. } = &mut v.0
								&& !n.is_empty()
							{
								*n = m.scope.qualify_name(n);
							}
							self.consts.insert(name.clone(), v);
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
				} if is_const_value(&v.0) => {
					let mut c = (**v).clone();
					if let Expr::StructLit { name: n, .. } = &mut c.0
						&& !n.is_empty()
					{
						*n = m.scope.qualify_name(n);
					}
					self.consts.insert(name.clone(), c);
				}
				Expr::Claim { typ, fills, .. } => {
					if !main && !crate::compiler::TypeCtx::builtin_type(typ) {
						*typ = format!("{}::{typ}", m.name);
					}
					// pull const fills out as associated consts, keyed `Type::name`
					for (n, v) in fills.iter().filter_map(|f| const_fill(&f.0)) {
						let mut lit = fold_const(&v.0, &self.consts, &m.scope)
							.or_else(|| is_const_value(&v.0).then(|| v.0.clone()))
							.ok_or_else(|| {
								err("cannot evaluate this at compile time", v.1, "not a const expression")
							})?;
						if let Expr::StructLit { name: sn, .. } = &mut lit
							&& !sn.is_empty()
						{
							*sn = m.scope.qualify_name(sn);
						}
						self.consts.insert(format!("{typ}::{n}"), (lit, v.1));
					}
					fills.retain(|f| const_fill(&f.0).is_none());
				}
				Expr::Doc(_) => {}
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
		}
		Ok(())
	}

	// Seal a module, then load its imports.
	fn seal(&mut self, module: Module, imports: Vec<(String, Span)>) -> Result<(), Reported> {
		self.loading.push(module.name.clone());
		self.modules.push(module);
		for (name, span) in imports {
			self.load_module(&name, span)?;
		}
		self.loading.pop();
		Ok(())
	}

	// Parse a set of files into one module.
	fn load_files(&mut self, name: &str, files: Vec<(String, String)>) -> Result<(), Reported> {
		let mut module = Module::new(name);
		let mut imports = vec![];
		for (file, src) in files {
			let base = self.map.push(file, src);
			let items = parse_file(&self.map, base)?;
			self.add_file(&mut module, &mut imports, items).map_err(|d| self.report(d))?;
		}
		self.seal(module, imports)
	}

	// Load a dir as one module.
	fn load_module(&mut self, name: &str, span: Span) -> Result<(), Reported> {
		if self.loading.iter().any(|m| m == name) {
			let msg = format!("import cycle: {} -> {name}", self.loading.join(" -> "));
			return Err(self.report(err(msg, span, "closes a cycle")));
		}
		if name == "rt" && !self.loading.last().is_some_and(|m| self.core_origin.contains(m)) {
			return Err(self.report(err("`rt` is internal to core", span, "not importable here")));
		}
		if self.modules.iter().any(|m| m.name == name) {
			return Ok(());
		}
		// resolve core's imports from internal files
		let from_core = self.loading.last().is_some_and(|m| self.core_origin.contains(m));
		let file = format!("{name}.oi");
		let has = |r: &&PathBuf| {
			r.join(name).is_dir() || (r.join(&file).is_file() && !self.entry_paths.contains(&r.join(&file)))
		};
		let root = self.roots.iter().find(has).unwrap_or(&self.roots[0]);
		let mut disk: Vec<_> = if from_core {
			vec![]
		} else {
			fs::read_dir(root.join(name))
				.into_iter()
				.flatten()
				.flatten()
				.map(|e| e.path())
				.filter(|p| p.extension().is_some_and(|x| x == "oi"))
				.collect()
		};
		disk.sort();
		let candidate = root.join(file);
		let mut files: Vec<(String, String)> = if !disk.is_empty() {
			disk.into_iter()
				.map(|path| {
					(
						path.display().to_string(),
						fs::read_to_string(&path).unwrap_or_default(),
					)
				})
				.collect()
		} else if !from_core && candidate.is_file() && !self.entry_paths.contains(&candidate) {
			vec![(
				candidate.display().to_string(),
				fs::read_to_string(&candidate).unwrap_or_default(),
			)]
		} else {
			vec![]
		};
		if files.is_empty() {
			files = match (CORE.get_dir(name), CORE.get_file(format!("{name}.oi"))) {
				(Some(d), _) => core_files(d),
				(_, Some(f)) => vec![(
					format!("core/{name}.oi"),
					f.contents_utf8().unwrap_or_default().to_string(),
				)],
				_ => vec![],
			};
			if !files.is_empty() {
				self.core_origin.insert(name.to_string());
			}
		}
		if files.is_empty() {
			return Err(self.report(err(format!("cannot find module `{name}`"), span, "no such module")));
		}
		self.load_files(name, files)
	}

	// Collapse `pub use` chains to their final targets, then point every binding at them.
	fn resolve_reexports(&mut self) -> HashMap<String, String> {
		let resolved: HashMap<_, _> = self
			.reexports
			.keys()
			.map(|alias| {
				let mut target = &self.reexports[alias];
				while let Some(next) = self.reexports.get(target) {
					target = next;
				}
				(alias.clone(), target.clone())
			})
			.collect();
		for m in &mut self.modules {
			for target in m.scope.env.values_mut() {
				if let Some(t) = resolved.get(target) {
					*target = t.clone();
				}
			}
		}
		resolved
	}

	// Every module implicitly uses core.
	fn seed_prelude(&mut self) {
		let core_pub: Vec<&String> = self.publics.iter().filter(|q| q.starts_with("core::")).collect();
		for m in self.modules.iter_mut().filter(|m| m.name != "core") {
			for q in &core_pub {
				m.scope
					.env
					.entry(q.strip_prefix("core::").unwrap().to_string())
					.or_insert_with(|| (*q).clone());
			}
		}
	}

	// Ensure selected names are public within their module.
	fn check_selected(&self) -> Result<(), Reported> {
		for (module, name, span) in &self.selected {
			let m = self.modules.iter().find(|m| &m.name == module).unwrap();
			let is_def = |q: &String| {
				self.consts.contains_key(q)
					|| self.modules.iter().any(|m| {
						m.items.iter().any(|i| {
							i.0.def_name() == Some(q)
								|| matches!(&i.0, Expr::MacroDef { name, .. } | Expr::Bind { name, .. } if name == q)
						})
					})
			};
			let (msg, label) = match m.scope.env.get(name) {
				None => (format!("module `{module}` has no `{name}`"), "no such name"),
				Some(q) if !is_def(q) => (
					format!("`{name}` cannot be imported"),
					"only fns and types can be imported for now",
				),
				Some(q) if !self.publics.contains(q) => {
					(format!("`{name}` is private to module `{module}`"), "not public")
				}
				_ => continue,
			};
			return Err(self.report(err(msg, *span, label)));
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

// Load the whole program from the entry's files.
// `root` anchors module lookups.
pub fn load(entry: Entry, root: &Path) -> Result<Program, Reported> {
	let deps = std::env::var_os("OI_PATH").unwrap_or_default();
	let deps = std::env::split_paths(&deps).filter(|p| !p.as_os_str().is_empty());
	let mut loader = Loader {
		roots: std::iter::once(root.to_path_buf())
			.chain(deps)
			.chain(std::iter::once(home().join("lib")))
			.collect(),
		entry_paths: entry
			.iter()
			.map(|(n, _)| root.join(Path::new(n).file_name().unwrap_or_default()))
			.collect(),
		map: SourceMap::default(),
		modules: vec![],
		publics: HashSet::new(),
		reexports: HashMap::new(),
		consts: HashMap::new(),
		annotations: HashMap::new(),
		loading: vec![],
		selected: vec![],
		core_origin: HashSet::from(["core".to_string()]),
	};
	// import core implicitly
	loader.load_files("core", core_files(&CORE))?;
	loader.load_files("main", entry)?;
	let reexports = loader.resolve_reexports();
	loader.seed_prelude();
	loader.check_selected()?;
	Ok(Program {
		roots: loader.roots.clone(),
		map: loader.map,
		modules: loader.modules,
		publics: loader.publics,
		reexports,
		consts: loader.consts,
		annotations: loader.annotations,
	})
}
