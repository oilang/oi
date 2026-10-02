use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A `Command` for the `oi` binary.
pub fn oi(args: &[&str]) -> Command {
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_oi"));
	cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
	cmd
}

// Ends a command chain, runs command, and returns output.
pub trait Run {
	fn run(&mut self, stdin: Option<&str>) -> Output;
}

impl Run for Command {
	fn run(&mut self, stdin: Option<&str>) -> Output {
		self.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
		let mut child = self.spawn().unwrap();
		if let Some(input) = stdin {
			child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
		}
		child.wait_with_output().unwrap()
	}
}

/// Assert success and a clean stderr, and return trimmed stdout.
pub fn ok(out: Output) -> String {
	assert!(
		out.status.success(),
		"oi failed:\n{}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert!(
		out.stderr.is_empty(),
		"unexpected stderr:\n{}",
		String::from_utf8_lossy(&out.stderr)
	);
	trim(&out.stdout)
}

/// Assert failure and return trimmed stderr.
#[allow(dead_code)]
pub fn err(out: Output) -> String {
	assert!(
		!out.status.success(),
		"expected failure, stdout:\n{}",
		String::from_utf8_lossy(&out.stdout)
	);
	trim(&out.stderr)
}

/// Lossy stderr text.
pub fn stderr(out: &Output) -> String {
	String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Run a built binary.
#[allow(dead_code)]
pub fn bin(path: impl AsRef<Path>) -> Output {
	Command::new(path.as_ref()).output().unwrap()
}

/// Sorted `.oi` files directly under a repo-relative dir.
#[allow(dead_code)]
pub fn oi_files(dir: &str) -> Vec<PathBuf> {
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
	let mut paths: Vec<_> = (std::fs::read_dir(root).unwrap())
		.map(|e| e.unwrap().path())
		.filter(|p| p.extension().is_some_and(|e| e == "oi"))
		.collect();
	paths.sort();
	assert!(!paths.is_empty(), "no .oi files found in {dir}");
	paths
}

/// Strip a single trailing newline.
pub fn trim(bytes: &[u8]) -> String {
	let s = String::from_utf8(bytes.to_vec()).unwrap();
	s.strip_suffix('\n').unwrap_or(&s).to_string()
}

/// Text joined by newlines.
pub trait Lines {
	fn text(&self) -> String;
}

impl Lines for &str {
	fn text(&self) -> String {
		(*self).to_string()
	}
}

impl Lines for &String {
	fn text(&self) -> String {
		(*self).clone()
	}
}

impl<const N: usize> Lines for [&str; N] {
	fn text(&self) -> String {
		self.join("\n")
	}
}

/// A project written under a fresh temp dir, deleted on drop.
#[allow(dead_code)]
pub struct Project(PathBuf);

#[allow(dead_code)]
impl Project {
	pub fn new() -> Self {
		use std::sync::atomic::{AtomicUsize, Ordering};
		static N: AtomicUsize = AtomicUsize::new(0);
		let n = N.fetch_add(1, Ordering::Relaxed);
		let dir = std::env::temp_dir().join(format!("oi_{}_{n}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		Project(dir)
	}

	pub fn file(self, path: &str, content: impl Lines) -> Self {
		let full = self.0.join(path);
		std::fs::create_dir_all(full.parent().unwrap()).unwrap();
		std::fs::write(full, content.text()).unwrap();
		self
	}

	/// Write main.oi under a `module main` header.
	pub fn main(self, body: impl Lines) -> Self {
		self.file("main.oi", &format!("module main\n{}", body.text()))
	}

	/// Write `<name>/lib.oi` under a `module <name>` header.
	pub fn lib(self, name: &str, body: impl Lines) -> Self {
		self.file(&format!("{name}/lib.oi"), &format!("module {name}\n{}", body.text()))
	}

	/// A headerless main that imports a `cext` module of foreign decls.
	pub fn foreign(main: impl Lines, decls: impl Lines) -> Self {
		let cext = format!("module cext\n{}", decls.text());
		Self::new()
			.file("main.oi", &format!("use cext\n{}", main.text()))
			.file("cext.oi", &cext)
	}

	/// Run `oi` in the project dir.
	pub fn oi(&self, args: &[&str]) -> Output {
		oi(args).current_dir(self).run(None)
	}

	/// Like `oi`, asserting success and returning stdout.
	pub fn ok(&self, args: &[&str]) -> String {
		ok(self.oi(args))
	}

	/// Run main.oi and assert its output.
	pub fn check(self, expected: impl Lines) {
		assert_eq!(ok(self.run()), expected.text());
	}

	/// Run main.oi and assert the failure mentions `expected`.
	pub fn fail_with(self, expected: &str) {
		let out = err(self.run());
		assert!(out.contains(expected), "{out}");
	}

	/// Run main.oi and return the raw output.
	pub fn run(&self) -> Output {
		self.oi(&["run", "main.oi"])
	}
}

impl Default for Project {
	fn default() -> Self {
		Self::new()
	}
}

impl AsRef<Path> for Project {
	fn as_ref(&self) -> &Path {
		&self.0
	}
}

impl Drop for Project {
	fn drop(&mut self) {
		std::fs::remove_dir_all(&self.0).ok();
	}
}
