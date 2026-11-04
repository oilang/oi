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

#[test]
fn sum_cast_extracts_member_or_zero() {
	let src = indoc! {r#"
		P :: struct { x: int }
		U :: P | string
		u: U = "x"
		w: U = P.{ x = 3 }
		n, ok := P.(u)
		if p := P.(w) do print(p.x)
		print(string.(u), string.(w), string.(w) or "none", n, ok)
	"#};
	check(src, ["3", "x  none P.{x = 0} false"]);
}

#[test]
fn error_slots() {
	// partial packs flow through their error slot
	let src = indoc! {r#"
		open :: fn(k: int) (n: int, err: ?Error) { if k > 0 do k, none else do 0, error("shut") }
		pair :: fn(k: int) (a, b: int, err: ?Error) { if k > 0 do k, -k, none else do 0, 0, error("odd") }
		twice :: fn(k: int) (n: int, err: ?Error) { open(k)? * 2, none }
		strict :: fn(k: int) !int { open(k)? }
		if n := open(2) do print(n)
		if p := pair(3) do print(p.0, p.1)
		pair(4) and print($.0 + $.1)
		n, err := twice(0)
		print(open(5)?, open(0) or 7, twice(4), n, err, strict(0))
	"#};
	check(
		src,
		[
			"2",
			"3 -3",
			"0",
			r#"5 7 (n = 8, err = none) 0 some.("shut") err.("shut")"#,
		],
	);

	// one name cannot hide an error slot
	let src = indoc! {"
		open :: fn(k: int) (n: int, err: ?Error) { k, none }
		t := open(1)
	"};
	fail(src, "or bind both, `n, err := ...`");

	// `defer or` reads the named error slot
	let src = indoc! {r#"
		Tx :: struct { n: int }
		store :: fn(n: int) !int { if n > 0 do n else do error("full") }
		commit :: fn(k: int) (tx: Tx, err: ?Error) {
			tx = Tx.{n = k}
			defer or print("abort", tx.n, $)
			defer and print("done", $.n)
			store(k) or {
				err = $
				return
			}
			return
		}
		a, ok := commit(1)
		b, bad := commit(-2)
		print(a.n, ok, b.n, bad)
	"#};
	check(src, ["done 1", "abort -2 full", r#"1 none -2 some.("full")"#]);
}

#[test]
fn fallible_returns_allocate_nothing_on_success() {
	let src = indoc! {"
		count := 0
		@c counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc { count = count + 1 }
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}
		open :: fn(k: int) (n: int, err: ?Error) { k, none }
		main :: fn() {
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr.(0) } }
			a := open(1) or 0
			b, err := open(2)
			print(a + b, count)
		}
	"};
	check(src, "3 0");
}
