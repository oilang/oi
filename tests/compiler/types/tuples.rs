use crate::helpers::*;

#[test]
fn tuple_literal() {
	check("(1, 2, 3)", "(1, 2, 3)");
}

#[test]
fn tuple_mixed_types() {
	check(r#"(true, 2, "lol")"#, r#"(true, 2, "lol")"#);
}

#[test]
fn tuple_named() {
	check("(a = 1, b = 2)", "(a = 1, b = 2)");
}

#[test]
fn tuple_partially_named() {
	check("(1, b = 2)", "(1, b = 2)");
}

#[test]
fn tuple_trailing_comma() {
	check("(1, 2,)", "(1, 2)");
}

#[test]
fn one_tuple_needs_comma() {
	check("(1)", "1");
	check("(1,)", "(1)");
}

#[test]
fn no_comma_ints() {
	check("(2 3 4)", "(2, 3, 4)");
}

#[test]
fn no_comma_mixed_literals() {
	check(
		r#"("lisp, innit?" true [2, 4, 5])"#,
		r#"("lisp, innit?", true, [2, 4, 5])"#,
	);
}

#[test]
fn no_comma_nested_array_no_comma() {
	check(
		r#"("lisp, innit?" true [2 4 5])"#,
		r#"("lisp, innit?", true, [2, 4, 5])"#,
	);
}

#[test]
fn nested_tuple() {
	check("(1, (2, 3))", "(1, (2, 3))");
}

#[test]
fn field_by_index() {
	check(["t :: (10, 20)", "t.1"], "20");
}

#[test]
fn field_by_name() {
	check(["t :: (a = 1, b = 2)", "t.b"], "2");
}

#[test]
fn named_and_positional_agree() {
	check("t :: (a = 1, b = 2); t.a == t.0", "true");
}

#[test]
fn field_float_load() {
	check(["t :: (1.5, 2.5)", "t.0"], "1.5");
}

#[test]
fn field_arithmetic() {
	check(["t :: (3, 4)", "t.0 * t.1"], "12");
}

#[test]
fn tuple_in_var_prints() {
	check(r#"t :: (1, "two", 3.0); t"#, r#"(1, "two", 3.0)"#);
}

#[test]
fn index_out_of_range() {
	fail(["t :: (1, 2)", "t.5"], "out of range");
}

#[test]
fn unknown_named_field() {
	fail(["t :: (a = 1,)", "t.z"], "no field `z`");
}

#[test]
fn field_of_non_tuple() {
	fail(["x :: 5", "x.0"], "cannot access a field");
}

#[test]
fn fn_returns_tuple() {
	let src = indoc! {"
		pair :: fn() (int, int) { (1, 2) }
		pair()
	"};
	check(src, "(1, 2)");
}

#[test]
fn fn_returns_tuple_field() {
	let src = indoc! {"
		pair :: fn() (int, int) { (10, 20) }
		t :: pair()
		t.1
	"};
	check(src, "20");
}

#[test]
fn fn_return_type_annotation_tuple_no_comma() {
	let src = indoc! {"
		pair :: fn() (int int) { (3, 4) }
		pair()
	"};
	check(src, "(3, 4)");
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
fn fn_tuple_return_composing() {
	let src = indoc! {"
		swap :: fn(x: int, y: int) (int, int) { (y, x) }
		t :: swap(1, 2)
		t.0
	"};
	check(src, "2");
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
	fail("t := (1, 2)\nt.z = 3", "tuple has no field `z`");
}
