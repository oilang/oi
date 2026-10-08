use crate::helpers::*;

#[test]
fn semicolons() {
	check("x :: 3; y :: x * x; y + x;", "12");
}

#[test]
fn blocks_are_expressions() {
	check(
		["a := { print(3); 5 }", "print(a + { 1 })", "x := 1", "{ x = 7 }", "x"],
		["3", "6", "7"],
	);
}

#[test]
fn assign_yields_the_place() {
	check(
		indoc! {"
			x := 0
			print((x = 3) + 1)
			a := 0
			b := 0
			a = b = 3
			print(a)
			p := .{ x = 1 }
			p.x = 5
		"},
		["4", "3", "5"],
	);
}

#[test]
fn append_chains() {
	check(["xs := [1]", "xs << 2 << 3"], "[1, 2, 3]");
}

#[test]
fn destructure_yields_the_rhs() {
	check(["v := ((a, b) := (1, 2))", "v"], "(1, 2)");
}

#[test]
fn typed_bind_yields_the_place() {
	check(["y := (x : f64 = 3) * 2.0", "y"], "6.0");
}
