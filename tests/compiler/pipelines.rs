use crate::helpers::*;

#[test]
fn threads_through_bare_fn_step() {
	let src = indoc! {"
		double :: fn(x: int) int { x * 2 }
		3 |> double
	"};
	check(src, "6");
}

#[test]
fn threads_through_multiple_steps() {
	let src = indoc! {"
		double :: fn(x: int) int { x * 2 }
		inc :: fn(x: int) int { x + 1 }
		3 |> double |> inc |> double
	"};
	check(src, "14");
}

#[test]
fn dollar_expression_step() {
	check("3 |> $ + 1", "4");
}

#[test]
fn pipe_is_loosest() {
	let src = indoc! {"
		double :: fn(x: int) int { x * 2 }
		1 + 1 |> double
	"};
	check(src, "4");
}

#[test]
fn question_step_unwraps_some() {
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		display :: fn(id: int) ?int {
			v :: id |> find?
			v + 1
		}
		display(7) or { -1 }
	"};
	check(src, "43");
}

#[test]
fn question_step_propagates_none() {
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		display :: fn(id: int) ?int {
			v :: id |> find?
			v + 1
		}
		display(1) or { -1 }
	"};
	check(src, "-1");
}

#[test]
fn bang_step_unwraps_ok() {
	let src = indoc! {r#"
		load :: fn(path: string) !int {
			if path == "ok" { return 42 }
			return error("missing")
		}
		double :: fn(path: string) !int {
			v :: path |> load?
			v * 2
		}
		double("ok") or { -1 }
	"#};
	check(src, "84");
}

#[test]
fn bang_step_propagates_error() {
	let src = indoc! {r#"
		load :: fn(path: string) !int {
			if path == "ok" { return 42 }
			return error("missing")
		}
		double :: fn(path: string) !int {
			v :: path |> load?
			v * 2
		}
		double("nope") or {
			print($)
			0
		}
	"#};
	check(src, "missing\n0");
}

#[test]
fn or_tail_after_chain() {
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		7 |> find or { -1 }
	"};
	check(src, "42");
}

#[test]
fn or_tail_after_chain_fallback() {
	let src = indoc! {"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		1 |> find or { -1 }
	"};
	check(src, "-1");
}

#[test]
fn or_tail_bare_literal() {
	let src = indoc! {r#"
		find :: fn(id: int) ?string {
			if id == 7 { return "found" }
			return none
		}
		1 |> find or "anonymous"
	"#};
	check(src, "anonymous");
}

#[test]
fn or_tail_bare_ident_calls_with_dollar() {
	let src = indoc! {r#"
		find :: fn(id: int) !int {
			if id == 7 { return 42 }
			return error("missing")
		}
		handler[E] :: fn(e: E) int {
			print(e.message())
			0
		}
		find(1) or handler
	"#};
	check(src, ["missing", "0"]);
}

#[test]
fn composes_named_fns() {
	let src = indoc! {"
		double :: fn(x: int) int { x * 2 }
		quad :: double |> double
		print(quad(3))
	"};
	check(src, "12");
}

#[test]
fn composes_with_a_call_stage() {
	let src = indoc! {"
		double :: fn(x: int) int { x * 2 }
		add :: fn(a: int, b: int) int { a + b }
		f :: double |> add(10, $)
		print(f(3))
	"};
	check(src, "16");
}

#[test]
fn composes_fn_literals() {
	let src = indoc! {"
		Point :: struct {
			x: int
			y: int
		}
		f :: fn(x: int) (int, int) { (x, x) } |> fn(x: int, y: int) Point { Point.{ x, y } }
		print(f(2))
	"};
	check(src, "Point.{x = 2, y = 2}");
}

#[test]
fn data_head_applies_a_literal_stage() {
	check("print(5 |> fn(x: int) int { x * 2 })", "10");
}

#[test]
fn composition_tail_must_be_a_fn() {
	let src = indoc! {"
		double :: fn(x: int) int { x * 2 }
		f :: double |> ($ + 1)
		print(f(3))
	"};
	fail(src, "cannot infer the composed return type");
}

#[test]
fn generic_head_cannot_compose() {
	let src = indoc! {"
		id[T] :: fn(x: T) T { x }
		double :: fn(x: int) int { x * 2 }
		f :: id |> double
		print(f(3))
	"};
	fail(src, "cannot compose a generic function");
}

#[test]
fn composes_heads_of_any_arity() {
	let src = indoc! {"
		zero :: fn() int { 7 }
		add :: fn(a: int, b: int) int { a + b }
		double :: fn(x: int) int { x * 2 }
		f :: zero |> double
		g :: add |> double
		print(f())
		print(g(2, 3))
	"};
	check(src, ["14", "10"]);
}
