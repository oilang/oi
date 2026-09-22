use crate::helpers::*;

#[test]
fn ternary_true() {
	check(r#"if 2 > 1 { "yes" } else { "no" }"#, "yes");
}

#[test]
fn else_if_first() {
	let src = indoc! {r#"
		i :: 0
		if i == 0 { "zero" } else if i == 1 { "one" } else { "idk" }
	"#};
	check(src, "zero");
}

#[test]
fn else_if_middle() {
	let src = indoc! {r#"
		i :: 1
		if i == 0 { "zero" } else if i == 1 { "one" } else { "idk" }
	"#};
	check(src, "one");
}

#[test]
fn else_if_last() {
	let src = indoc! {r#"
		i :: 2
		if i == 0 { "zero" } else if i == 1 { "one" } else { "idk" }
	"#};
	check(src, "idk");
}

#[test]
fn no_else_true() {
	check("if true { 42 }", "42");
}

#[test]
fn no_else_false_int() {
	check("if false { 42 }", "0");
}

#[test]
fn no_else_false_string() {
	check(r#"if false { "idk" }"#, "");
}

#[test]
fn if_as_binding() {
	let src = indoc! {"
		x :: if true { 10 } else { 20 }
		x
	"};
	check(src, "10");
}

#[test]
fn nested_if() {
	let src = indoc! {"
		x :: if true { if false { 1 } else { 2 } } else { 3 }
		x
	"};
	check(src, "2");
}

#[test]
fn branch_binding_is_local() {
	let src = indoc! {"
		x := 1
		if true {
			y :: 5
			x = y
		}
		x
	"};
	check(src, "5");
}

#[test]
fn branch_binding_does_not_leak() {
	let src = indoc! {"
		if true { y :: 5 }
		y
	"};
	fail_with(src, "undefined variable");
}

#[test]
fn guard_return_taken() {
	let src = indoc! {"
		abs :: fn(x: int) int {
			if x < 0 { return -x }
			x
		}
		abs(-5)
	"};
	check(src, "5");
}

#[test]
fn return_in_one_branch() {
	let src = indoc! {"
		pick :: fn(x: int) int {
			if x > 0 { return 1 } else { 99 }
		}
		pick(5)
	"};
	check(src, "1");
}

#[test]
fn condition_must_be_bool() {
	fail_with("if 1 { 2 } else { 3 }", "must be Bool");
}

#[test]
fn mismatched_branches() {
	fail_with(r#"if true { 1 } else { "x" }"#, "mismatched types");
}

#[test]
fn do_body() {
	check(r#"if 2 > 1 do "yes" else do "no""#, "yes");
}

#[test]
fn do_else_if_chain() {
	let src = indoc! {r#"
		i :: 1
		if i == 0 do "zero" else if i == 1 do "one" else do "idk"
	"#};
	check(src, "one");
}

#[test]
fn header_pattern_bind() {
	check(
		indoc! {"
			Coin :: enum { quarter(int) penny }
			if .quarter.(cents) := Coin.quarter.(25) { print(cents) }
		"},
		"25",
	);
	check(
		indoc! {r#"
			Coin :: enum { quarter(int) penny }
			if .quarter.(cents) := Coin.penny { print(cents) } else { print("nope") }
		"#},
		"nope",
	);
	check(
		indoc! {"
			Coin :: enum { quarter(int) penny }
			if .quarter.(cents) := Coin.penny { cents }
		"},
		"0",
	);
}

#[test]
fn header_binding() {
	let src = indoc! {"
		x := 5
		if x := 9 do print(x)
		print(x)
		if (a, b) :: (1, 2) do print(a + b)
		if n : int do print(n)
	"};
	check(src, ["9", "5", "3", "0"]);
	fail_with(
		r#"if x := 5 { print(x) } else { print("dead") }"#,
		"this binding always succeeds, so `else` can never run",
	);
}

#[test]
fn header_bind_scoped_to_body() {
	let src = indoc! {"
		Coin :: enum { quarter(int) penny }
		if .quarter.(state) := Coin.penny { print(state) } else { print(state) }
	"};
	fail_with(src, "undefined variable `state`");
}

#[test]
fn do_guard_return() {
	let src = indoc! {"
		abs :: fn(x: int) int {
			if x < 0 do return -x
			x
		}
		abs(-5)
	"};
	check(src, "5");
}
