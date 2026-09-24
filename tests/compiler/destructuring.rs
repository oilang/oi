use crate::helpers::*;

#[test]
fn bind_basic() {
	check(r#"(foo, bar) :: ("food", "bard"); foo"#, "food");
	check(r#"(foo, bar) :: ("food", "bard"); bar"#, "bard");
}

#[test]
fn bind_from_fn() {
	let src = indoc! {"
		pair :: fn() (int, int) { (10, 20) }
		(a, b) :: pair()
		a + b
	"};
	check(src, "30");
}

#[test]
fn bind_mut_reassigned() {
	let src = indoc! {"
		(a, b) := (1, 2)
		a = a + b
		a
	"};
	check(src, "3");
}

#[test]
fn swap() {
	let src = indoc! {"
		(a, b) := (1, 2)
		(a, b) = (b, a)
		a
	"};
	check(src, "2");
}

#[test]
fn loose_commas() {
	let src = indoc! {r#"
		get_coords :: fn() (int, int) { (7, 2) }
		(lat long) :: get_coords()
		(a, b) := (lat long)
		(a b) = (b a)
		loop (x y) in [(1 2)] { print(x + y) }
		match (a b) { (l r) => print(l - r), }
	"#};
	check(src, "3\n-5");
}

#[test]
fn bare_tuple_still_expr() {
	check("(a, b) :: (1, 2)\n(a, b)", "(1, 2)");
}

#[test]
fn fail_arity_mismatch() {
	fail("(a, b, c) :: (1, 2)", "fields");
}

#[test]
fn fail_non_tuple() {
	fail("(a, b) :: 5", "cannot destructure");
}

#[test]
fn fail_assign_immutable() {
	fail("(a, b) :: (1, 2)\n(a, b) = (3, 4)", "immutably bound");
}

#[test]
fn struct_bind() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point.{ x, y = b } :: Point.{ 1, 2 }
		x + b
	"};
	check(src, "3");
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point.{ x } := Point.{ 1, 2 }
		x = x + 1
		x
	"};
	check(src, "2");
}

#[test]
fn fail_struct_bind() {
	fail("Point :: struct { x: int }\nPoint.{ z } :: Point.{ 1 }", "no field `z`");
	fail("Point :: struct { x: int }\nPoint.{ x } :: 5", "cannot destructure");
}

#[test]
fn array_bind() {
	let src = indoc! {"
		[a b] :: [1 2 3]
		a + b
	"};
	check(src, "3");
	let src = indoc! {"
		[x] := int.[7]
		x = x + 1
		x
	"};
	check(src, "8");
}

#[test]
fn skip_underscore() {
	check("(a, _) :: (1, 2)\na", "1");
	check("[_ b] :: [1 2]\nb", "2");
	check(
		"Point :: struct { x: int, y: int }\nPoint.{ y = _, x } :: Point.{ 1, 2 }\nx",
		"1",
	);
}

#[test]
fn assign_through_pattern() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		x := 0
		Point.{ x } = Point.{ 1, 2 }
		x
	"};
	check(src, "1");
}

#[test]
fn fail_array_bind() {
	fail_rt("[a b] :: [1]", "out of range");
	fail("[a b] :: 5", "cannot destructure");
}
