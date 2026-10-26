use crate::common::Project;
use crate::helpers::*;
use indoc::indoc;

#[test]
fn value_params() {
	let src = indoc! {r#"
		repeat[T; N: int] :: fn(x: T) out: [N]T {
			loop i in 0..N { out[i] = x }
			out
		}
		Matrix[R: int, C: int] :: struct { cells: [R][C]float }
		print(repeat[string, 2]("hi"))
		m := Matrix[2, 4].{}
		print(m.cells.len(), m.cells[0].len())
	"#};
	check(src, [r#"["hi", "hi"]"#, "2 4"]);
}

#[test]
fn a_value_param_crosses_a_module() {
	Project::new()
		.file("g/mod.oi", "module g\npub zeros[N: int] :: fn() out: [N]int { out }")
		.file("main.oi", "use g\nprint(g.zeros[3]().len())")
		.check("3");
}

#[test]
fn a_lone_value_arg_beats_indexing() {
	let src = indoc! {r"
		zeros[N: int] :: fn() [N]int { out: [N]int; out }
		fs: []fn() int = .[fn() int { 7 }]
		print(zeros[3]().len(), fs[0]())
	"};
	check(src, "3 7");
}

#[test]
fn a_value_param_takes_a_const() {
	let src = indoc! {r"
		N :: 3
		repeat[T; N: int] :: fn(x: T) [N]T { out: [N]T; out }
		Matrix[R: int, C: int] :: struct { cells: [R][C]float }
		print(repeat[int, N](0), Matrix[N, N].{}.cells[0].len())
	"};
	check(src, "[0, 0, 0] 3");
}

#[test]
fn param_kind_mismatch() {
	fail(
		[
			"repeat[T; N: int] :: fn(x: T) [N]T { out: [N]T; out }",
			"print(repeat[int, int](1))",
		],
		"`N` is a value parameter",
	);
	fail(
		["first[A, B] :: fn(a: A, b: B) A { a }", "print(first[int, 2](1, 2))"],
		"`B` is a type parameter",
	);
}

#[test]
fn a_value_param_infers_from_a_fixed_array() {
	let src = indoc! {r"
		last[T; N: int] :: fn(xs: [N]T) T { xs[N - 1] }
		xs: [3]int = .[1, 2, 3]
		print(last(xs))
	"};
	check(src, "3");
}

#[test]
fn a_value_param_takes_any_comptime_value() {
	let src = indoc! {r#"
		greet[S: string] :: fn() string { S }
		Config :: struct { n: int }
		pool[C: Config] :: fn() int { C.n }
		size[N: int] :: fn() string { comp if N > 2 { "big" } else { "small" } }
		sq :: fn(x: int) int { x * x }
		print(greet["hi"](), pool[Config.{n = 3}](), size[1 + 1](), size[sq(2)]())
	"#};
	check(src, "hi 3 small big");
}

#[test]
fn a_value_arg_sees_the_enclosing_params() {
	let src = indoc! {r"
		sum[N: int] :: fn() int { comp if N == 0 { 0 } else { N + sum[N - 1]() } }
		print(sum[3]())
	"};
	check(src, "6");
}

#[test]
fn a_value_param_takes_any_type_as_its_bound() {
	check(
		["second[X: []int] :: fn() int { X[1] }", "print(second[.[1, 2, 3]]())"],
		"2",
	);
}

#[test]
fn an_anon_value_arg_takes_its_bound() {
	check(
		[
			"Mode :: enum { a, b }",
			"pick[M: Mode] :: fn() Mode { M }",
			"print(pick[.b]())",
		],
		"b",
	);
}

#[test]
fn a_value_arg_must_match_its_bound() {
	fail(
		["greet[S: string] :: fn() string { S }", "print(greet[3]())"],
		"`S` is a `string` parameter",
	);
}
