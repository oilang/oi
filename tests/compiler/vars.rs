use crate::helpers::*;

#[test]
fn variable() {
	check(["x :: 42", "π :: 3.14", "print(x, π)"], "42 3.14");
}

#[test]
fn assign() {
	check(
		[
			"x := 1",
			"x = 2",
			"y := 10",
			"y = y + 5",
			r#"s := "old""#,
			r#"s = "new""#,
			"print(x, y, s)",
		],
		"2 15 new",
	);
}

#[test]
fn compound_assign() {
	let src = indoc! {"
		a := 10  b := 10  c := 10  d := 10  e := 10  f := 2
		a += 5  b -= 5  c *= 5  d /= 5  e %= 4  f **= 5
		print(a, b, c, d, e, f)
	"};
	check(src, "15 5 50 2 2 32");
}

#[test]
fn nested_place_assign() {
	check(
		indoc! {"
			P :: struct { x: int }
			ps := [P.{1}, P.{2}]
			g := [[1, 2], [3, 4]]
			a := [1, 2, 3]
			i := 0
			at :: fn(mut i: int) int { i += 1  i }
			ps[at(mut i)].x += 10
			g[1][0] = 7
			a[1] += 10
			(ps[1].x, g[1][0], i, a[1])
		"},
		"(12, 7, 1, 12)",
	);
}

#[test]
fn declare_zero() {
	check(
		indoc! {"
			Point :: struct { x: int, y: int }
			n: int
			s: string
			print(n, s.len)
			n = 7
			p: Point
			p.x = 5
			print(n, p.x, p.y)
		"},
		["0 0", "7 5 0"],
	);
}

#[test]
fn annotated_binding() {
	check(
		[
			"a : int : 2",
			r#"b : string : "hi""#,
			"small : i16 : 5_000",
			"f : f32 : 1.5",
			"x : f64 : 5",
			"print(a, b, small, f, x)",
		],
		"2 hi 5000 1.5 5.0",
	);
}

#[test]
fn annotation_type_mismatch() {
	fail(r#"x : int : "hi""#, "expected int, got string");
}

#[test]
fn annotation_out_of_range() {
	fail(["x : i8 : 9999", "x"], "out of range for i8");
}

#[test]
fn main_file_const_visible_in_fn_body() {
	check(
		indoc! {"
			scene :: 2
			f :: fn() int { scene }
			print(f())
		"},
		"2",
	);
}
