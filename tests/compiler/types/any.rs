use crate::helpers::*;

#[test]
fn coerce_and_match() {
	check(
		indoc! {r#"
			xs: []any = [1, "two"]
			loop x in xs {
				match x {
					n @ int => print(n + 1),
					s @ string => print(s),
					else => print("?"),
				}
			}
		"#},
		["2", "two"],
	);
}

#[test]
fn zero_value_falls_to_else() {
	check(
		indoc! {"
			x: any
			match x {
				n @ int => n,
				else => -1,
			}
		"},
		"-1",
	);
}

#[test]
fn needs_else() {
	fail(
		indoc! {"
			x: any = 7
			match x { n @ int => n }
		"},
		"needs `else`",
	);
}

#[test]
fn assertion_casts() {
	check(["x: any = 7", "int.(x)"], "some.(7)");
	check(["x: any = 7", "string.(x)"], "none");
}

#[test]
fn print_dispatches_on_typeid() {
	check(
		indoc! {r#"
			P :: struct { x: int }
			xs: []any = [3, "hi", P.{ x = 4 }]
			loop x in xs { print(x) }
			y: any
			print(y)
		"#},
		["3", "hi", "P.{x = 4}", "<any>"],
	);
}

#[test]
fn eq_dispatches_on_typeid() {
	check(
		indoc! {r#"
			P :: struct { x: int }
			a: any = P.{ x = 1 }
			b: any = P.{ x = 1 }
			c: any = "1"
			z: any
			print(a == b, a == c, z == z)
		"#},
		"true false true",
	);
}
