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
