use crate::helpers::*;

#[test]
fn trailing_fn() {
	let src = indoc! {r#"
		retry :: fn(n: int, f: fn() int) int { f() }
		twice :: fn(f: fn() int) int { f() + f() }
		run_test :: fn(name: string, f: fn() int) int { print(name); f() }
		print(retry(2) fn() int { 21 })
		print(retry(2) { 22 })
		x :: twice fn() int { 21 }
		print(x)
		run_test "reg" fn() int { 23 }
	"#};
	check(src, ["21", "22", "42", "reg", "23"]);
}

#[test]
fn method_trailing_fn() {
	let src = indoc! {"
		Box :: struct { n: int }
		Box :< {
			add :: fn(self; f: fn() int) int { self.n + f() }
			m :: fn(self; k: int, f: fn() int) int { self.n + k + f() }
		}
		b :: Box.{ n = 10 }
		print(b.add fn() int { 5 })
		b.m(1) fn() int { 5 }
	"};
	check(src, ["15", "16"]);
}

#[test]
fn trailing_fn_chains() {
	let src = indoc! {"
		xs := [1, 2, 3, 4]
		print(xs.filter { $ > 2 }.map { $ * 10 }, !xs.any { $ > 9 })
	"};
	check(src, "[30, 40] true");
}

#[test]
fn trailing_fn_on_fn_value() {
	let src = indoc! {"
		fs := [fn(g: fn(int) int) int { g(1) }]
		a := fs[0] { $ + 1 }
		b := fs[0]() { $ + 2 }
		print(a, b)
	"};
	check(src, "2 3");
}

#[test]
fn leading_literals() {
	let src = indoc! {r#"
		Box :: struct { n: int }
		Box :< { tag :: fn(self; a: :go) int { self.n } }
		shout :: fn(s: string) string { s }
		s :: shout "hey"
		print(s)
		Box.{ n = 10 }.tag :go
	"#};
	check(src, ["hey", "10"]);
}

#[test]
fn leading_arg_is_any_expr() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		p :: Point.{ x = 1, y = 2 }
		print p
		print p.x + 10
		print -2
		print 1 - 2
	"#};
	check(src, ["Point.{x = 1, y = 2}", "11", "-2", "-1"]);
}

#[test]
fn headers_stay_juxt_free() {
	let src = indoc! {"
		cond :: true
		if cond { print(1) }
		i := 0
		loop i < 3 { i = i + 1 }
		print(i)
		x :: 5
		match x { 5 => print(9), else => print(0) }
	"};
	check(src, ["1", "3", "9"]);
}

#[test]
fn call_then_literal_return() {
	let src = indoc! {r#"
		logret :: fn() string { print(1); "done" }
		logret()
	"#};
	check(src, ["1", "done"]);
}

#[test]
fn lists_beat_leading_args() {
	let src = indoc! {"
		lat :: 1
		long :: 2
		print((lat long 4))
		print([lat long 4])
	"};
	check(src, ["(1, 2, 4)", "[1, 2, 4]"]);
}

#[test]
fn macro_stmt_arg_then_block() {
	let src = indoc! {"
		m! :: fn(n: Ast, b: Ast) Ast { `if %n { %{..b.items} }` }
		m! true { print(1) }
		m! false { print(9) }
	"};
	check(src, "1");
}

#[test]
fn macro_block_arg() {
	let src = indoc! {"
		m! :: fn(b: Ast) Ast { `%{..b.items}` }
		m! { print(2) }
		m!({ print(3) })
	"};
	check(src, ["2", "3"]);
}

#[test]
fn juxt_enum_shorthand_arg() {
	let src = indoc! {"
		Phase :: enum { startup }
		hook :: fn(p: Phase, f: fn() int) int { print(p); f() }
		hook .startup { 21 }
	"};
	check(src, ["startup", "21"]);
}

#[test]
fn newline_led_dot_stays_access() {
	let src = indoc! {r#"
		"ab"
			.len()
	"#};
	check(src, "2");
}

#[test]
fn leading_arg_method_claims_block() {
	let src = indoc! {"
		retry :: fn(n: int, f: fn() int) { print(f() + n) }
		attempts :: 3
		xs := [1, 2, 3, 4]
		print xs.filter { $ > 2 }.map { $ * 10 }
		assert! xs.any { $ > 2 }
		retry attempts { 10 }
		retry 3 { 20 }
	"};
	check(src, ["[30, 40]", "13", "23"]);

	let src = indoc! {r#"
		Count :: struct { n: int }
		Count : Iterator[int] < {
			next :: fn(mut self) ?int { if self.n == 0 do return none; self.n -= 1; self.n }
		}
		it: Iterator[int] = Count.{ n = 6 }
		print(it.next(), it.map { $ * 2 }.filter { $ > 2 }.collect())
	"#};
	check(src, "some.(5) [8, 6, 4]");
}
