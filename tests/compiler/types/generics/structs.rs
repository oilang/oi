use crate::helpers::*;
use indoc::indoc;

#[test]
fn infer_from_literal() {
	let src = indoc! {"
		Pair[T] :: struct { a: T, b: T }
		p :: Pair.{ a = 3, b = 4 }
		p.a + p.b
	"};
	check(src, "7");
}

#[test]
fn nested_instantiation() {
	let src = indoc! {"
		Box[T] :: struct { v: T }
		Box.{ v = Box.{ v = 5 } }.v.v
	"};
	check(src, "5");
}

#[test]
fn type_position_param() {
	let src = indoc! {"
		Pair[T] :: struct { a: T, b: T }
		sum :: fn(p: Pair[int]) int { p.a + p.b }
		sum(Pair.{ a = 3, b = 4 })
	"};
	check(src, "7");
}

#[test]
fn conflicting_field_types_error() {
	fail(
		indoc! {r#"
			Pair[T] :: struct { a: T, b: T }
			Pair.{ a = 3, b = "x" }
		"#},
		"bound to both",
	);
}

#[test]
fn cannot_infer_error() {
	fail(
		indoc! {"
			Pair[T] :: struct { a: T, b: T }
			Pair.{}
		"},
		"cannot infer",
	);
}

#[test]
fn empty_lit_infers_from_annotation() {
	check(
		indoc! {"
			Box[T] :: struct { v: T }
			b : Box[int] : Box.{}
			b.v
		"},
		"0",
	);
}

#[test]
fn partial_lit_infers_from_annotation() {
	check(
		indoc! {r#"
			Pair[A, B] :: struct { a: A, b: B }
			p : Pair[int, string] : Pair.{ a = 7 }
			p.a
		"#},
		"7",
	);
}

#[test]
fn bare_name_needs_type_arguments() {
	fail(
		indoc! {"
			Pair[T] :: struct { a: T, b: T }
			f :: fn(p: Pair) int { p.a }
			0
		"},
		"needs type arguments",
	);
}

#[test]
fn generic_fn_round_trip() {
	let src = indoc! {"
		Box[T] :: struct { v: T }
		wrap[T] :: fn(v: T) Box[T] { Box.{ v = v } }
		wrap(9).v
	"};
	check(src, "9");
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
fn concrete_field_type_still_checked() {
	fail(
		indoc! {r#"
			Tagged[T] :: struct { v: T, id: int }
			Tagged.{ v = 1.5, id = "x" }
		"#},
		"expected int",
	);
}

#[test]
fn type_args_on_non_generic_struct_error() {
	fail(
		indoc! {"
			Point :: struct { x: int, y: int }
			f :: fn(p: Point[int]) int { p.x }
			0
		"},
		"is not generic",
	);
}

#[test]
fn explicit_instance_head() {
	check(
		indoc! {"
			Box[T] :: struct { v: T }
			b :: Box[int].{ v = 7 }
			b.v
		"},
		"7",
	);
	fail(
		indoc! {"
			Box[T] :: struct { v: T }
			Box[string].{ v = 7 }
		"},
		"expected string, got int",
	);
}
