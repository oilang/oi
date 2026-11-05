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
		match n := 15 { 0..18 => n, _ => 0 }
	"#};
	check(src, ["adult", "15"]);
}

#[test]
fn match_destructure() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		print(match (3, 4) { (x, y) => x + y, })
		print(match Point.{ x = 3, y = 4 } { Point.{ y = b, x } => x + b, })
		print(match Point.{ x = 3, y = 4 } { .{ y = b, x } => x * b, })
		print(match [3, 4] { [x, y] => x + y, })
		match [1, 2, 3] {
			[x, y] => 0,
			_ => 99,
		}
	"#};
	check(src, ["7", "7", "12", "7", "99"]);
}

#[test]
fn match_sub_patterns() {
	let src = indoc! {r#"
		print(match 1, 0 {
			(0, 0) => "both",
			(_, 0) => "b",
			_ => "none"
		})
		o: ?(int, int) = .some.((7, 2))
		match o {
			.some.((0, y)) => print(y),
			.some.((x, 1..3)) => print("near", x),
			_ => print("other")
		}
	"#};
	check(src, ["b", "near 7"]);
	fail(
		"n: ?int = .none match n { .some.(0) => 1, .none => 2 }",
		"missing: some",
	);

	let src = indoc! {r#"
		User :: struct { name: string, age: int }
		greet :: fn(u: User) string do match u {
			User.{ age = 0..18 } => "minor: {u.name}",
			_ => "adult"
		}
		print(greet(User.{ name = "kid", age = 9 }), greet(User.{ name = "old", age = 40 }))
		Shape :: enum {
			circle { radius: int }
			point
		}
		match Shape.circle.{ radius = 0 } {
			.circle.{ radius = 0 } => "dot",
			.circle.{ radius } => "{radius}",
			.point => ""
		}
	"#};
	check(src, ["minor: kid adult", "dot"]);
}

#[test]
fn match_errors() {
	fail(r#"match "s" { 0..5 => 1, _ => 2 }"#, "integer subject");
	fail("match (1, 2) { (a, b, c) => a, }", "pattern binds 3 names");
	fail(
		["x: ?int = 1", "match .some.(n) := x { _ => print(1) }"],
		"a `match` header can't refute",
	);
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

#[test]
fn fallthrough() {
	let src = indoc! {r#"
		f :: fn(n: int) {
			match n {
				1 => { print("one"); fallthrough },
				2 => print("two"),
				3 => { print("three"); fallthrough },
				else => print("other"),
			}
		}
		f(1)
		f(2)
		f(3)
	"#};
	check(src, ["one", "two", "two", "three", "other"]);

	fail("match 1 { 1 => { fallthrough; 2 } _ => 3 }", "last statement");
	fail("match 1 { 1 => fallthrough }", "last arm");
	fail("match (1, 2) { _ => fallthrough, (a, b) => a }", "binds names");
}

#[test]
fn match_bind_header() {
	let src = indoc! {"
		count :: fn() int { 3 }
		match n := count() { 0 => print(0), else => print(n + 1) }
		match n := 0 { 0 => print(n), else => () }
	"};
	check(src, ["4", "0"]);
	fail(["match n := 2 { _ => () }", "print(n)"], "undefined variable `n`");
}

#[test]
fn match_arm_guard() {
	let src = indoc! {r#"
		loop i in 2..5 {
			x: ?int = i
			print(match x { .some.(n) if n > 3 => "big", .some.(n) => "small {n}", .none => "none" })
		}
	"#};
	check(src, ["small 2", "small 3", "big"]);
	fail(
		r#"x: ?int = 4 match x { .some.(n) if n > 3 => (), .none => () }"#,
		"missing: some",
	);
}

#[test]
fn match_comma_subject_is_tuple() {
	let src = indoc! {r#"
		a :: 3
		b :: 4
		match a, b {
			(x, y) => x * y,
		}
	"#};
	check(src, "12");
}

#[test]
fn match_without_subject() {
	let src = indoc! {r#"
		x :: 7
		match {
			x < 5 => "small",
			x < 10 => "medium",
			else => "large",
		}
	"#};
	check(src, "medium");
}

#[test]
fn match_nested_binds_cover_variants() {
	let src = indoc! {r#"
		P :: struct { x: (int, int), y: int }
		pair: ?(int, int) = .some.((1, 2))
		print(match pair {
			.some.((x, y)) => x + y,
			.none => 0
		})
		point: ?P = .some.(P.{ x = (3, 4), y = 5 })
		print(match point {
			.some.(P.{ x = (a, b), y }) => a + b + y,
			.none => 0
		})
	"#};
	check(src, ["3", "12"]);
}
