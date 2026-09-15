use crate::helpers::*;
use indoc::indoc;

#[test]
fn construct_some() {
	check("?int.(42)", "some(42)");
}

#[test]
fn construct_none() {
	check("?int.(none)", "none");
}

#[test]
fn zero_value_is_none() {
	check("o: ?int\no", "none");
}

#[test]
fn bare_none_without_context_errors() {
	fail_with("none", "cannot infer the type");
}

#[test]
fn ord_gives_tag() {
	check("ord(?int.(42))", "1");
	check("ord(?int.(none))", "0");
}

#[test]
fn int_cast_errors() {
	fail_with("int.(?int.(42))", "no backing value");
}

#[test]
fn eq_same_some() {
	check("?int.(42) == ?int.(42)", "true");
}

#[test]
fn eq_different_some() {
	check("?int.(42) == ?int.(7)", "false");
}

#[test]
fn eq_none_vs_some() {
	check("?int.(none) == ?int.(42)", "false");
	check("?int.(none) != ?int.(42)", "true");
}

#[test]
fn field_type_mismatch() {
	fail_with("?int.(3.0)", "cannot cast float to ?int");
}

#[test]
fn ordering_rejected() {
	fail_with("?int.(1) < ?int.(2)", "only `==` and `!=`");
}

#[test]
fn match_binds_some() {
	check(
		indoc! {r#"
			o :: ?int.(42)
			match o {
				.some(n) => n,
				.none => -1,
			}
		"#},
		"42",
	);
}

#[test]
fn match_none_arm() {
	check(
		indoc! {r#"
			o :: ?int.(none)
			match o {
				.some(n) => n,
				.none => -1,
			}
		"#},
		"-1",
	);
}

#[test]
fn match_non_exhaustive_errors() {
	fail_with(
		indoc! {r"
			o :: ?int.(42)
			match o {
				.some(n) => n,
			}
		"},
		"non-exhaustive match, missing: none",
	);
}

#[test]
fn struct_field_type() {
	check(
		"Box :: struct { val: ?int }
		b :: Box.{ val = ?int.(42) }
		b.val",
		"some(42)",
	);
}

#[test]
fn fn_param_type() {
	let src = indoc! {"
		unwrap_or :: fn(o: ?int, fallback: int) int {
			match o {
				.some(n) => n,
				.none => fallback,
			}
		}
		unwrap_or(?int.(42), 0)
	"};
	check(src, "42");
}

#[test]
fn bare_value_return_wraps_some() {
	let src = indoc! {"
		find :: fn(x: int) ?int {
			return x
		}
		find(5)
	"};
	check(src, "some(5)");
}

#[test]
fn bare_none_return_wraps() {
	let src = indoc! {"
		find :: fn(x: int) ?int {
			return none
		}
		find(5)
	"};
	check(src, "none");
}

#[test]
fn long_form_matches_shorthand() {
	let src = indoc! {"
		find :: fn(id: int) Option[int] {
			if id == 7 { return 42 }
			return none
		}
		find(7) or { -1 }
	"};
	check(src, "42");
	let src = indoc! {"
		find :: fn(id: int) Option[int] {
			if id == 7 { return 42 }
			return none
		}
		find(1) or { -1 }
	"};
	check(src, "-1");
}

#[test]
fn array_payload_is_independent_copy() {
	let src = indoc! {"
		wrap :: fn(a: []int) ?[]int { return a }
		a := [1]
		o :: wrap(a)
		a << 2
		match o { .some(v) => v, .none => [0] }
	"};
	check(src, "[1]");
}

#[test]
fn values_coerce_into_options() {
	let src = indoc! {r#"
		T :: enum { nil, int, float }
		S :: struct { ret: ?T, d: ?int }
		mk :: fn(ret: ?T, d: ?int = none) S { S.{ ret, d } }
		pick :: fn(b: bool) ?T { if b { .int } else { none } }
		a: ?int = 3
		b: ?T = .int
		s := S.{ .float, 4 }
		print("{a or 0} {b or .nil} {s.ret or .nil} {s.d or 0}")
		print("{mk(.int).ret or .nil} {mk(T.int, a).d or 0} {mk(none).ret == none}")
		print("{pick(true) or .nil} {pick(false) or .nil}")
	"#};
	check(src, ["3 int float 4", "int 3 true", "int nil"]);
}
