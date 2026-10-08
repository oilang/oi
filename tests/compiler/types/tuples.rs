use crate::helpers::*;

#[test]
fn tuple_literals() {
	check(
		indoc! {r#"
			print((1, 2, 3), (true, 2, "lol"), (a = 1, b = 2), (1, b = 2), (1, 2,))
			print((1), (1,), (2 3 4), (1, (2, 3)))
			print(("lisp, innit?" true [2, 4, 5]), ("lisp, innit?" true [2 4 5]))
		"#},
		[
			r#"(1, 2, 3) (true, 2, "lol") (a = 1, b = 2) (1, b = 2) (1, 2)"#,
			"1 (1) (2, 3, 4) (1, (2, 3))",
			r#"("lisp, innit?", true, [2, 4, 5]) ("lisp, innit?", true, [2, 4, 5])"#,
		],
	);
}

#[test]
fn field_access() {
	check(
		[
			"t :: (a = 1, b = 2)",
			"f :: (1.5, 2.5)",
			"p :: (3, 4)",
			"print(t.b, t.a == t.0, f.0, p.0 * p.1)",
		],
		"2 true 1.5 12",
	);
}

#[test]
fn field_rejections() {
	fail(["t :: (1, 2)", "t.5"], "out of range");
	fail(["t :: (a = 1,)", "t.z"], "no field `z`");
	fail(["x :: 5", "x.0"], "cannot access a field");
}

#[test]
fn fn_returns_tuple() {
	let src = indoc! {"
		pair :: fn() (int int) { (3, 4) }
		swap :: fn(x: int, y: int) (int, int) { (y, x) }
		t :: swap(1, 2)
		print(pair(), t.0)
	"};
	check(src, "(3, 4) 2");
}

#[test]
fn fn_return_type_mismatch_tuple() {
	let src = indoc! {"
		bad :: fn() (int, int) { 42 }
		bad()
	"};
	fail(src, "wrong return type");
}

#[test]
fn if_no_else_tuple_zero() {
	let src = indoc! {"
		t :: if false { (1, 2) }
		t
	"};
	check(src, "(0, 0)");
}

#[test]
fn field_names_are_hints() {
	check(
		indoc! {"
			t : (int, int) : (x = 1, y = 2)
			t.0 + t.1
		"},
		"3",
	);
	check(
		indoc! {"
			f :: fn(t: (int, int)) int { t.0 }
			f((x = 7, y = 8))
		"},
		"7",
	);
}

#[test]
fn named_type_annotation() {
	check(
		indoc! {"
			dimensions :: fn() (width: int, height: int) { (1920, 1080) }
			d :: dimensions()
			print(d)
			d.width + d.0
		"},
		["(width = 1920, height = 1080)", "3840"],
	);
	check(
		indoc! {"
			d : (int, int) = (w = 1, h = 2)
			d
		"},
		"(1, 2)",
	);
}

#[test]
fn array_slot_is_independent_copy() {
	check(
		indoc! {"
			a := [1]
			t :: (a, 0)
			a << 2
			t.0
		"},
		"[1]",
	);
}

#[test]
fn elements_take_the_expected_type() {
	check(
		indoc! {"
			F :: struct { x: int }
			f :: fn(p: (F, int)) { print(p.0.x + p.1) }
			f((.{ x = 1 }, 2))
			p : (F, int) = (.{ x = 3 }, 4)
			p.0.x + p.1
		"},
		["3", "7"],
	);
}

#[test]
fn equality_is_structural() {
	check(
		indoc! {"
			P :: struct { x: int }
			print((1, (2, 3)) == (1, (2, 3)))
			print((1, P.{2}) == (1, P.{3}))
		"},
		["true", "false"],
	);
}

#[test]
fn field_assign() {
	check(
		indoc! {"
			t := (1, b = 2)
			t.0 += 10
			t.b = 20
			t
		"},
		"(11, b = 20)",
	);
	fail("t := (1, 2); t.z = 3", "tuple has no field `z`");
}

#[test]
fn alias_beside_main() {
	check(
		indoc! {r#"
			T :: (int, string, float)
			main :: fn() {
				t :: T.(2 "ciea" 2)
				print(t)
			}
		"#},
		r#"(2, "ciea", 2.0)"#,
	);
}

#[test]
fn dot_tuple_casts() {
	check(
		indoc! {r#"
			first :: fn(t: (int, string)) int { t.0 }
			pair :: fn() (int, float) { .(7, 2) }
			x : ?int = .(5)
			print(first(.(3, "c")), pair(), (int, string).(1, "a"), x)
		"#},
		r#"3 (7, 2.0) (1, "a") some.(5)"#,
	);
}
