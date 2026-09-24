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
