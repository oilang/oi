use crate::helpers::*;

#[test]
fn question_on_option() {
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		display :: fn(id: int) ?int {
			v :: find(id)?
			v + 1
		}
		print(display(7) or { -1 })
		print(display(1) or { -1 })
	"};
	check(src, ["43", "-1"]);
}

#[test]
fn question_on_result() {
	let src = indoc! {r#"
		load :: fn(path: string) !int {
			if path == "ok" { return 42 }
			return error("missing")
		}
		double :: fn(path: string) !int {
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
fn requires_option_or_result() {
	fail(["f :: fn() int { 42? }", "f()"], "`?` needs a `?T` or `!T` value");
}

#[test]
fn panics_in_main() {
	fail_rt(["find :: fn() ?int { none }", "find()?"], "panic: unwrapped `none`");
	fail_rt(
		[r#"load :: fn() !int { error("missing") }"#, "load()?"],
		"panic: missing",
	);
}

#[test]
fn bang_main() {
	check(["load :: fn() !int { 42 }", "main :: fn() ! { print(load()?) }"], "42");
	let bad = indoc! {r#"
		load :: fn() !int { return error("missing") }
		main :: fn() ! { print(load()?) }
	"#};
	fail_rt(bad, "error: missing");
	let pinned = indoc! {r#"
		A :: struct { m: string }
		A :< Error
		main :: fn() A! { return A.{ m = "boom" } }
	"#};
	fail_rt(pinned, r#"error: A.{m = "boom"}"#);
	fail("main :: fn() int { 5 }", "`main` cannot return `int`");
}

#[test]
fn requires_matching_enclosing_return() {
	fail(
		[
			"find :: fn() ?int { 42 }",
			"display :: fn() int { find()? }",
			"display()",
		],
		"needs an enclosing fn returning `?T`",
	);
	fail(
		[
			"load :: fn() !int { 42 }",
			"display :: fn() ?int { load()? }",
			"display()",
		],
		"needs an enclosing fn returning `!T`",
	);
}

#[test]
fn question_converts_via_from() {
	let src = indoc! {"
		A :: struct { code: int }
		B :: struct { code: int }
		B : From[A] < { from :: fn(a: A) Self { B.{ code = a.code + 1 } } }
		inner :: fn() A!int { return A.{ code = 7 } }
		outer :: fn() B!int { inner()? }
		print(outer() or { $.code })
	"};
	check(src, "8");
}
