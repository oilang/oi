use crate::helpers::*;

#[test]
fn unwraps_or_falls_back() {
	let src = indoc! {r#"
		print(?int.(none) or { -1 })
		print(!int.(error("oops")) or { -1 })
		print(?int.(42) or { -1 })
		x :: !int.(42) or {
			print("ran")
			9
		}
		x
	"#};
	check(src, ["-1", "-1", "42", "42"]);
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
fn fallback_can_diverge() {
	let src = indoc! {"
		unwrap_or_bail :: fn(o: ?int) int {
			v :: o or { return -1 }
			v
		}
		f :: fn(o: ?int) int {
			y := o or return 0
			z := match o { .none => return 0, .some.(v) => v }
			y + z
		}
		print(unwrap_or_bail(?int.(none)))
		print(unwrap_or_bail(?int.(42)))
		f(?int.(none)) + f(?int.(2))
	"};
	check(src, ["-1", "42", "4"]);
}

#[test]
fn diverging_fallback_ends_at_semicolon() {
	let src = indoc! {"
		f :: fn(o: ?int) {
			loop { y := o or break; print(y); break }
			y := o or return; print(y)
		}
		f(?int.(1))
		f(?int.(none))
	"};
	check(src, "1\n1");
}

#[test]
fn fallback_can_panic() {
	check(r#"!string.("hi") or { panic!("boom") }"#, "hi");
	fail_rt(r#"!string.(error("boom")) or { panic!("boom") }"#, "panic: boom");
	fail_rt(r#"!string.(error("boom")) or panic!("boom")"#, "panic: boom");
}

#[test]
fn type_errors() {
	fail(
		r#"?int.(42) or { "wrong" }"#,
		"or` branches have mismatched types: int and str",
	);
	fail("42 or { 0 }", "needs a `?T`/`!T` value");
}
