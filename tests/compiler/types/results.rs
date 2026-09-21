use crate::helpers::*;
use indoc::indoc;

#[test]
fn construct_ok() {
	check("!int.(42)", "ok.(42)");
}

#[test]
fn construct_err() {
	check(r#"!int.(error("oops"))"#, r#"err.("oops")"#);
}

#[test]
fn zero_value_is_ok() {
	check("r: !int; r", "ok.(0)");
}

#[test]
fn ord_gives_tag() {
	check("ord(!int.(42))", "0");
	check(r#"ord(!int.(error("oops")))"#, "1");
}

#[test]
fn int_cast_errors() {
	fail_with("int.(!int.(42))", "no backing value");
}

#[test]
fn eq_same_ok() {
	check("!int.(42) == !int.(42)", "true");
}

#[test]
fn eq_different_ok() {
	check("!int.(42) == !int.(7)", "false");
}

#[test]
fn eq_ok_vs_err() {
	check(r#"!int.(42) == !int.(error("oops"))"#, "false");
	check(r#"!int.(42) != !int.(error("oops"))"#, "true");
}

#[test]
fn field_type_mismatch() {
	fail_with("!int.(3.0)", "expected int or Error, got float");
}

#[test]
fn ordering_rejected() {
	fail_with("!int.(1) < !int.(2)", "only `==` and `!=`");
}

#[test]
fn match_binds_ok() {
	check(
		indoc! {r#"
			r :: !int.(42)
			match r {
				.ok.(n) => n,
				.err.(e) => -1,
			}
		"#},
		"42",
	);
}

#[test]
fn match_err_arm() {
	check(
		indoc! {r#"
			r :: !int.(error("oops"))
			match r {
				.ok.(n) => n,
				.err.(e) => -1,
			}
		"#},
		"-1",
	);
}

#[test]
fn match_non_exhaustive_errors() {
	fail_with(
		indoc! {r"
			r :: !int.(42)
			match r {
				.ok.(n) => n,
			}
		"},
		"non-exhaustive match, missing: err",
	);
}

#[test]
fn struct_field_type() {
	check(
		"Box :: struct { val: !int }
		b :: Box.{ val = !int.(42) }
		b.val",
		"ok.(42)",
	);
}

#[test]
fn fn_param_type() {
	let src = indoc! {"
		unwrap_or :: fn(r: !int, fallback: int) int {
			match r {
				.ok.(n) => n,
				.err.(e) => fallback,
			}
		}
		unwrap_or(!int.(42), 0)
	"};
	check(src, "42");
}

#[test]
fn bare_value_return_wraps_ok() {
	let src = indoc! {"
		find :: fn(x: int) !int {
			return x
		}
		find(5)
	"};
	check(src, "ok.(5)");
}

#[test]
fn bare_error_return_wraps_err() {
	let src = indoc! {r#"
		find :: fn(x: int) !int {
			return error("not found")
		}
		find(5)
	"#};
	check(src, r#"err.("not found")"#);
}

#[test]
fn error_message() {
	check(r#"error("oops").message()"#, "oops");
}

#[test]
fn error_message_via_dollar() {
	let src = indoc! {r#"
		!int.(error("boom")) or {
			print($.message())
			0
		}
	"#};
	check(src, "boom\n0");
}

#[test]
fn error_unknown_method() {
	fail_with(r#"error("oops").nope()"#, "no method `nope`");
}

#[test]
fn long_form_matches_shorthand() {
	let src = indoc! {r#"
		load :: fn(path: string) Result[int, Error] {
			if path == "ok" { return 42 }
			return error("missing")
		}
		double :: fn(path: string) Result[int, Error] {
			v :: load(path)?
			v * 2
		}
		double("ok") or { -1 }
	"#};
	check(src, "84");
	let src = indoc! {r#"
		load :: fn(path: string) Result[int, Error] {
			if path == "ok" { return 42 }
			return error("missing")
		}
		double :: fn(path: string) Result[int, Error] {
			v :: load(path)?
			v * 2
		}
		double("nope") or {
			print($)
			0
		}
	"#};
	check(src, ["missing", "0"]);
}

#[test]
fn long_form_nested() {
	let src = indoc! {r#"
		load :: fn() Result[[]int, Error] {
			return [1, 2, 3]
		}
		load() or { [-1] }
	"#};
	check(src, "[1, 2, 3]");
}

#[test]
fn pinned_enum_error() {
	let src = indoc! {"
		NetError :: enum { timeout refused }
		fetch :: fn(x: int) Result[int, NetError] {
			if x > 0 { return x }
			return error(NetError.timeout)
		}
		fetch(-1) or {
			print($ == NetError.timeout)
			0
		}
	"};
	check(src, ["true", "0"]);
}

#[test]
fn pinned_bare_err_return() {
	let src = indoc! {r#"
		parse :: fn(x: int) Result[int, string] {
			if x > 0 { return x }
			return "nope"
		}
		parse(-1) or {
			print($)
			0
		}
	"#};
	check(src, ["nope", "0"]);
}

#[test]
fn pinned_error_propagates() {
	let src = indoc! {"
		NetError :: enum { timeout refused }
		fetch :: fn(x: int) Result[int, NetError] {
			if x > 0 { return x }
			return error(NetError.timeout)
		}
		retry :: fn(x: int) Result[int, NetError] {
			v :: fetch(x)?
			v * 2
		}
		retry(-1) or { -1 }
	"};
	check(src, "-1");
}

#[test]
fn unclaimed_error_propagation_rejected() {
	let src = indoc! {"
		NetError :: enum { timeout refused }
		fetch :: fn(x: int) Result[int, NetError] { x }
		load :: fn() !int {
			v :: fetch(1)?
			v
		}
		load()
	"};
	fail_with(src, "does not claim Error");
}

#[test]
fn error_cause_defaults_to_none() {
	let src = indoc! {r#"
		find :: fn() !int {
			return error("boom")
		}
		find() or {
			print("{$.cause()}")
			0
		}
	"#};
	check(src, ["none", "0"]);
}

#[test]
fn claimed_error_bare_return() {
	let src = indoc! {r#"
		NetError :: enum { timeout refused }
		NetError : Error < {
			message :: fn(self) string { "net down" }
		}
		f :: fn() !int { return NetError.timeout }
		f() or {
			print($.message())
			0
		}
	"#};
	check(src, ["net down", "0"]);
}

#[test]
fn claimed_error_wrapped() {
	let src = indoc! {r#"
		NetError :: enum { timeout refused }
		NetError : Error < {
			message :: fn(self) string { "net down" }
		}
		f :: fn() !int { return error(NetError.timeout) }
		f() or {
			print($.message())
			0
		}
	"#};
	check(src, ["net down", "0"]);
}

#[test]
fn pinned_propagates_into_open() {
	let src = indoc! {r#"
		NetError :: enum { timeout refused }
		NetError : Error < {
			message :: fn(self) string { "net down" }
		}
		a :: fn() Result[int, NetError] { return NetError.refused }
		b :: fn() !int { a()? }
		b() or {
			print($.message())
			0
		}
	"#};
	check(src, ["net down", "0"]);
}

#[test]
fn error_disambiguates_pinned() {
	let src = indoc! {"
		f :: fn() Result[int, int] { return error(-42) }
		f() or {
			print($)
			0
		}
	"};
	check(src, ["-42", "0"]);
}

#[test]
fn pinned_shorthand_signature() {
	let src = indoc! {"
		NetError :: enum { timeout refused }
		fetch :: fn(x: int) NetError!int {
			if x > 0 { return x }
			return NetError.timeout
		}
		fetch(-1) or {
			print($ == NetError.timeout)
			0
		}
	"};
	check(src, ["true", "0"]);
}

#[test]
fn pinned_variant_shorthand() {
	let src = indoc! {"
		NetError :: enum { timeout refused }
		fetch :: fn(x: int) NetError!int {
			if x == 0 { return .timeout }
			if x < 0 { return error(.refused) }
			x
		}
		fetch(0) or {
			print($ == NetError.timeout)
			0
		}
		fetch(-1) or {
			print($ == NetError.refused)
			0
		}
	"};
	check(src, ["true", "true", "0"]);
}

#[test]
fn bang_on_value_call_is_not() {
	let src = indoc! {r#"
		f :: fn(get: fn(s: string) bool) bool { !get("x") }
		xs := [1, 2]
		print(f(fn(s: string) bool { s == "x" }), !xs.contains(9))
	"#};
	check(src, "false true");
}
