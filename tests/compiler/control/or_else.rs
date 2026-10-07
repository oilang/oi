use crate::helpers::*;

#[test]
fn fallback_on_none() {
	check("?int.(none) or { -1 }", "-1");
}

#[test]
fn fallback_on_err() {
	check(r#"!int.(error("oops")) or { -1 }"#, "-1");
}

#[test]
fn unwraps_some() {
	check("?int.(42) or { -1 }", "42");
}

#[test]
fn skips_fallback_body_when_ok() {
	let src = indoc! {r#"
		!int.(42) or {
			print("ran")
			9
		}
	"#};
	check(src, "42");
}

#[test]
fn dollar_is_error_message() {
	let src = indoc! {r#"
		!int.(error("boom")) or {
			print($)
			9
		}
	"#};
	check(src, ["boom", "9"]);
}

#[test]
fn as_binding() {
	check(["x :: ?int.(none) or { 99 }", "x"], "99");
}

#[test]
fn fallback_can_diverge() {
	let src = indoc! {"
		unwrap_or_bail :: fn(o: ?int) int {
			v :: o or { return -1 }
			v
		}
		unwrap_or_bail(?int.(none))
	"};
	check(src, "-1");

	let src = indoc! {"
		unwrap_or_bail :: fn(o: ?int) int {
			v :: o or { return -1 }
			v
		}
		unwrap_or_bail(?int.(42))
	"};
	check(src, "42");
}

#[test]
fn bare_return_diverges() {
	let src = indoc! {"
		f :: fn(o: ?int) int {
			y := o or return 0
			z := match o { .none => return 0, .some.(v) => v }
			y + z
		}
		f(?int.(none)) + f(?int.(2))
	"};
	check(src, "4");
}

#[test]
fn fallback_can_panic() {
	check(r#"!string.("hi") or { panic!("boom") }"#, "hi");
	fail_rt(r#"!string.(error("boom")) or { panic!("boom") }"#, "panic: boom");
	fail_rt(r#"!string.(error("boom")) or panic!("boom")"#, "panic: boom");
}

#[test]
fn type_mismatch_errors() {
	fail(
		r#"?int.(42) or { "wrong" }"#,
		"or` branches have mismatched types: int and str",
	);
}

#[test]
fn requires_option_or_result() {
	fail("42 or { 0 }", "needs a `?T`/`!T` value");
}
