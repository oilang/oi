use crate::common::{Run, oi, stderr, trim};

fn repl(input: &str) -> String {
	let out = oi(&["repl"]).run(Some(input));
	assert!(out.status.success(), "repl failed:\n{}", stderr(&out));
	trim(&out.stdout)
}

#[test]
fn only_definitions_replay() {
	assert_eq!(repl("x := 1\nprint x\ny := 2\n"), "1\n1\n2");
}

#[test]
fn raw_macro_body_replays() {
	let src = indoc::indoc! {r#"
		r! :: fn(body: Tokens) Ast { `%{body.items.len}` }
		n := r! { a "}" b }
		print(n)
	"#};
	assert_eq!(repl(src), "3\n3");
}

#[test]
fn clear_resets_the_session() {
	assert_eq!(repl("x := 1\n:c\nprint x\n"), "1");
}
