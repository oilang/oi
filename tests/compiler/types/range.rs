use crate::helpers::*;
use indoc::indoc;

#[test]
fn ranges() {
	let src = indoc! {r#"
		grade :: fn(n: int) string { match n { ..18 => "minor", 18..=64 => "adult", 65.. => "elder" } }
		xs :: [1, 2, 3, 4]
		print(1..3, [..3], [..xs, 5])
		print(grade(2), grade(64), grade(65))
		print(xs[1..=2], xs[2..], xs[..])
		loop i in 2..=3 { print(i) }
	"#};
	check(
		src,
		[
			"1..3 [0..3] [1, 2, 3, 4, 5]",
			"minor adult elder",
			"[2, 3] [3, 4] [1, 2, 3, 4]",
			"2",
			"3",
		],
	);
}

#[test]
fn stepped_ranges() {
	let src = indoc! {"
		loop i in 0..2..=4 { print(i) }
		loop i in 3.. {
			if i == 5 { break }
			print(i)
		}
		loop i in 9..7.. {
			if i < 5 { break }
			print(i)
		}
	"};
	check(src, ["0", "2", "4", "3", "4", "9", "7", "5"]);
}

#[test]
fn stepped_pattern_and_membership() {
	check(
		[
			"print(4 in 0..2..10, 5 in 0..2..10)",
			r#"match 4 { 0..2..=4 => print("hit") }"#,
		],
		["true false", "hit"],
	);
}

#[test]
fn spreads_into_an_array() {
	check("[..(0..3), 7]", "[0, 1, 2, 7]");
	check("[..(10..8..0)]", "[10, 8, 6, 4, 2]");
}

#[test]
fn spreads_into_a_vararg() {
	let src = indoc! {"
		sum :: fn(xs: ..int) int {
			t := 0
			loop x in xs { t += x }
			t
		}
		print(sum(..(1..=4)))
	"};
	check(src, "10");
}

#[test]
fn subscripts_with_a_range_value() {
	check(
		["xs :: [1, 2, 3, 4]", "r := 1..3", "print(xs[r], xs[1..])"],
		"[2, 3] [2, 3, 4]",
	);
}

#[test]
fn spread_rejections() {
	fail_rt("[..(3..)]", "open range");
	fail_rt(["xs :: [1, 2, 3, 4]", "r := 0..2..4", "print(xs[r])"], "strided");
}
