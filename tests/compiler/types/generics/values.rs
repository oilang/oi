use crate::common::Project;
use crate::helpers::*;
use indoc::indoc;

#[test]
fn value_params() {
	let src = indoc! {r#"
		repeat[T, N: int] :: fn(x: T) out: [N]T {
			loop i in 0..N { out[i] = x }
			out
		}
		Matrix[R: int, C: int] :: struct { cells: [R][C]float }
		print(repeat[string, 2]("hi"))
		m := Matrix[2, 4].{}
		print(m.cells.len, m.cells[0].len)
	"#};
	check(src, [r#"["hi", "hi"]"#, "2 4"]);
}

#[test]
fn a_value_param_crosses_a_module() {
	Project::new()
		.file("g/mod.oi", "module g\npub zeros[N: int] :: fn() out: [N]int { out }")
		.file("main.oi", "use g\nprint(g.zeros[3]().len)")
		.check("3");
}

#[test]
fn a_lone_value_arg_beats_indexing() {
	let src = indoc! {r"
		zeros[N: int] :: fn() [N]int { out: [N]int; out }
		fs: []fn() int = .[fn() int { 7 }]
		print(zeros[3]().len, fs[0]())
	"};
	check(src, "3 7");
}

#[test]
fn a_value_param_takes_a_const() {
	let src = indoc! {r"
		N :: 3
		repeat[T, N: int] :: fn(x: T) [N]T { out: [N]T; out }
		Matrix[R: int, C: int] :: struct { cells: [R][C]float }
		print(repeat[int, N](0), Matrix[N, N].{}.cells[0].len)
	"};
	check(src, "[0, 0, 0] 3");
}

#[test]
fn a_type_where_a_value_belongs() {
	fail(
		indoc! {"
			repeat[T, N: int] :: fn(x: T) [N]T { out: [N]T; out }
			print(repeat[int, int](1))
		"},
		"`N` is a value parameter",
	);
}

#[test]
fn a_value_where_a_type_belongs() {
	fail(
		indoc! {"
			first[A, B] :: fn(a: A, b: B) A { a }
			print(first[int, 2](1, 2))
		"},
		"`B` is a type parameter",
	);
}

#[test]
fn a_value_param_infers_from_a_fixed_array() {
	let src = indoc! {r"
		last[T, N: int] :: fn(xs: [N]T) T { xs[N - 1] }
		xs: [3]int = .[1, 2, 3]
		print(last(xs))
	"};
	check(src, "3");
}
