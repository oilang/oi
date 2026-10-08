use crate::helpers::*;
use indoc::indoc;

#[test]
fn inference() {
	let src = indoc! {"
		Pair[T] :: struct { a: T, b: T }
		Box[T] :: struct { v: T }
		sum :: fn(p: Pair[int]) int { p.a + p.b }
		wrap[T] :: fn(v: T) Box[T] { Box.{ v = v } }
		p :: Pair.{ a = 3, b = 4 }
		print(p.a + p.b, Box.{ v = Box.{ v = 5 } }.v.v, sum(Pair.{ a = 3, b = 4 }), wrap(9).v)
	"};
	check(src, "7 5 7 9");
}

#[test]
fn annotation_and_head() {
	check(
		indoc! {"
			Box[T] :: struct { v: T }
			Pair[A, B] :: struct { a: A, b: B }
			b : Box[int] : Box.{}
			p : Pair[int, string] : Pair.{ a = 7 }
			h :: Box[int].{ v = 7 }
			print(b.v, p.a, h.v)
		"},
		"0 7 7",
	);
}

#[test]
fn rejections() {
	fail(
		["Pair[T] :: struct { a: T, b: T }", r#"Pair.{ a = 3, b = "x" }"#],
		"bound to both",
	);
	fail(["Pair[T] :: struct { a: T, b: T }", "Pair.{}"], "cannot infer");
	fail(
		["Pair[T] :: struct { a: T, b: T }", "f :: fn(p: Pair) int { p.a }", "0"],
		"needs type arguments",
	);
	fail(
		[
			"Tagged[T] :: struct { v: T, id: int }",
			r#"Tagged.{ v = 1.5, id = "x" }"#,
		],
		"expected int",
	);
	fail(
		[
			"Point :: struct { x: int, y: int }",
			"f :: fn(p: Point[int]) int { p.x }",
			"0",
		],
		"is not generic",
	);
	fail(
		["Box[T] :: struct { v: T }", "Box[string].{ v = 7 }"],
		"expected string, got int",
	);
}

#[test]
fn tuple_args_make_distinct_instances() {
	let src = indoc! {r#"
		Box[T] :: struct { v: T }
		unwrap[T] :: fn(b: Box[T]) T { b.v }
		a := Box.{ v = (1, "a") }
		b := Box.{ v = (true, false) }
		print("{unwrap(a)}")
		print("{unwrap(b)}")
	"#};
	check(src, [r#"(1, "a")"#, "(true, false)"]);
}

#[test]
fn generic_tuple_struct() {
	let src = indoc! {r#"
		B[T] :: struct (T)
		b :: B[int](3)
		c : B[string] = .("x")
		print(b, c)
	"#};
	check(src, r#"B[int](3) B[string]("x")"#);
}
