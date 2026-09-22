use crate::helpers::*;

#[test]
fn match_string() {
	let src = indoc! {r#"
		os :: "linux"
		match os {
			"darwin" => "macOS",
			"linux" => "Linux",
			else => "other",
		}
	"#};
	check(src, "Linux");
}

#[test]
fn match_else_taken() {
	let src = indoc! {r#"
		x :: 99
		match x {
			1 => "one",
			else => "other",
		}
	"#};
	check(src, "other");
}

#[test]
fn match_no_else_hit() {
	let src = indoc! {r#"
		match "linux" {
			"linux" => "penguin",
		}
	"#};
	check(src, "penguin");
}

#[test]
fn match_no_else_miss_int() {
	check("match 5 { 1 => 10, }", "0");
}

#[test]
fn match_no_else_miss_string() {
	check(r#"match "x" { "y" => "yes", }"#, "");
}

#[test]
fn match_as_binding() {
	let src = indoc! {r#"
		label :: match 2 {
			1 => "one",
			2 => "two",
			else => "other",
		}
		label
	"#};
	check(src, "two");
}

#[test]
fn match_or_patterns() {
	let src = indoc! {r#"
		os :: "macos"
		match os {
			"darwin", "macos" => "Apple",
			"linux" => "Linux",
			else => "other",
		}
	"#};
	check(src, "Apple");
}

#[test]
fn match_or_patterns_second() {
	let src = indoc! {r#"
		os :: "darwin"
		match os {
			"darwin", "macos" => "Apple",
			else => "other",
		}
	"#};
	check(src, "Apple");
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
		match age {
			0..18 => "minor",
			18..65 => "adult",
			_ => "senior",
		}
	"#};
	check(src, "adult");
}

#[test]
fn match_binding() {
	check(r#"match 15 { n @ 0..18 => n, _ => 0 }"#, "15");
}

#[test]
fn match_range_needs_int_subject() {
	fail_with(r#"match "s" { 0..5 => 1, _ => 2 }"#, "integer subject");
}

#[test]
fn match_tuple_destructure() {
	check(["p :: (3, 4)", "match p { (x, y) => x + y, }"], "7");
}

#[test]
fn match_tuple_arity_mismatch() {
	fail_with("match (1, 2) { (a, b, c) => a, }", "pattern binds 3 names");
}

#[test]
fn match_struct_destructure() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		p :: Point.{ x = 3, y = 4 }
		match p {
			Point.{ y = b, x } => x + b,
		}
	"#};
	check(src, "7");
}

#[test]
fn match_struct_unknown_field() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		match Point.{ x = 1, y = 2 } {
			Point.{ z } => z,
		}
	"#};
	fail_with(src, "no field `z`");
}

#[test]
fn match_array_destructure() {
	let src = indoc! {r#"
		a :: [3, 4]
		match a {
			[x, y] => x + y,
		}
	"#};
	check(src, "7");
}

#[test]
fn match_array_length_guard() {
	let src = indoc! {r#"
		a :: [1, 2, 3]
		match a {
			[x, y] => 0,
			_ => 99,
		}
	"#};
	check(src, "99");
}

#[test]
fn match_enum_non_exhaustive() {
	let src = indoc! {r#"
		Color :: enum { red green blue }
		c :: Color.red
		match c {
			.red => 1,
			.green => 2,
		}
	"#};
	fail_with(src, "non-exhaustive match, missing: blue");
}

#[test]
fn match_mismatched_arm_types() {
	fail_with(r#"match 1 { 1 => "str", else => 2 }"#, "mismatched types");
}

#[test]
fn match_pattern_type_mismatch() {
	fail_with(r#"match 1 { "str" => 1, }"#, "type mismatch");
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
