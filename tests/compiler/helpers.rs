use std::process::Output;

pub(crate) use indoc::indoc;
use pretty_assertions::assert_eq;

use crate::common::{Lines, Run, oi, ok, trim};

/// Run `src` through `oi exec`.
fn exec(src: &str) -> Output {
	oi(&["exec"]).run(Some(src))
}

/// Run provided source, returning trimmed stdout.
pub(crate) fn run(src: &str) -> String {
	ok(exec(src))
}

/// Run provided source, returning (trimmed stdout, raw stderr).
pub(crate) fn run_streams(src: &str) -> (String, String) {
	let out = exec(src);
	(trim(&out.stdout), trim(&out.stderr))
}

/// Run provided source expecting it to fail, returning the error.
fn failure(src: impl Lines, at_compile_time: bool, expected: &str) -> String {
	let src = src.text();
	let out = exec(&src);
	let err = trim(&out.stderr);
	// the compiler's own diagnostics carry ariadne's banner; a runtime abort never does
	let compiled = !err.starts_with("Error:");
	assert!(
		!out.status.success() && compiled == !at_compile_time && err.contains(expected),
		"expected a {} containing {expected:?}\nsrc:\n{src}\nstatus: {:?}\nstdout:\n{}\nstderr:\n{err}",
		if at_compile_time { "compile error" } else { "runtime abort" },
		out.status,
		String::from_utf8_lossy(&out.stdout)
	);
	err
}

/// Run provided source expecting the compiler to reject it, with an error containing `expected`
/// (`""` for any).
pub(crate) fn fail(src: impl Lines, expected: &str) -> String {
	failure(src, true, expected)
}

/// Run provided source expecting it to compile, then abort at runtime.
pub(crate) fn fail_rt(src: impl Lines, expected: &str) -> String {
	failure(src, false, expected)
}

/// Run provided source expecting a given result.
pub(crate) fn check(src: impl Lines, expected: impl Lines) {
	let src = src.text();
	assert_eq!(run(&src), expected.text(), "\nsrc:\n{src}");
}

/// Run under the leak checker, returning the live-allocation count at exit.
pub(crate) fn leaks(src: impl Lines) -> i64 {
	let src = src.text();
	let out = oi(&["exec"]).env("OI_LEAK_CHECK", "1").run(Some(&src));
	assert!(
		out.status.success(),
		"src:\n{src}\nstderr:\n{}",
		String::from_utf8_lossy(&out.stderr)
	);
	let err = String::from_utf8_lossy(&out.stderr).to_string();
	let count = err
		.lines()
		.find_map(|l| l.strip_prefix("leaked allocations: "))
		.unwrap_or_else(|| panic!("no leak report\nsrc:\n{src}\nstderr:\n{err}"));
	count.parse().unwrap()
}

/// Run and assert every allocation was freed.
pub(crate) fn assert_clean(src: impl Lines) {
	let src = src.text();
	assert_eq!(leaks(&src), 0, "leaked\nsrc:\n{src}");
}
