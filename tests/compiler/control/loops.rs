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
		loop i in 0.."hi".len { print("hi"[i]) }
		sum := 0
		loop (k, v) in [ "a" = 1, "bb" = 2 ] {
			sum += k.len * v
		}
		sum
	"#};
	check(src, ["104", "105", "5"]);
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
		xs := [1, 5, 20]
		loop x in xs { if x > 9 do break x } or -1
	"};
	check(hit, "20");

	let miss = indoc! {"
		xs := [1, 5, 9]
		loop x in xs { if x > 9 do break x } or -1
	"};
	check(miss, "-1");
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
	"};
	check(src, ["0", "1", "2", "0", "1", ":done"]);
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
