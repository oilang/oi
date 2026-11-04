use crate::helpers::*;
use indoc::indoc;

#[test]
fn comma_lists_destructure_and_build_tuples() {
	let src = indoc! {"
		pair :: fn(a, b: int) (int, int) { return b, a }
		tail :: fn(a: int) (int, int) { a, a + 1 }
		q, r := 1, 2
		q, r = r, q
		x, y := pair(q, r)
		m, n :: tail(x)
		print(q, r, x, y, m, n)
	"};
	check(src, "2 1 1 2 1 2");
}

#[test]
fn name_list_needs_a_tuple_value() {
	fail("lo, hi := 0", "chain `a := b := ...`");
}

#[test]
fn two_names_destructure_option_and_result() {
	let src = indoc! {r#"
		find :: fn(n: int) ?string { if n > 0 do "hit" else do none }
		parse :: fn(n: int) !int { if n > 0 do n else do error("bad") }
		s, ok := find(1)
		e, gone := find(0)
		n, err := parse(7)
		m, bad := parse(0)
		print(s, ok, e == "", gone, n, err == none, m, bad != none)
	"#};
	check(src, "hit true true false 7 true 0 true");
}
