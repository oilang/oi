use crate::helpers::*;

#[test]
fn threads_steps() {
	let src = indoc! {"
		double :: fn(x: int) int { x * 2 }
		inc :: fn(x: int) int { x + 1 }
		print(3 |> double, 3 |> double |> inc |> double)
		print(3 |> $ + 1, 1 + 1 |> double)
	"};
	check(src, ["6 14", "4 4"]);
}

#[test]
fn question_step() {
	let src = indoc! {r#"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		display :: fn(id: int) ?int {
			v :: id |> find?
			v + 1
		}
		load :: fn(path: string) !int {
			if path == "ok" { return 42 }
			return error("missing")
		}
		double :: fn(path: string) !int {
			v :: path |> load?
			v * 2
		}
		print(display(7) or { -1 }, display(1) or { -1 }, double("ok") or { -1 })
		double("nope") or {
			print($)
			0
		}
	"#};
	check(src, ["43 -1 84", "missing", "0"]);
}

#[test]
fn or_catches_question_steps() {
	let src = indoc! {r#"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		load :: fn(id: int) !int {
			if id > 0 { return id }
			return error("missing {id}")
		}
		show :: fn(id: int) int { find(id)? |> $ * 2 or -1 }
		both :: fn(id: int) int { id |> load? |> { load($ - 2)? } or { print($); 0 } }
		inner :: fn(id: int) int { id |> fn (x: int) ?int { find(x)? + 1 } or 0 }
		print(show(7), show(1), both(3), both(1), inner(7), inner(1))
	"#};
	check(src, ["84 -1 1 missing -1", "0 43 0"]);
}

#[test]
fn wrapped_head_hints_unwrap() {
	let src = indoc! {"
		double :: fn(x: int) int { x * 2 }
		maybe :: fn() ?int { 1 }
		maybe() |> double
	"};
	fail(src, "add `?` to unwrap the head");
}

#[test]
fn or_tail_after_chain() {
	let src = indoc! {r#"
		find :: fn(id: int) ?int {
			if id == 7 { return 42 }
			return none
		}
		name :: fn(id: int) ?string {
			if id == 7 { return "found" }
			return none
		}
		print(7 |> find or { -1 }, 1 |> find or { -1 })
		1 |> name or "anonymous"
	"#};
	check(src, ["42 -1", "anonymous"]);
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
		zero :: fn() int { 7 }
		double :: fn(x: int) int { x * 2 }
		add :: fn(a: int, b: int) int { a + b }
		quad :: double |> double
		f :: double |> add(10, $)
		g :: zero |> double
		h :: add |> double
		print(quad(3), f(3), g(), h(2, 3))
	"};
	check(src, "12 16 14 10");
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
		print(5 |> fn(x: int) int { x * 2 })
	"};
	check(src, ["Point.{x = 2, y = 2}", "10"]);
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
