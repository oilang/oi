use crate::helpers::*;

#[test]
fn variable() {
	check(["x :: 42", "x"], "42");
}

#[test]
fn unicode_idents() {
	check(["π :: 3.14", "π"], "3.14");
}

#[test]
fn assign() {
	check(["x := 1", "x = 2", "x"], "2");
}

#[test]
fn assign_from_self() {
	check(["x := 10", "x = x + 5", "x"], "15");
}

#[test]
fn assign_string() {
	check(r#"s := "old"; s = "new"; s"#, "new");
}

#[test]
fn compound_assign() {
	check(["x := 10", "x += 5", "x"], "15");
	check(["x := 10", "x -= 5", "x"], "5");
	check(["x := 10", "x *= 5", "x"], "50");
	check(["x := 10", "x /= 5", "x"], "2");
	check(["x := 10", "x %= 4", "x"], "2");
	check(["x := 2", "x **= 5", "x"], "32");
}

#[test]
fn compound_assign_index() {
	check(["a := [1, 2, 3]", "a[1] += 10", "a[1]"], "12");
}

#[test]
fn compound_assign_field() {
	check(
		indoc! {"
			Point :: struct { x: int, y: int }
			p := Point.{ x = 1, y = 2 }
			p.x += 10
			p.x
		"},
		"11",
	);
}

#[test]
fn declare_zero_int() {
	check(["n: int", "n"], "0");
}

#[test]
fn declare_zero_string() {
	check(["s: string", "s"], "");
}

#[test]
fn declare_zero_then_assign() {
	check(["n: int", "n = 7", "n"], "7");
}

#[test]
fn declare_zero_struct() {
	check(
		indoc! {"
			Point :: struct { x: int, y: int }
			p: Point
			p.x = 5
			p.x
		"},
		"5",
	);
}

#[test]
fn annotated_binding() {
	check(["a : int : 2", "a"], "2");
	check(r#"b : string : "hi"; b"#, "hi");
}

#[test]
fn annotation_type_mismatch() {
	fail_with(r#"x : int : "hi""#, "expected int, got string");
}

#[test]
fn annotation_pins_width() {
	// the literal defaults to int, but the annotation narrows it to i16
	check(["small : i16 : 5_000", "small"], "5000");
}

#[test]
fn annotation_coerces_float() {
	check(["f : f32 : 1.5", "f"], "1.5");
	check(["x : f64 : 5", "x"], "5.0");
}

#[test]
fn annotation_out_of_range() {
	fail_with(["x : i8 : 9999", "x"], "out of range for i8");
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
