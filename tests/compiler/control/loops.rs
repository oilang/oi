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
fn break_exits() {
	let src = indoc! {"
		i := 0
		loop {
			i = i + 1
			if i == 3 { break }
		}
		i
	"};
	check(src, "3");
}

#[test]
fn continue_skips() {
	let src = indoc! {"
		sum := 0
		i := 0
		loop {
			i = i + 1
			if i > 10 { break }
			if i % 2 == 1 { continue }
			sum = sum + i
		}
		sum
	"};
	check(src, "30");
}

#[test]
fn break_targets_innermost() {
	let src = indoc! {"
		outer := 0
		loop {
			outer = outer + 1
			inner := 0
			loop {
				inner = inner + 1
				if inner == 2 { break }
			}
			if outer == 3 { break }
		}
		outer
	"};
	check(src, "3");
}

#[test]
fn break_outside_loop() {
	fail_with("break", "outside of a loop");
}

#[test]
fn continue_outside_loop() {
	fail_with("continue", "outside of a loop");
}

#[test]
fn while_counts() {
	let src = indoc! {"
		i := 0
		loop i < 5 {
			i = i + 1
		}
		i
	"};
	check(src, "5");
}

#[test]
fn while_never_enters() {
	let src = indoc! {"
		i := 10
		loop i < 5 {
			i = i + 1
		}
		i
	"};
	check(src, "10");
}

#[test]
fn while_condition_must_be_bool() {
	fail_with("loop 3 { }", "must be Bool");
}

// loops over ranges

#[test]
fn for_range_sums() {
	let src = indoc! {"
		sum := 0
		loop i in 0..5 {
			sum = sum + i
		}
		sum
	"};
	check(src, "10");
}

#[test]
fn for_range_excludes_end() {
	let src = indoc! {"
		loop i in 0..3 { print(i) }
	"};
	check(src, ["0", "1", "2"]);
}

#[test]
fn for_range_empty() {
	let src = indoc! {"
		sum := 99
		loop i in 3..3 {
			sum = 0
		}
		sum
	"};
	check(src, "99");
}

#[test]
fn for_range_continue_advances() {
	let src = indoc! {"
		sum := 0
		loop i in 0..6 {
			if i % 2 == 1 { continue }
			sum = sum + i
		}
		sum
	"};
	check(src, "6");
}

#[test]
fn for_var_is_scoped() {
	fail_with(["loop i in 0..3 { i }", "i"], "undefined variable");
}

// loops over iterables

#[test]
fn for_each_sums() {
	let src = indoc! {"
		sum := 0
		loop x in [2, 4, 6, 8] {
			sum = sum + x
		}
		sum
	"};
	check(src, "20");
}

#[test]
fn slice_iterates_its_window() {
	let src = indoc! {"
		a :: [0, 2, 4, 6, 8]
		sum := 0
		loop x in a[1..4] {
			sum = sum + x
		}
		sum
	"};
	check(src, "12");
}

#[test]
fn for_each_tuple_destructure() {
	let src = indoc! {"
		sum := 0
		loop (x, y) in [(0, 0), (1, 2), (3, 4)] {
			sum = sum + x + y
		}
		sum
	"};
	check(src, "10");
}

#[test]
fn for_each_iterable_must_be_array() {
	fail_with("loop x in 5 { x }", "not iterable");
}

#[test]
fn for_range_bound_must_be_int() {
	fail_with("loop i in 0..true { i }", "must be Int");
}

#[test]
fn for_tuple_pattern_on_non_tuple() {
	fail_with("loop (x, y) in [1, 2, 3] { x }", "destructure");
}

#[test]
fn for_tuple_pattern_wrong_field_count() {
	fail_with("loop (x, y, z) in [(1, 2)] { x }", "fields");
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
fn for_struct_and_array_patterns() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		loop Point.{ x } in [Point.{ 1, 2 }, Point.{ 3, 4 }] { print(x) }
		loop [a b] in [[1 2] [3 4]] { print(a + b) }
	"};
	check(src, ["1", "3", "3", "7"]);
}

#[test]
fn for_each_string_bytes_and_map_entries() {
	let src = indoc! {r#"
		loop b in "hi" { print(b) }
		sum := 0
		loop (k, v) in ["a" = 1, "bb" = 2] { sum += k.len * v }
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
	fail_with("x := break", "never produce a value");
	fail_with("loop { x := continue }", "never produce a value");
	fail_with("loop { if true { break 1 } else { break } }", "mismatched types");
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
fn do_bodies() {
	let src = indoc! {"
		i := 0
		loop i < 3 do i += 1
		loop n in 0..2 do print(n)
		i
	"};
	check(src, ["0", "1", "3"]);
}
