use crate::helpers::*;

#[test]
fn fn_call() {
	let src = indoc! {"
		double :: fn() int { 21 * 2 }
		double()
	"};
	check(src, "42");
}

#[test]
fn fn_vars() {
	let src = indoc! {"
		area :: fn() int {
			width :: 12
			height :: 5
			width * height
		}

		area()
	"};
	check(src, "60");
}

#[test]
fn fn_args() {
	let src = indoc! {"
		add :: fn(x: int, y: int) int {
			x + y
		}
		add(3, 4)
	"};
	check(src, "7");
}

#[test]
fn fn_args_nested() {
	let src = indoc! {"
		add :: fn(x: int, y: int) int { x + y }
		add3 :: fn(a: int, b: int, c: int) int { add(add(a, b), c) }
		add3(1, 2, 3)
	"};
	check(src, "6");
}

#[test]
fn fn_arg_float() {
	let src = indoc! {"
		scale :: fn(x: f64) f64 { x * 2.0 }
		scale(2.5)
	"};
	check(src, "5.0");
}

#[test]
fn fn_arg_trailing_comma() {
	let src = indoc! {"
		add :: fn(x: int, y: int,) int { x + y }
		add(40, 2,)
	"};
	check(src, "42");
}

#[test]
fn self_recursion() {
	let src = indoc! {"
		fact :: fn(n: int) int { if n <= 1 { 1 } else { n * fact(n - 1) } }
		fact(5)
	"};
	check(src, "120");
}

#[test]
fn forward_reference() {
	let src = indoc! {"
		a :: fn() int { b() + 1 }
		b :: fn() int { 41 }
		a()
	"};
	check(src, "42");
}

#[test]
fn fn_arg_wrong_type() {
	let src = indoc! {"
		i :: fn(x: int) int { x }
		i(2.4)
	"};
	fail_with(src, "wrong argument type");
}

#[test]
fn fn_return_keyword() {
	let src = indoc! {"
		add :: fn(x: int, y: int) int {
			return x + y
		}
		add(3, 4)
	"};
	check(src, "7");
}

#[test]
fn fn_return_short_circuits() {
	let src = indoc! {"
		five :: fn() int {
			return 5
			10
		}
		five()
	"};
	check(src, "5");
}

#[test]
fn fn_return_bare() {
	let src = indoc! {"
		z :: fn() int { return }
		z()
	"};
	check(src, "0");
}

#[test]
fn named_result() {
	let src = indoc! {r#"
		User :: struct { name: string = "" }
		rename :: fn(who: string) u: User {
			assert!(u.name == "")
			u.name = who
			return
		}
		first :: fn(xs: []int) n: int {
			loop x in xs {
				n = x
				return
			}
			return
		}
		three :: fn() r := 2 {
			r += 1
			return
		}
		print(first([7, 8]))
		print(three())
		rename("me").name
	"#};
	check(src, ["7", "3", "me"]);
}

#[test]
fn default_param() {
	let src = indoc! {"
		add :: fn(x: int, y: int = 10, z := 100) int { x + y + z }
		print(add(1))
		print(add(1, 2, 3))
	"};
	check(src, ["111", "6"]);
}

#[test]
fn default_param_refs_earlier_param() {
	let src = indoc! {r#"
		Wrapper :: struct { s: string }
		Wrapper :< {
			wrap :: fn(self, left: string, right: string = left) string { left + self.s + right }
		}
		Wrapper.{"hi"}.wrap("|")
	"#};
	check(src, "|hi|");
}

#[test]
fn default_param_not_trailing() {
	let src = indoc! {"
		add :: fn(x: int = 1, y: int) int { x + y }
		add(1)
	"};
	fail_with(src, "defaults must be trailing");
}

#[test]
fn named_args() {
	let src = indoc! {r#"
		win :: fn(title: string, x := 0, w := 854, h := 480) string { "{title} {x} {w}x{h}" }
		first[T] :: fn(a: T, b: T) T { a }
		Options :: struct { foo: int, bar: bool }
		opts :: fn(o: Options) { print(o) }
		print(win(w = 640, title = "a"))
		print(win("b", h = 360))
		print(first(b = 2, a = 9))
		opts(bar = true, foo = 4)
	"#};
	check(
		src,
		["a 0 640x480", "b 0 854x360", "9", "Options.{foo = 4, bar = true}"],
	);
}

#[test]
fn named_args_on_fn_value() {
	let src = indoc! {r#"
		win :: fn(title: string, x := 0, w := 854) string { "{title} {x} {w}" }
		apply :: fn(f: fn(n: int) int, x: int) int { f(n = x) }
		f := win
		print(f(w = 640, title = "a"))
		print(f("b"))
		print(apply(fn(n: int) int { n * 3 }, 4))
	"#};
	check(src, ["a 0 640", "b 0 854", "12"]);
}

#[test]
fn redefinition_shadows() {
	let src = indoc! {r#"
		call :: fn() { f() }
		f :: fn() { print("one") }
		f :: fn() { print("two") }
		call()
	"#};
	check(src, "two");
}

#[test]
fn duplicate_definition_reports() {
	let src = indoc! {r"
		def! :: fn() Ast { `t :: fn() int { 7 }` }
		def!()
		def!()
	"};
	fail_with(src, "duplicate fn `t`");
}
