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
fn producers_by_consumer() {
	let src = indoc! {r#"
		find :: fn(n: int) ?int { if n > 0 do n else do none }
		parse :: fn(n: int) !int { if n > 0 do n else do error("bad") }
		open :: fn(k: int) (n: int, err: ?Error) {
			if k > 0 do k,
			none else do 0,
			error("shut")
		}
		hits :: fn(m: [string]int, a: any) ?int {
			find(1)? + m["a"]? + int.(a)?
		}
		miss :: fn(m: [string]int) ?int do m["z"]?
		m := ["a" = 1]
		a: any = 5
		loop {
			w := m["a"] or break
			x := int.(a) or break
			y := parse(2) or break
			z := open(2) or break
			print(w + x + y + z)
			v := m["z"] or break
			print("no")
		}
		loop v := m["z"] do print("no")
		loop v := parse(0) do print("no")
		loop v := open(0) do print("no")
		loop v := int.(a) {
			print(v)
			break
		}
		print(hits(m, a), miss(m))
		print(
			m["z"] and $ or -1,
			int.(a) and $ * 2 or 0,
			find(2) and $ or -1,
			parse(0) and $ or -1,
			open(2) and $ or 0,
		)
		x := find(2)
		y := parse(0)
		v, ok := find(0) or find(0)
		print(
			x,
			y,
			find(0) or find(3),
			m["z"] or m["a"],
			v,
			ok,
		)
	"#};
	check(
		src,
		[
			"10",
			"5",
			"some.(7) none",
			"-1 10 2 -1 2",
			r#"some.(2) err.("bad") some.(3) 1 0 false"#,
		],
	);
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
fn tuple_and_enum_payloads_ride_packs() {
	// the tuple keeps its own block, the `?T` box is gone
	let src = indoc! {r#"
		count := 0
		@c counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc do count += 1
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}
		Dir :: enum { up, down }
		Shape :: enum { circle(int), square(int) }
		Turn :: trait { turn : fn(self, k: int) ?Dir }
		Wheel :: struct {}
		Wheel : Turn < {
			turn :: fn(self, k: int) ?Dir {
				if k > 0 do .up else do none
			}
		}
		span :: fn(k: int) ?(int, int) {
			if k > 0 do (k, -k) else do none
		}
		parse :: fn(k: int) !(int, string) {
			if k > 0 do (k, "ok") else do error("bad")
		}
		shape :: fn(k: int) ?Shape {
			if k > 0 do .circle.(k) else do none
		}
		main :: fn() {
			wheel : Turn : Wheel.{}
			call := span
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr.(0) } }
			lo, hi := call(2) or (0, 0)
			dir := wheel.turn(1) or .down
			print(lo + hi, dir, count)
			pair, ok := span(3)
			print(pair.1, ok, wheel.turn(0), parse(0), shape(6), shape(0) or .square.(1))
		}
	"#};
	check(
		src,
		["0 up 1", r#"-3 true none err.("bad") some.(circle.(6)) square.(1)"#],
	);
}

#[test]
fn fallible_returns_allocate_nothing_on_success() {
	let src = indoc! {r#"
		count := 0
		@c counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc do count += 1
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}
		open :: fn(k: int) (n: int, err: ?Error) { k, none }
		parse :: fn(k: int) !int {
			if k > 0 do k else do error("bad")
		}
		half :: fn(k: int) ?int {
			if k % 2 == 0 do k / 2 else do none
		}
		main :: fn() {
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr.(0) } }
			a := open(1) or 0
			b, err := open(2)
			c := parse(3) or 0
			d, bad := parse(4)
			e := half(10) or 0
			f, ok := half(12)
			print(a + b + c + d + e + f, count)
		}
	"#};
	check(src, "21 0");

	assert_clean(indoc! {"
		arr :: fn(k: int) ?[]int {
			if k > 0 do .[k] else do none
		}
		fwd :: fn(k: int) ?[]int { a := arr(k)?; a }
		print(fwd(0) or .[], fwd(1))
	"});
	fail("g :: fn() ?int { x: !int = 1; x? }; g()", "returning `!T`, found ?int");
}

#[test]
fn header_binds_allocate_nothing() {
	let src = indoc! {r#"
		count := 0
		@c counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc { count += 1 }
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}
		parse :: fn(k: int) !int {
			if k > 0 do k else do error("bad")
		}
		half :: fn(k: int) ?int {
			if k % 2 == 0 do k / 2 else do none
		}
		main :: fn() {
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr.(0) } }
			sum := 0
			if v := half(10) do sum += v
			if v := half(3) do sum += 100 else do sum += 1
			if v := parse(2) do sum += v
			k := 8
			loop v := half(k) {
				sum += v
				k = v
			}
			print(sum, count)
		}
	"#};
	check(src, "15 0");

	let drain = indoc! {r#"
		use sync
		ch := sync.Chan[string].{ cap = 4 }
		loop i in 1..4 {
			ch <- "m{i}" or panic!("closed")
		}
		ch.close()
		out := ""
		loop msg := ch.recv() do out += msg
		print(out)
	"#};
	check(drain, "m1m2m3");
	assert_clean(drain);
}

#[test]
fn iterator_next_allocates_nothing() {
	let src = indoc! {r#"
		count := 0
		@c counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc do count += 1
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}
		Countdown :: struct { n: int }
		Countdown : Iterator[int] < {
			next :: fn(mut self) ?int {
				if self.n == 0 do return none
				self.n -= 1
				self.n
			}
		}
		main :: fn() {
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr.(0) } }
			sum := 0
			loop k in Countdown.{ n = 5 } { sum += k }
			print(sum, count)
		}
	"#};
	check(src, "10 0");

	assert_clean(indoc! {r#"
		Words :: struct { n: int }
		Words : Iterator[string] < {
			next :: fn(mut self) ?string {
				if self.n == 0 do return none
				self.n -= 1
				"w{self.n}"
			}
		}
		out := ""
		loop w in Words.{ n = 3 } {
			if w == "w0" do break
			out += w
		}
		print(out)
	"#});
}

#[test]
fn custom_error_returns_allocate_nothing() {
	let src = indoc! {"
		MyErr :: enum { bad, worse }
		count := 0
		@c counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc do count += 1
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}
		parse :: fn(k: int) MyErr!int {
			if k > 0 do k else if k < 0 do .bad else do error(.worse)
		}
		twice :: fn(k: int) MyErr!int {
			v := parse(k)?; v * 2
		}
		main :: fn() {
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr.(0) } }
			a := parse(1) or 0
			b, err := parse(0)
			c := twice(2) or 0
			allocs := count
			print(a + b + c, err, twice(-1), allocs)
		}
	"};
	check(src, "5 some.(worse) err.(bad) 0");
}

#[test]
fn keyed_loop_reuses_its_key() {
	let src = indoc! {r#"
		count := 0
		@c counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc do count += 1
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}
		Three :: struct {}
		Three : Keyed[int, int] < {
			pairs :: fn(self; after: ?int) ?(int, int) {
				i :: (after or -1) + 1
				if i < 3 do (i * 10, i) else do none
			}
		}
		main :: fn() {
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr.(0) } }
			sum := 0
			loop x, i in Three.{} do sum += x + i
			print(sum, count)
		}
	"#};
	check(src, "33 5");
}
