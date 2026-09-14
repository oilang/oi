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
	"};
	check(src, ["0", "2", "4", "3", "4"]);
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
