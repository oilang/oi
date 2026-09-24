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
		oi(&["run", "main.oi"]).current_dir(self).run(None)
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
