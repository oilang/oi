use crate::helpers::*;

#[test]
fn calls_and_captures() {
	let src = indoc! {"
		apply :: fn(f: fn(int) int, x: int) int { f(x) }
		mul :: fn [] (x: int, y: int) int { x * y }
		double :: fn [] (n: int) int { n * 2 }
		factor :: 3
		triple :: fn [factor] (x: int) int { x * factor }
		moved :: fn [move factor] (x: int) int { x * factor }
		f := fn(x: int) int { x + 1 }
		g := fn() int { f(2) }
		quad := fn(x: int) int do x * 4
		print(mul(6, 7), apply(double, 21), triple(4), moved(4), apply(triple, 2), g(), quad(3))
	"};
	check(src, "42 42 12 12 6 3 12");
}

#[test]
fn bad_calls() {
	fail(
		["add :: fn [] (x: int, y: int) int { x + y }", "add(1)"],
		"expects 2 argument",
	);
	fail(
		["add :: fn [] (x: int, y: int) int { x + y }", "add(1, 2.0)"],
		"wrong argument type",
	);
	fail(["x :: 5", "x()"], "not callable");
	fail(["f :: fn [missing] () int { 0 }", "f()"], "undefined variable");
	fail(
		["x :: 3", "f :: fn [mut x] () int { x }", "f()"],
		"cannot capture `x` as `mut`",
	);
}

#[test]
fn implicit_captures() {
	let src = indoc! {"
		n :: 10
		b :: 32
		nums :: [1, 2, 3]
		scale :: fn(x: int) int { x * n }
		add :: fn() int { n + b }
		inner :: fn() int { n :: 5; n }
		param :: fn(n: int) int { n * 2 }
		sum :: fn() int {
			s := 0
			loop n in nums { s = s + n }
			s
		}
		print(scale(5), add(), inner() + n, param(4) + n, sum())
	"};
	check(src, "50 42 15 18 6");
}

#[test]
fn closure_cannot_escape() {
	let src = indoc! {"
		make :: fn() {
			n :: 10
			return fn() int { n }
		}
		make()
	"};
	fail(src, "borrows its captures, so it can't be returned");
	fail(
		["n :: 10", "arr :: [fn [n] () int { n }]"],
		"borrows its captures, so it can't be stored in an array",
	);

	let src = indoc! {"
		smuggle[T] :: fn(x: T) []T {
			a: []T
			a << x
			a
		}
		n :: 10
		smuggle(fn [n] () int { n })
	"};
	fail(src, "borrows its captures, so it can't be stored in an array");
	fail(
		["n :: 10", r#"["a" = fn [n] () int { n }]"#],
		"borrows its captures, so it can't be stored in a map",
	);
	fail(
		[
			"Box[T] :: struct { v: T }",
			"n :: 10",
			"Box.{ v = fn [n] () int { n } }",
		],
		"borrows its captures, so it can't be stored in a field",
	);

	let src = indoc! {"
		Box :: struct { f: fn() int }
		keep :: fn(f: fn() int) Box { Box.{ f = f } }
		apply :: fn(f: fn() int) int { f() }
		n :: 10
		print(apply(fn [n] () int { n }))
		keep(fn [n] () int { n })
	"};
	fail(
		src,
		"borrows its captures, so it can't be passed to `keep`, which keeps it",
	);

	let src = indoc! {"
		pair :: fn() (fn() int, int) {
			n :: 10
			return (fn [n] () int { n }, 1)
		}
		pair()
	"};
	fail(src, "borrows its captures, so it can't be returned");
}

#[test]
fn zeroed_fns() {
	let src = indoc! {"
		f: fn()
		f()
		g: fn(int) bool
		print(g(7))
	"};
	check(src, "false");
	let src = indoc! {"
		P :: struct { pee: float }
		Q :: struct { cue: int, p: P }
		p: fn() P
		q: fn() Q
		print(p())
		print(q())
	"};
	check(src, ["P.{pee = 0.0}", "Q.{cue = 0, p = P.{pee = 0.0}}"]);
}

#[test]
fn capture_mut_writes_visible_outside() {
	let src = indoc! {"
		counter := 0
		inc :: fn [mut counter] () int { counter = counter + 1; counter }
		inc()
		inc()
		counter
	"};
	check(src, "2");
}

#[test]
fn move_capture_escapes_via_return() {
	let src = indoc! {"
		make :: fn() fn() int {
			xs :: [7]
			return fn [move xs] () int { xs[0] }
		}
		f :: make()
		f()
	"};
	check(src, "7");
}

#[test]
fn apply_a_fn_value_that_isnt_a_name() {
	let src = indoc! {"
		make :: fn() fn() int { fn() int { 7 } }
		fns :: [fn() int { 9 }]
		print(make()())
		print(fns[0]())
	"};
	check(src, ["7", "9"]);
}

#[test]
fn move_capture_kills_the_name() {
	let src = indoc! {"
		xs :: [7]
		f :: fn [move xs] () int { xs[0] }
		print(xs)
	"};
	fail(src, "undefined variable");
}

#[test]
fn move_capture_of_fn_param_is_borrowed() {
	let src = indoc! {"
		make :: fn(xs: []int) fn() int {
			fn [move xs] () int { xs[0] }
		}
		make([1])
	"};
	fail(src, "cannot move `xs`, it is borrowed here");
}

#[test]
fn move_capture_inside_loop_of_outer_binding() {
	let src = indoc! {"
		xs :: [1]
		i := 0
		loop i < 2 {
			f :: fn [move xs] () int { xs[0] }
			i = i + 1
		}
	"};
	fail(src, "cannot move `xs` out of the enclosing loop");
}

#[test]
#[ignore]
// FIX: broke by sandwiches
fn implicit_capture_ignores_match_bound_name() {
	let src = indoc! {r#"
		r :: !int.(7)
		f :: fn() int {
			match r {
				.ok.(n) => n * 2,
				.err.(e) => -1,
			}
		}
		f()
	"#};
	check(src, "14");
}

#[test]
fn bare_fn_still_needs_ret_without_context() {
	let src = indoc! {"
		f := fn { 21 }
	"};
	fail(src, "explicit return type");
}

#[test]
fn trailing_and_inline_literals_infer() {
	let src = indoc! {r#"
		retry :: fn(n: int, f: fn() int) int { f() + n }
		apply :: fn(f: fn() int) int { f() }
		run :: fn(f: fn() ()) { f() }
		op :: fn(n: int, f: fn(int) int) int { f(n) }
		k := 21
		print(retry(2) { 21 }, retry(2) fn { 21 }, retry(0) { k * 2 }, apply(fn { 7 }))
		run { print("hi") }
		print(op(4, fn { $ * 4 }), op(4, { $ * 4 }), op(4) { $ + 1 })
	"#};
	check(src, ["23 23 42 7", "hi", "16 16 5"]);
}

#[test]
fn params_tuple_inferred() {
	let src = indoc! {r#"
		apply :: fn(f: fn(int, string) string) string { f(1, "x") }
		apply(fn { $.1 + "{$.0}" })
	"#};
	check(src, "x1");
}

#[test]
fn literal_against_declared_type() {
	let src = indoc! {"
		Op :: fn(int) int
		double : fn(int) int : { $ * 2 }
		t : Op : fn(n) { n * 3 }
		B :: struct { f: fn(int) int = { $ * 2 } }
		C :: struct { f: fn(int) int = { $ } }
		b := B.{}
		c := C.{ f = { $ * 5 } }
		g := b.f
		h := c.f
		print(double(4), t(3), g(4), h(4))
	"};
	check(src, "8 9 8 20");
}

#[test]
fn local_fn_recurses() {
	let src = indoc! {"
		run :: fn() int {
			fac :: fn(n: int) int { if n < 2 { 1 } else { n * fac(n - 1) } }
			fac2 : fn(int) int : fn(n) { if n < 2 { 1 } else { n * fac2(n - 1) } }
			step := 2
			count :: fn(n: int) int { if n <= 0 { 0 } else { step + count(n - step) } }
			fac(5) + fac2(5) + count(20)
		}
		print(run())
	"};
	check(src, "260");
}

#[test]
fn unit_fn_var_rebinds() {
	let src = indoc! {"
		f : fn(int) = { print($) }
		f(7)
		f = { print($ + 1) }
		f(7)
	"};
	check(src, ["7", "8"]);
}

#[test]
fn named_fn_param_needs_a_type() {
	let src = indoc! {"
		f :: fn(x) { x + 1 }
		print(f(1))
	"};
	fail(src, "needs a type");
}

#[test]
fn fn_field_calls_without_parens() {
	let src = indoc! {"
		Iface :: struct { call: @c fn(int) int }
		add1 :: @c fn(n: int) int { n + 1 }
		i := Iface.{call = add1}
		print(i.call(41))
	"};
	check(src, "42");
}

#[test]
fn map_value_block_literal() {
	let src = indoc! {r#"
		Handler :: fn(string) string
		routes : [string]Handler : ["/" = { "home:{$}" }]
		h := routes["/"]
		print(h("x"))
	"#};
	check(src, "home:x");
}

#[test]
fn ret_less_fn_type_ends_its_line() {
	let src = indoc! {"
		Handler :: fn(int)
		x := 0
		x = 1
		call :: fn(n: int, h: Handler) { h(n) }
		call(x) { print($) }
	"};
	check(src, "1");
}
