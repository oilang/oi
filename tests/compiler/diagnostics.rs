use crate::helpers::*;

#[test]
fn undefined_variable() {
	fail_with("foo", "undefined variable");
}

#[test]
fn undefined_function() {
	fail_with("bar()", "undefined function");
}

#[test]
fn wrong_arg_count() {
	let src = indoc! {"
		add :: fn(x: int, y: int) int { x + y }
		add(1)
	"};
	fail_with(src, "expects 2 argument");
}

#[test]
fn wrong_arg_type() {
	let src = indoc! {r#"
		double :: fn(x: int) int { x + x }
		double("nope")
	"#};
	fail_with(src, "expected int argument");
}

#[test]
fn wrong_return_type() {
	let src = indoc! {r#"
		bad :: fn() int { "nope" }
		bad()
	"#};
	fail_with(src, "expected int return value");
}

#[test]
fn unknown_return_type() {
	let src = indoc! {"
		bad :: fn() blob { 1 }
		bad()
	"};
	fail_with(src, "unknown type `blob`");
}

#[test]
fn return_keyword_wrong_type() {
	let src = indoc! {"
		bad :: fn() int { return 2.0 }
		bad()
	"};
	fail_with(src, "expected int return value");
}

#[test]
fn type_mismatch() {
	fail_with(r#"1 + "x""#, "cannot apply `+`");
}

#[test]
fn unexpected_token() {
	fail_with("2 +", "expected");
}

#[test]
fn invalid_token() {
	fail_with(r"\", r"unexpected character `\`");
}

#[test]
fn assign_to_immutable() {
	fail_with(["x :: 1", "x = 2"], "cannot assign to immutable");
}

#[test]
fn assign_to_undefined() {
	fail_with("x = 5", "cannot assign to undefined variable");
}

#[test]
fn assign_wrong_type() {
	fail_with(["x := 1", "x = 2.0"], "cannot assign float");
}

#[test]
fn annotation_space_after_at() {
	fail_with(
		["@ c", "f :: fn() int { 42 }"],
		"must immediately follow, with no space",
	);
}

#[test]
fn parse_error_names_its_file() {
	crate::common::Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.hi())"])
		.file("foo/a.oi", ["module foo", "pub bad :: fn() int { ) }"])
		.file("foo/b.oi", ["module foo", "pub hi :: fn() int { 1 }"])
		.fail_with("foo/a.oi:2");
}

#[test]
fn top_level_stmt_with_main() {
	let src = indoc! {"
		main :: fn() {
			1
		}
		2
	"};
	fail_with(src, "top-level statements");
}
