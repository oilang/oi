use crate::common::{Run, oi, trim};

fn repl(input: &str) -> String {
	let out = oi(&["repl"]).run(Some(input));
	assert!(
		out.status.success(),
		"repl failed:\n{}",
		String::from_utf8_lossy(&out.stderr)
	);
	trim(&out.stdout)
}

#[test]
fn only_definitions_replay() {
	assert_eq!(repl("x := 1\nprint x\ny := 2\n"), "1\n1\n2");
}

#[test]
fn clear_resets_the_session() {
	assert_eq!(repl("x := 1\n:c\nprint x\n"), "1");
}
