use crate::helpers::*;
use indoc::indoc;

#[test]
fn instantiations() {
	let src = indoc! {"
		max[T] :: fn(a: T, b: T) T {
			if a > b { a } else { b }
		}
		fact[T] :: fn(n: T) T {
			if n <= 1 { 1 } else { n * fact(n - 1) }
		}
		is_even[T] :: fn(n: T) bool {
			if n == 0 { true } else { is_odd(n - 1) }
		}
		is_odd[T] :: fn(n: T) bool {
			if n == 0 { false } else { is_even(n - 1) }
		}
		print(max(3, 7), max(3.5, 1.2), max[int](3, 7), fact(5), is_even(10))
	"};
	check(src, "7 3.5 7 120 true");
}

#[test]
fn first_of_array() {
	let src = indoc! {"
		first[T] :: fn(xs: []T) ?T {
			if xs.len == 0 { ?T.(none) } else { ?T.(xs[0]) }
		}
		first([1, 2, 3])
	"};
	check(src, "some.(1)");
}

#[test]
fn omitted_return_type_is_unit() {
	let src = indoc! {r#"
		show[T] :: fn(x: T) { print("{x}") }
		show(1)
		show(2.5)
		show((1, "a"))
		show((true, false))
	"#};
	check(src, ["1", "2.5", r#"(1, "a")"#, "(true, false)"]);
}

#[test]
fn rejections() {
	fail(
		indoc! {r#"
			max[T] :: fn(a: T, b: T) T { if a > b { a } else { b } }
			max(1, "a")
		"#},
		"bound to both",
	);
	fail(["noret[T] :: fn(x: T) { x }", "noret(1)"], "expected ()");
	fail(
		[
			"max[T] :: fn(a: T, b: T) T { if a > b { a } else { b } }",
			"max[int, string](3, 7)",
		],
		"expects 1 type argument",
	);
	fail(
		["add :: fn(a: int, b: int) int { a + b }", "add[int](3, 7)"],
		"is not generic",
	);
}

#[test]
fn explicit_and_default_args() {
	let src = indoc! {r#"
		none_of[T] :: fn() ?T { ?T.(none) }
		dnone_of[T = int] :: fn() ?T { ?T.(none) }
		id[T = int] :: fn(x: T) T { x }
		print(none_of[int](), dnone_of(), id("a"), dnone_of[string]())
	"#};
	check(src, "none none a none");
}

#[test]
fn bounds() {
	let src = indoc! {"
		biggest[T: Ord] :: fn(a: T, b: T) T {
			if a > b { a } else { b }
		}
		biggest(3, 7)
	"};
	check(src, "7");
	check(["Ord :: trait {}", "int :< Ord", src], "7");
	fail(["Ord :: trait {}", src], "does not claim");
	fail(src.replace("Ord", "Odr").as_str(), "unknown trait");
}

#[test]
fn static_call_through_a_bound() {
	let src = indoc! {r#"
		Maker :: trait {
			make: fn() Self
			tag :: fn() string { "made" }
		}
		A :: struct { n: int }
		A : Maker < { make :: fn() Self { .{ n = 1 } } }
		build[T: Maker] :: fn() T { print T.tag(); T.make() }
		print(build[A]().n)
	"#};
	check(src, ["made", "1"]);
}

#[test]
fn static_call_on_an_array_bound_param() {
	let src = indoc! {r#"
		empty[T] :: fn(xs: T) bool { T.is_empty(xs) }
		print(empty([1, 2]))
	"#};
	check(src, "false");
}
