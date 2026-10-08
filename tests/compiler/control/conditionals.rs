use crate::helpers::*;

#[test]
fn else_if_chains() {
	let src = indoc! {r#"
		name :: fn(i: int) string { if i == 0 { "zero" } else if i == 1 { "one" } else { "idk" } }
		name_do :: fn(i: int) string { if i == 0 do "zero" else if i == 1 do "one" else do "idk" }
		print(name(0))
		print(name(1))
		print(name(2))
		print(name_do(1))
		print(name_do(2))
		if 2 > 1 { "yes" } else { "no" }
	"#};
	check(src, ["zero", "one", "idk", "one", "idk", "yes"]);
}

#[test]
fn no_else_yields_zero_value() {
	check("if true { 42 }", "42");
	check("if false { 42 }", "0");
	check(r#"if false { "idk" }"#, "");
}

#[test]
fn if_as_binding() {
	let src = indoc! {"
		x :: if true { 10 } else { 20 }
		y :: if true { if false { 1 } else { 2 } } else { 3 }
		x + y
	"};
	check(src, "12");
}

#[test]
fn branch_bindings_are_local() {
	let src = indoc! {"
		x := 1
		if true {
			y :: 5
			x = y
		}
		x
	"};
	check(src, "5");
	fail(["if true { y :: 5 }", "y"], "undefined variable");
}

#[test]
fn return_from_a_branch() {
	let src = indoc! {"
		abs :: fn(x: int) int {
			if x < 0 { return -x }
			x
		}
		abs_do :: fn(x: int) int {
			if x < 0 do return -x
			x
		}
		pick :: fn(x: int) int {
			if x > 0 { return 1 } else { 99 }
		}
		print(abs(-5))
		print(abs_do(-6))
		print(pick(5))
	"};
	check(src, ["5", "6", "1"]);
}

#[test]
fn condition_must_be_bool() {
	fail("if 1 { 2 } else { 3 }", "must be Bool");
}

#[test]
fn mismatched_branches() {
	fail(r#"if true { 1 } else { "x" }"#, "mismatched types");
}

#[test]
fn discarded_branch_mismatch_is_allowed() {
	let src = indoc! {r#"
		f :: fn(n: int) []int {
			if n < 0 { print("bad") }
			else if n == 0 { [0] }
			else { [n] }
			[n, n]
		}
		print(f(5))
		print(f(-1))
	"#};
	check(src, ["[5, 5]", "bad", "[-1, -1]"]);
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
	fail(
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
	fail(src, "undefined variable `state`");
}

#[test]
fn header_bind_unwraps() {
	let src = indoc! {r#"
		a: ?int = 3
		b: ?int = none
		if v := a { print(v) } else { print("none") }
		if v := b { print(v) } else { print("none") }
	"#};
	check(src, ["3", "none"]);
}
