use crate::helpers::*;

#[test]
fn counts_to_a_return() {
	let src = indoc! {"
		i := 0
		loop {
			i = i + 1
			if i == 3 { return i }
		}
	"};
	check(src, "3");
}

#[test]
fn break_and_continue() {
	let src = indoc! {"
		sum := 0
		i := 0
		loop {
			i = i + 1
			if i > 10 { break }
			if i % 2 == 1 { continue }
			inner := 0
			loop {
				inner = inner + 1
				if inner == 2 { break }
			}
			sum = sum + i
		}
		sum + i
	"};
	check(src, "41");
	check(["i := 0", "loop i < 5 { if i == 3 { break }", "i += 1 }", "i"], "3");
	fail("break", "outside of a loop");
	fail("continue", "outside of a loop");
}

#[test]
fn while_loops() {
	let src = indoc! {"
		i := 0
		loop i < 5 { i = i + 1 }
		j := 10
		loop j < 5 { j = j + 1 }
		print(i)
		print(j)
	"};
	check(src, ["5", "10"]);
}

#[test]
fn loop_block_header() {
	let src = indoc! {"
		i := 0
		loop { i += 1; i < 3 } do print(i)
		loop { n := i; n < 5 } { print(n); i += 1 }
		loop { i += 1; if i > 6 { break } }
		{ print(i) }
	"};
	check(src, ["1", "2", "3", "4", "7"]);
}

// loops over ranges

#[test]
fn for_range() {
	let src = indoc! {"
		loop i in 0..3 { print(i) }
		sum := 99
		loop i in 3..3 { sum = 0 }
		print(sum)
		sum = 0
		loop i in 0..6 {
			if i % 2 == 1 { continue }
			sum = sum + i
		}
		print(sum)
	"};
	check(src, ["0", "1", "2", "99", "6"]);
}

#[test]
fn loop_header_errors() {
	fail("loop 3 {}", "cannot iterate");
	fail(["loop i in 0..3 { i }", "i"], "undefined variable");
	fail("loop x in 5 { x }", "not iterable");
	fail("loop i in 0..true { i }", "must be Int");
	fail("loop (x, y) in [1, 2, 3] { x }", "destructure");
	fail("loop (x, y, z) in [(1, 2)] { x }", "fields");
}

// loops over iterables

#[test]
fn for_each_sums() {
	let src = indoc! {"
		a :: [0, 2, 4, 6, 8]
		sum := 0
		loop x in a { sum = sum + x }
		loop x in a[1..4] { sum = sum + x }
		sum
	"};
	check(src, "32");
}

#[test]
fn for_each_bind_is_independent_copy() {
	let src = indoc! {"
		outer :: [[1], [2]]
		got := [0]
		loop x in outer {
			got = x
		}
		got << 99
		outer[1]
	"};
	check(src, "[2]");
}

#[test]
fn for_struct_tuple_and_array_patterns() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		loop Point.{ x } in [Point.{ 1, 2 }, Point.{ 3, 4 }] { print(x) }
		loop [a b] in [[1 2] [3 4]] { print(a + b) }
		loop (x, y) in [(0, 0), (1, 2)] { print(x + y) }
	"};
	check(src, ["1", "3", "3", "7", "0", "3"]);
}

#[test]
fn for_each_string_bytes_and_map_entries() {
	let src = indoc! {r#"
		loop i in 0.."hi".len() { print("hi"[i]) }
		sum := 0
		loop v, k in [ "a" = 1, "bb" = 2 ] {
			sum += k.len() * v
		}
		sum
	"#};
	check(src, ["104", "105", "5"]);
}

#[test]
fn keyed_loops() {
	let src = indoc! {r#"
		i := 1
		m := ["a{i}" = "x{i}"]
		loop v in m do print(v)
		loop v, k in m do print(k, v)
		loop x, i in ["p" "q"] do print(x, i)
	"#};
	check(src, ["x1", "a1 x1", "p 0", "q 1"]);
	assert_clean(src);
	fail(r#"loop (k, v) in ["a" = 1] do k"#, "loop v, k in m");
	fail("loop x, i in 0..2 do x", "not keyed");
}

#[test]
fn mut_binds_write_back() {
	let src = indoc! {r#"
		P :: struct { a: int }
		ps := [P.{1} P.{2}]
		qs := ps
		loop with mut p in ps do a *= 10
		Q :: struct { m: [string]int }
		q := Q.{ m = ["ab" = 1] }
		loop mut v, k in q.m do v += k.len()
		print(ps, qs, q.m)
	"#};
	check(src, r#"[P.{a = 10}, P.{a = 20}] [P.{a = 1}, P.{a = 2}] ["ab" = 3]"#);
	assert_clean(src);

	let src = indoc! {r#"
		P :: struct { a: int }
		rows: @soa []P
		rows << P.{1}
		loop mut row in rows do row.a += 1
	"#};
	check([src, "print(rows.a)"], "[2]");
	fail(["xs := [1]", "loop mut x, mut i in xs {}"], "the key is not a place");
}

#[test]
fn keyed_protocol() {
	let src = indoc! {r#"
		Ring :: struct { items: []int, start: int }
		Ring : Keyed[string, int] < {
			Cursor :: int
			pairs :: fn(self; after: ?int) ?(int, int) {
				i :: (after or -1) + 1
				if i >= self.items.len() do return none
				(self.items[(self.start + i) % self.items.len()], i)
			}
			key :: fn(self; at: int) string { "k{at}" }
		}
		ring := Ring.{ [1 2 3], 1 }
		loop x, k in ring do print(k, x)
		loop x in ring do x
	"#};
	check(src, ["k0 2", "k1 3", "k2 1", "[2, 3, 1]"]);
	assert_clean(src);
	fail(
		[src, "loop move x in ring {}"],
		"`loop move` needs an array, map or fixed array",
	);
}

// breaks with values

#[test]
fn infinite_loop_break_value() {
	let src = indoc! {"
		i := 0
		n := loop {
			i += 1
			if i == 3 { break i * 2 }
		}
		n
	"};
	check(src, "6");
}

#[test]
fn for_loop_break_value_or_else() {
	let hit = indoc! {"
		xs := [1 5 20]
		loop x in xs { if x > 9 do break x } or -1
	"};
	check(hit, "20");

	let miss = indoc! {"
		xs := [1 5 9]
		loop x in xs { if x > 9 do break x } or -1
	"};
	check(miss, "-1");

	let bare = indoc! {"
		xs := [1 5 20]
		loop x in xs {
			if x == 5 do break
			if x > 9 do break x
		} or -1
	"};
	check(bare, "-1");
}

#[test]
fn loop_collects_tails() {
	let src = indoc! {r#"
		print(loop x in [1 2 3] do x ** 2)
		print(loop x in [1 2 3 4] { if x == 3 do continue; x ** 2 })
		loop x in ["a" "b" "c"] { if x == "c" do break; "{x}!" }
	"#};
	check(src, ["[1, 4, 9]", "[1, 4, 16]", r#"["a!", "b!"]"#]);
	assert_clean(src);
}

#[test]
fn nested_tail_loop_collects_nothing() {
	let src = indoc! {"
		count := 0
		counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc do count += 1
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}
		main :: fn() {
			xs := [1 2 3]
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr.(0) } }
			sum := 0
			loop x in xs {
				loop y in xs do sum += x * y
			}
			print(sum, count)
		}
	"};
	check(src, "36 0");
}

#[test]
fn break_value_errors() {
	fail("x := break", "never produce a value");
	fail("loop { x := continue }", "never produce a value");
	fail("fn() int { 1 + return 0 }", "never produce a value");
	fail("loop { if true { break 1 } else { break } }", "mismatched types");
}

#[test]
fn custom_iterators() {
	let src = indoc! {"
		Countdown :: struct { n: int }
		Countdown : Iterator[int] < {
			next :: fn(mut self) ?int {
				if self.n <= 0 { return none }
				self.n -= 1
				self.n
			}
		}
		loop x in Countdown.{ n = 3 } { print(x) }
		r :: 0..2
		loop n in r { print(n) }
		loop n in r { print(n) }
	"};
	check(src, ["2", "1", "0", "0", "1", "0", "1"]);
}

#[test]
fn loop_match() {
	let src = indoc! {"
		a := 0..2
		loop match a.next() {
			.some.(n) => print(n),
		}
		b := 0..2
		out :: loop match b.next() {
			.none => break :done,
			.some.(x) => print(x),
		}
		print(out)
	"};
	check(src, ["0", "1", "0", "1", ":done"]);
}

#[test]
fn header_bind() {
	let src = indoc! {"
		it := 0..3
		loop .some.(n) := it.next() { print(n) }
		it2 := 0..2
		loop n := it2.next() { print(n) }
		print(:done)
		print(loop a := 1, b := 2 { break a + b } + 1)
		o : ?int = 3
		print(loop a := 1; b := o { break a + b })
	"};
	check(src, ["0", "1", "2", "0", "1", ":done", "4", "some.(4)"]);
}

#[test]
fn header_bind_then_test() {
	let src = indoc! {"
		it := 0..9
		loop n := it.next(), n < 2 { print(n) }
		print(loop n := it.next(); n < 5 do n * 10)
	"};
	check(src, ["0", "1", "[30, 40]"]);
}

#[test]
fn zip_clauses() {
	let src = indoc! {r#"
		xs := ["a" "b" "c" "d"]
		q := 5..7
		loop x in xs, n := q.next() { print(x, n) }
		print(loop x in xs, n in 1..9, x != "c" do "{x}{n}")
		it := 0..9
		print(loop n := it.next() { if n > 3 do break n })
		print(loop x in [1 5 9], x < 3 { if x == 5 do break x })
		v := loop x := 5 do break x; print(v)
	"#};
	check(src, ["a 5", "b 6", r#"["a1", "b2"]"#, "some.(4)", "none", "5"]);
	assert_clean(src);
}

#[test]
fn nest_clauses() {
	let src = indoc! {"
		print(loop y in ..2; x in ..3 do (x, y))
		count :: fn(y: int) ?int {
			if y < 3 do return y; none
		}
		loop y in [1 2 5 2]; n := count(y); x in ..n { print(y, x) }
		loop y in 0..2; x in 0..2 { if x == y do continue; print(y, x) }
		print(loop y in 0..3; x in 0..3 { if y == 1 do break x })
	"};
	let grid = "[(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)]";
	check(src, [grid, "1 0", "2 0", "2 1", "0 1", "1 0", "some.(0)"]);
}

#[test]
fn do_bodies() {
	let src = indoc! {"
		i := 0
		loop i < 3 do i += 1
		loop n in 0..2 do print(n)
		i
	"};
	check(src, ["0", "1", "3"]);
}

#[test]
fn loop_header_iterates_without_binding() {
	let src = indoc! {"
		n := 0
		loop 2..4 { n += 10 }
		loop [1 2 3] { n += 1 }
		n
	"};
	check(src, "23");
}

#[test]
fn labeled_block_breaks_or_yields_tail() {
	let src = indoc! {"
		f :: fn(bad: bool) int {
			:blk {
				if bad do break :blk 0
				42
			}
		}
		print(f(true), f(false))
	"};
	check(src, "0 42");
}

#[test]
fn continue_outer_label() {
	let src = indoc! {"
		loop :outer x in [1 2 3] {
			loop y in [1 2 3] {
				if y == x do continue :outer
				print(x, y)
			}
		}
	"};
	check(src, ["2 1", "3 1", "3 2"]);
}

#[test]
fn discard_yields_unit() {
	let src = indoc! {r#"
		x := discard 5
		print("{x}")
	"#};
	check(src, "()");
}
