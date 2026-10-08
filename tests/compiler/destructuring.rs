use crate::helpers::*;

#[test]
fn tuple_bind() {
	let src = indoc! {r#"
		pair :: fn() (int, int) { (10, 20) }
		(foo, bar) :: ("food", "bard")
		(c, d) :: pair()
		(a, b) := (1, 2)
		a = a + b
		print(foo, bar, c + d, a)
		(a, b) = (b, a)
		print(a)
		(e, _) :: (1, 2)
		[_ f] :: [1 2]
		print(e, f)
		(a, b)
	"#};
	check(src, ["food bard 30 3", "2", "1 2", "(2, 3)"]);
}

#[test]
fn fail_tuple_bind() {
	fail("(a, b, c) :: (1, 2)", "fields");
	fail("(a, b) :: 5", "cannot destructure");
	fail("(a, b) :: (1, 2)\n(a, b) = (3, 4)", "immutably bound");
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
fn struct_bind() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point.{ x, y = b } :: Point.{ 1, 2 }
		Point.{ y = _, x = c } :: Point.{ 1, 2 }
		x + b + c
	"};
	check(src, "4");
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
