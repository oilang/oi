use crate::helpers::*;
use indoc::indoc;

#[test]
fn basics() {
	check(
		indoc! {r#"
			Box :: struct { val: !int }
			print(!int.(42), !int.(error("oops")), Box.{ val = !int.(42) }.val, ord(!int.(42)), ord(!int.(error("oops"))))
			print(!int.(42) == !int.(42), !int.(42) == !int.(7), !int.(42) == !int.(error("oops")), !int.(42) != !int.(error("oops")))
		"#},
		[r#"ok.(42) err.("oops") ok.(42) 0 1"#, "true false false true"],
	);
}

#[test]
fn nozero_leaves_no_zero_value() {
	fail("r: !int; r", "`r` is not assigned on every path");
	fail("f :: fn() !int { return }; f()", "`!int` has no zero value");
}

#[test]
fn field_type_mismatch() {
	fail("!int.(3.0)", "expected int or Error, got float");
}

#[test]
fn match_arms() {
	let src = indoc! {r#"
		unwrap_or :: fn(r: !int, fallback: int) int {
			match r {
				.ok.(n) => n,
				.err.(e) => fallback,
			}
		}
		print(unwrap_or(!int.(42), 0), unwrap_or(!int.(error("oops")), -1))
	"#};
	check(src, "42 -1");
}

#[test]
fn bare_returns_wrap() {
	let src = indoc! {r#"
		find :: fn(x: int) !int {
			if x > 0 { return x }
			return error("not found")
		}
		print(find(5), find(0))
	"#};
	check(src, r#"ok.(5) err.("not found")"#);
}

#[test]
fn error_message() {
	let src = indoc! {r#"
		print(error("oops").message())
		!int.(error("boom")) or {
			print($.message())
			0
		}
	"#};
	check(src, ["oops", "boom", "0"]);
}

#[test]
fn error_unknown_method() {
	fail(r#"error("oops").nope()"#, "no method `nope`");
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
		print(double("ok") or { -1 })
		double("nope") or {
			print($)
			0
		}
	"#};
	check(src, ["84", "missing", "0"]);
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
	fail(src, "does not claim Error");
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
fn claimed_error() {
	let src = indoc! {r#"
		NetError :: enum { timeout refused }
		NetError : Error < {
			message :: fn(self) string { "net down" }
		}
		bare :: fn() !int { return NetError.timeout }
		wrapped :: fn() !int { return error(NetError.timeout) }
		a :: fn() Result[int, NetError] { return NetError.refused }
		b :: fn() !int { a()? }
		bare() or { print($.message()) 0 }
		wrapped() or { print($.message()) 0 }
		b() or { print($.message()) 0 }
	"#};
	check(src, ["net down", "net down", "net down", "0"]);
}

#[test]
fn error_message_defaults_to_str() {
	let src = indoc! {r#"
		NetError :: enum { timeout refused }
		NetError :< Error
		Io :: struct {}
		Io :< Error
		e :: fn() !int { return NetError.timeout }
		a :: fn() !int { return :oops }
		i :: fn() !int { return Io.{} }
		e() or { print($.message()) 0 }
		a() or { print($.message()) 0 }
		i() or { print($.message()) 0 }
	"#};
	check(src, ["timeout", ":oops", "Io.{}", "0"]);
}

#[test]
fn result_ok_discards_the_error() {
	let src = indoc! {r#"
		f :: fn(n: int) !int { if n == 0 { return error("boom") } n }
		print(f(1).ok())
		print(f(0).ok())
	"#};
	check(src, ["some.(1)", "none"]);
}

#[test]
fn result_context_wraps_the_error() {
	let src = indoc! {r#"
		f :: fn(n: int) !int { if n == 0 { return error("disk on fire") } n }
		g :: fn() !int { f(0).context("loading save")? }
		f(0).context("loading save") or {
			print($.message())
			print(match $.cause() { .some.(e) => e.message(), .none => "none" })
			0
		}
		g() or { print($.message()) 0 }
	"#};
	check(
		src,
		[
			"loading save: disk on fire",
			"disk on fire",
			"loading save: disk on fire",
			"0",
		],
	);
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
