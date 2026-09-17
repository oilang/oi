use crate::helpers::*;

#[test]
fn stmts() {
	let src = indoc! {"
		x :: 3
		y :: x * x
		z :: y + x
		z
	"};
	check(src, "12");
}

#[test]
fn semicolons_join_lines() {
	check("x :: 3; y :: x * x; y + x", "12");
}

#[test]
fn semicolon_terminator() {
	check("1 + 1;", "2");
}

#[test]
fn trailing_assign_yields_the_place() {
	check(["x := 1", "5", "x = 3"], "3");
}

#[test]
fn assign_in_expression_position() {
	check(["x := 0", "n := (x = 3) + 1", "n"], "4");
}

#[test]
fn field_read_back() {
	let src = indoc! {"
		p := .{ x = 1 }
		p.x = 5
	"};
	check(src, "5");
}

#[test]
fn append_chains() {
	check(["xs := [1]", "xs << 2 << 3"], "[1, 2, 3]");
}

#[test]
fn assign_chains_right_assoc() {
	check(["a := 0", "b := 0", "a = b = 3", "a"], "3");
}

#[test]
fn destructure_yields_the_rhs() {
	check(["v := ((a, b) := (1, 2))", "v"], "(1, 2)");
}

#[test]
fn typed_bind_yields_the_place() {
	check(["y := (x : f64 = 3) * 2.0", "y"], "6.0");
}
