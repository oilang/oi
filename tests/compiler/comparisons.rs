use crate::helpers::*;

#[test]
fn compares() {
	check(
		indoc! {"
			print(2 == 2, 2 != 3, 2 < 3, 2 > 3, 3 <= 3, 2 >= 3)
			print(1.5 < 2.0, true == true, 1.0 == 1, 1 == 1.0, 1 < 2.0)
			print(1 + 2 < 2 + 2, 1 < 2 == 3 < 4)
		"},
		[
			"true true true false true false",
			"true true true true true",
			"true true",
		],
	);
}

#[test]
fn mismatched_types() {
	fail(r#"1 < "x""#, "cannot compare");
}

#[test]
fn option_against_none_skips_payload_eq() {
	check(
		indoc! {"
			J :: :null | int | []J
			B :: struct { j: J }
			o: ?B = none
			p: ?B = B.{j = 1}
			print(o == none, o != none, p == none, p != none)
		"},
		["true false false true"],
	);
}
