use crate::helpers::*;

#[test]
fn match_literals() {
	let src = indoc! {r#"
		os :: fn(s: string) string {
			match s {
				"darwin", "macos" => "Apple",
				"linux" => "Linux",
				else => "other",
			}
		}
		print(os("linux"))
		print(os("macos"))
		print(os("darwin"))
		print(os("plan9"))
		label :: match 2 {
			1 => "one",
			2 => "two",
			else => "other",
		}
		print(label)
		match "linux" {
			"linux" => "penguin",
		}
	"#};
	check(src, ["Linux", "Apple", "Apple", "other", "two", "penguin"]);
}

#[test]
fn match_no_else_miss() {
	check("match 5 { 1 => 10, }", "0");
	check(r#"match "x" { "y" => "yes", }"#, "");
}

#[test]
fn match_true_as_if_chain() {
	let src = indoc! {r#"
		x :: 7
		match true {
			x < 5 => "small",
			x < 10 => "medium",
			else => "large",
		}
	"#};
	check(src, "medium");
}

#[test]
fn match_wildcard() {
	check(r#"match 5 { 1 => "one", _ => "other" }"#, "other");
	let src = indoc! {r#"
		Color :: enum { red green blue }
		match Color.blue {
			.red => 1,
			_ => 9,
		}
	"#};
	check(src, "9");
}

#[test]
fn match_range() {
	let src = indoc! {r#"
		age :: 18
		print(match age {
			0..18 => "minor",
			18..65 => "adult",
			_ => "senior",
		})
		match 15 { n @ 0..18 => n, _ => 0 }
	"#};
	check(src, ["adult", "15"]);
}

#[test]
fn match_destructure() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		print(match (3, 4) { (x, y) => x + y, })
		print(match Point.{ x = 3, y = 4 } { Point.{ y = b, x } => x + b, })
		print(match [3, 4] { [x, y] => x + y, })
		match [1, 2, 3] {
			[x, y] => 0,
			_ => 99,
		}
	"#};
	check(src, ["7", "7", "7", "99"]);
}

#[test]
fn match_errors() {
	fail(r#"match "s" { 0..5 => 1, _ => 2 }"#, "integer subject");
	fail("match (1, 2) { (a, b, c) => a, }", "pattern binds 3 names");
	fail(
		[
			"Point :: struct { x: int, y: int }",
			"match Point.{ x = 1, y = 2 } { Point.{ z } => z, }",
		],
		"no field `z`",
	);
	fail(
		[
			"Color :: enum { red green blue }",
			"match Color.red { .red => 1, .green => 2, }",
		],
		"non-exhaustive match, missing: blue",
	);
	fail(r#"match 1 { 1 => "str", else => 2 }"#, "mismatched types");
	fail(r#"match 1 { "str" => 1, }"#, "type mismatch");
}

#[test]
fn match_arm_chain_across_lines() {
	let src = indoc! {"
		C :: struct { r: int }
		C :< { area :: fn(self) int { self.r * self.r } }
		c :: C.{r = 3}
		match true {
			true => c
				.area()
			else => 0
		}
	"};
	check(src, "9");
}

#[test]
fn payload_bind_is_independent_copy() {
	let src = indoc! {"
		Box :: enum { empty has([]int) }
		b :: Box.has.([1])
		v := match b {
			.has.(x) => x,
			.empty => [0],
		}
		v << 99
		match b {
			.has.(x) => x,
			.empty => [0],
		}
	"};
	check(src, "[1]");
}

#[test]
fn bare_name_binds_propagator_payloads() {
	let src = indoc! {"
		o :: ?int.(42)
		match o { n => n, else => -1 }
	"};
	check(src, "42");
	check(["o :: ?int.(none)", "match o { n => n, else => -1 }"], "-1");

	let src = indoc! {"
		r :: !int.(42)
		match r { v => v, else => -1 }
	"};
	check(src, "42");
}
