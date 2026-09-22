use crate::helpers::*;

#[test]
fn paren_call_trailing_fn() {
	let src = indoc! {"
		retry :: fn(n: int, f: fn() int) int { f() }
		retry(2) fn() int { 21 }
	"};
	check(src, "21");
}

#[test]
fn bare_block_desugars_to_anon_fn() {
	let src = indoc! {"
		retry :: fn(n: int, f: fn() int) int { f() }
		retry(2) { 21 }
	"};
	check(src, "21");
}

#[test]
fn trailing_only_no_parens() {
	let src = indoc! {"
		twice :: fn(f: fn() int) int { f() + f() }
		twice fn() int { 21 }
	"};
	check(src, "42");
}

#[test]
fn method_trailing_fn() {
	let src = indoc! {"
		Box :: struct { n: int }
		Box :< {
			with :: fn(self, f: fn() int) int { self.n + f() }
			m :: fn(self, k: int, f: fn() int) int { self.n + k + f() }
		}
		b :: Box.{ n = 10 }
		print(b.with fn() int { 5 })
		b.m(1) fn() int { 5 }
	"};
	check(src, ["15", "16"]);
}

#[test]
fn leading_literals() {
	let src = indoc! {r#"
		Box :: struct { n: int }
		Box :< { tag :: fn(self, a: :go) int { self.n } }
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
fn literal_and_trailing_fn() {
	let src = indoc! {r#"
		run_test :: fn(name: string, f: fn() int) int { print(name) f() }
		run_test "reg" fn() int { 21 }
	"#};
	check(src, ["reg", "21"]);
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
		logret :: fn() string { print(1) "done" }
		logret()
	"#};
	check(src, ["1", "done"]);
}

#[test]
fn bind_rhs_trailing_fn() {
	let src = indoc! {"
		twice :: fn(f: fn() int) int { f() + f() }
		x :: twice fn() int { 21 }
		x
	"};
	check(src, "42");
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
		hook :: fn(p: Phase, f: fn() int) int { print(p) f() }
		hook .startup { 21 }
	"};
	check(src, ["startup", "21"]);
}

// a `.` after a newline is still a method chain, not a juxt arg
#[test]
fn newline_led_dot_stays_access() {
	let src = indoc! {r#"
		"ab"
			.len
	"#};
	check(src, "2");
}
