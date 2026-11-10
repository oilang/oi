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
fn do_body_leaves_commas_to_the_call() {
	check(
		[
			"f :: fn(a: int, b: int) int do a * b",
			"print(f(if false do 2 else do 3, 5))",
		],
		"15",
	);
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
		if (v := a) && v > 0 && (w := a) { print(v + w) }
		if (v := a) && (w := b) { print(v + w) } else { print("none") }
	"#};
	check(src, ["3", "none", "6", "none"]);
}

#[test]
fn header_bind_is_mutable() {
	let src = indoc! {r#"
		x : ?int = 3
		if i := x { i += 1; print(i) }
		f :: fn() ?string do "a"
		loop s := f() { s += "b"; print(s); break }
		if .some.(n) := x { n += 2; print(n) }
		loop .some.(s) := f() { s += "c"; print(s); break }
	"#};
	check(src, ["4", "ab", "5", "ac"]);
	assert_clean(src);
}

#[test]
fn header_bind_statement_drops_its_value() {
	let src = indoc! {r#"
		g :: fn(xs: []string; after: ?string) int {
			i := 0
			if k := after { loop n in xs { i = 1 } }
			i
		}
		print(g(["a"], none), g(["a"], "b"))
	"#};
	check(src, "0 1");
	assert_clean(src);
}

#[test]
fn header_binds_stay_in_the_construct() {
	check("if (x := 5) > 9 { print(0) } else if x > 1 { print(x) }", "5");
	check("if { x := 5; x > 1 } { print(x) }", "5");
	check("match { x := 2; x } { 2 => print(x), _ => () }", "2");
	check("loop x in { ys := [1 2]; ys } { print(x, ys.len()) }", ["1 2", "2 2"]);
	fail(["if (x := 5) > 0 { print(x) }", "print(x)"], "undefined variable `x`");
}

#[test]
fn header_clauses() {
	let src = indoc! {r#"
		f :: fn() ?int do 5
		g :: fn() ?int do none
		if a := f(), b := f(), a > b do print(a) else do print("no")
		if n := 12; n > 9; n > 99 do print(n) else do print("small")
		if a := f(); b := g() do print(a + b) else do print("miss")
		k := 2
		if k in [1 2], a := f() do print(k + a)
		if a := g(), b := 2 do print(a + b) else do print("miss")
	"#};
	check(src, ["no", "small", "miss", "7", "miss"]);
}
