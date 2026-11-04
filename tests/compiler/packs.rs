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

#[test]
fn index_reads_zero_and_flow_sees_option() {
	let src = indoc! {r#"
		Sparse :: struct { at: int }
		Sparse : Index[int, ?int] < {
			index :: fn(self; i: int) ?int { if i == self.at do i * 10 else do none }
		}
		stats := ["hp" = 7]
		stats["mp"] += 2
		hp, found := stats["xp"]
		if mp := stats["mp"] do print(mp)
		s := Sparse.{ at = 3 }
		stored: ?int = s[4]
		print(stats["xp"], stats["xp"] or 100, hp, found, s[3], s[4], stored)
	"#};
	check(src, ["2", "0 100 0 false 30 0 none"]);
}

#[test]
fn tuple_returns_through_fn_values() {
	let src = indoc! {"
		divmod :: fn(a, b: int) (q, r: int) { a / b, a % b }
		apply :: fn(f: fn(int, int) (int, int), a: int) (int, int) { f(a, 3) }
		q, r := apply(divmod, 7)
		d := divmod(9, 4)
		print(q, r, d.q, d.r)
	"};
	check(src, "2 1 2 1");
}

#[test]
fn any_cast_reads_zero_and_flow_sees_option() {
	let src = indoc! {r#"
		a: any = "five"
		b: any = 7
		n, ok := int.(a)
		if v := int.(b) do print(v)
		stored: ?int = int.(a)
		print(int.(a), int.(b), int.(a) or 100, string.(a), n, ok, stored)
	"#};
	check(src, ["7", "0 7 100 five 0 false none"]);
}
