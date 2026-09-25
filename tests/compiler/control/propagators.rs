use crate::helpers::*;

#[test]
fn question_unwraps_some() {
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		display :: fn(id: int) ?int {
			v :: find(id)?
			v + 1
		}
		display(7) or { -1 }
	"};
	check(src, "43");
}

#[test]
fn question_propagates_none() {
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		display :: fn(id: int) ?int {
			v :: find(id)?
			v + 1
		}
		display(1) or { -1 }
	"};
	check(src, "-1");
}

#[test]
fn bang_unwraps_ok() {
	let src = indoc! {r#"
		load :: fn(path: string) !int {
			if path == "ok" { return 42 }
			return error("missing")
		}
		double :: fn(path: string) !int {
			v :: load(path)?
			v * 2
		}
		double("ok") or { -1 }
	"#};
	check(src, "84");
}

#[test]
fn bang_propagates_error() {
	let src = indoc! {r#"
		load :: fn(path: string) !int {
			if path == "ok" { return 42 }
			return error("missing")
		}
		double :: fn(path: string) !int {
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
fn requires_option_or_result() {
	fail(["f :: fn() int { 42? }", "f()"], "`?` needs a `?T` or `!T` value");
}

#[test]
fn option_panics_in_main() {
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		find(1)?
	"};
	fail_rt(src, "panic: unwrapped `none`");
}

#[test]
fn result_panics_in_main() {
	let src = indoc! {r#"
		load :: fn(path: string) !int {
			if path == "ok" { return 42 }
			return error("missing")
		}
		load("nope")?
	"#};
	fail_rt(src, "panic: missing");
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
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		display :: fn(id: int) int {
			find(id)?
		}
		display(7)
	"};
	fail(src, "needs an enclosing fn returning `?T`");
}

#[test]
fn requires_matching_enclosing_return_result() {
	let src = indoc! {r#"
		load :: fn(path: string) !int {
			if path == "ok" { return 42 }
			return error("missing")
		}
		display :: fn(path: string) ?int {
			load(path)?
		}
		display("ok")
	"#};
	fail(src, "needs an enclosing fn returning `!T`");
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
