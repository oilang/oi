//! Leak checks.
//! Every allocation a program makes is freed by exit.

use crate::helpers::*;

#[test]
fn copies_and_cow() {
	assert_clean(indoc! {"
		a := [1, 2, 3]
		b :: a
		c :: b
		a << 4
		print(a)
		print(c)
	"});
}

#[test]
fn slices() {
	assert_clean([
		"a :: [1, 2, 3, 4]",
		"b :: a[1..3]",
		"print(b)",
		r#"s :: "hello""#,
		"print(s[1..3])",
	]);
}

#[test]
fn fn_call_and_return() {
	assert_clean(indoc! {"
		make :: fn() []int { [1, 2] }
		id :: fn(a: []int) []int { a }
		x :: id(make())
		print(x)
	"});
}

#[test]
fn loop_temps_and_binds() {
	assert_clean(indoc! {"
		i := 0
		loop i < 100 {
			t :: [i, i]
			i = t[1] + 1
		}
		total := 0
		loop e in [10, 20, 30] {
			total = total + e
		}
		print(i, total)
	"});
}

#[test]
fn logical_short_circuit_skips_allocating_rhs() {
	assert_clean(indoc! {r#"
		names :: fn() []string { ["a", "b"] }
		i := 0
		loop i < 5 {
			x :: i > 2
			if !x && ("q" in names()) { }
			i = i + 1
		}
	"#});
}

#[test]
fn early_return() {
	assert_clean(indoc! {"
		f :: fn(n: int) int {
			a :: [1, 2, 3]
			if n > 1 { return a[0] }
			a[1]
		}
		print(f(5))
	"});
}

#[test]
fn break_and_continue() {
	assert_clean(indoc! {"
		i := 0
		loop {
			i = i + 1
			if i == 3 { continue }
			xs :: [i]
			if xs[0] > 5 { break }
		}
		print(i)
	"});
}

#[test]
fn map_set_delete_copy() {
	assert_clean(indoc! {r#"
		m := ["a" = 1]
		n :: m
		m["b"] = 2
		m.delete["a"]
		print(n["a"])
	"#});
}

#[test]
fn reassign_and_shadow() {
	assert_clean(indoc! {"
		a := [1]
		a = [2, 3]
		b :: [4]
		b :: [5]
		print(a)
		print(b)
	"});
}

#[test]
fn branch_merges() {
	assert_clean(indoc! {"
		x :: if true { [1] } else { [2] }
		y :: match 2 { 1 => [9], 2 => [4, 5], else => [0] }
		print(x)
		print(y)
	"});
}

#[test]
fn buffer_owns_its_elements() {
	assert_clean(indoc! {"
		P :: struct { a: int }
		p :: [P.{a = 1}]
		a := [[1]]
		b := a
		b << [2]
		print(p[0].a, a, b)
	"});
}

#[test]
fn defer_releases_after_the_body_runs() {
	assert_clean(indoc! {"
		f :: fn(early: bool) int {
			xs :: [ 1 2 3 ]
			defer print(xs[0])
			if early { return 0 }
			xs[1]
		}
		print(f(true))
		print(f(false))
	"});
}

#[test]
fn struct_literal_owns_its_field_handles() {
	assert_clean(indoc! {"
		Bag :: struct { items: []int }
		s :: Bag.{ items = [ 1 2 ] }
		print(s.items[0])
	"});
	let plain = "Bag :: struct { n: int }\nmk :: fn() Bag { Bag.{ n = 1 } }\nprint(mk().n)";
	let held = "Bag :: struct { items: []int }\nmk :: fn() Bag { Bag.{ items = [ 1 2 ] } }\nprint(mk().items[0])";
	assert_clean(plain);
	assert_clean(held);
}

#[test]
fn struct_owns_its_nested_structs() {
	assert_clean(indoc! {"
		I :: struct { n: int }
		B :: struct { i: I, j: I }
		f :: fn() {
			b := B.{ i = I.{ 1 }, j = I.{ 2 } }
			b.j = I.{ 3 }
			print(b.i.n)
		}
		f()
	"});
}

#[test]
fn tuple_frees_its_struct_elements() {
	let held = indoc! {"
		File :: struct { fd: int }
		File : Drop < { drop :: fn(mut self) {} }
		P :: struct { n: int }
		t :: (File.{fd = 1}, P.{n = 2})
	"};
	assert_clean(held);
	assert_clean(["t := ([1], 2)", "u := t", "u.0 = [3]", "print(t)"]);
}

#[test]
fn omitted_handle_field_has_one_owner() {
	assert_clean(["Bag :: struct { items: []int }", "b := Bag.{}", "print(b.items.len)"]);
}

#[test]
fn closure_env_releases_moved_captures() {
	assert_clean([
		"ys :: [4]",
		"g :: fn [move ys] () int { ys[0] }",
		"h :: g",
		"print(h())",
	]);
}

#[test]
fn boxed_enums_free_their_box() {
	assert_clean(indoc! {"
		E :: enum { a, b([]int) }
		e := E.b.([1 2])
		f := e
		o: ?int = 3
		S :: struct { o: ?int }
		s := S.{ o = 4 }
		t := s
		r :: fn() !int { 4 }
		x := r()
		P :: struct { n: int }
		p: ?P = P.{ n = 1 }
		print(f, o, t, x)
	"});
}
