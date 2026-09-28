use crate::helpers::*;
use indoc::indoc;

#[test]
fn value_params() {
	let src = indoc! {r#"
		repeat[T, N: int] :: fn(x: T) [N]T {
			out: [N]T
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
