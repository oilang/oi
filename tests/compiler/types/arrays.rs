use crate::helpers::*;
use indoc::indoc;

#[test]
fn literals() {
	check(
		indoc! {r#"
			a :: [10, 20]
			b :: [30, 40]
			print([1, 2, 3], ["a", "b"], [1, 2,], [2 4 6])
			print((1, [2, 3], "x"), [(1, 2), (3, 4)], [a, b], [a, b][1])
		"#},
		[
			r#"[1, 2, 3] ["a", "b"] [1, 2] [2, 4, 6]"#,
			r#"(1, [2, 3], "x") [(1, 2), (3, 4)] [[10, 20], [30, 40]] [30, 40]"#,
		],
	);
}

#[test]
fn indexing() {
	let src = indoc! {"
		first :: fn(xs: []int) int { xs.0 }
		a :: [10, 20, 30]
		i :: 2
		print(a[1], a[i], a.0, a.len, first([9, 8, 7]))
	"};
	check(src, "20 30 10 3 9");
}

#[test]
fn access_rejections() {
	fail(r#"[1, "two"]"#, "must share a type");
	fail([r#"a :: ["x", "y"]"#, "[1, ..a]"], "must share a type");
	fail("[]", "empty array");
	fail(["x :: 5", "x[0]"], "cannot index");
	fail(r#"a :: [1, 2]; a["x"]"#, "index must be Int");
	fail(["a :: [1, 2]", "a.foo"], "no field `foo`");
	fail(["x :: 5", "x[0..1]"], "cannot slice");
	fail(r#"a :: [1, 2, 3]; a[true..2]"#, "must be Int");
}

#[test]
fn bounds_checks() {
	fail_rt(["a :: [1, 2]", "a[5]"], "out of range");
	fail_rt(["a :: [1, 2, 3]", "a[1..9]"], "out of bounds");
	fail_rt(["a :: [1, 2, 3]", "a[3..1]"], "out of bounds");
	fail_rt(["a := [1, 2]", "a[5] = 9"], "out of range");
	fail_rt(["a: [2]int", "a[5]"], "out of range");
}

#[test]
fn spread() {
	let src = indoc! {"
		a :: [2, 3]
		b :: [5, 6]
		print([..a])
		[1, ..a, ..b, 7]
	"};
	check(src, ["[2, 3]", "[1, 2, 3, 5, 6, 7]"]);
}

#[test]
fn slices() {
	let src = indoc! {"
		a :: [0, 2, 4, 6, 8]
		lo :: 1
		hi :: 4
		assert!(a[1..][0] == 2)
		print(a[1..3], a[..3], a[1..], a[..], a[2..2], a[lo..hi])
	"};
	check(src, "[2, 4] [0, 2, 4] [2, 4, 6, 8] [0, 2, 4, 6, 8] [] [2, 4, 6]");
}

#[test]
fn index_assign() {
	check(["a := [1, 2, 3]", "i := 2", "a[1] = 99", "a[i] = 7", "a"], "[1, 99, 7]");
}

#[test]
fn append_and_extend() {
	// a starts at cap == len == 2
	let src = indoc! {"
		a := [1, 2]
		a << 3
		a << 4
		a << 5
		s :: [1, 2, 3]
		b := s[1..]
		b << 99
		c := [1]
		odd := [1, 3, 5]
		e :: odd[0..0]
		odd << [9, 11]
		odd << e
		f := s[0..0]
		f << [3, 4]
		print(a, b, (c << 2 << 3).len, odd, f)
	"};
	check(src, "[1, 2, 3, 4, 5] [2, 3, 99] 3 [1, 3, 5, 9, 11] [3, 4]");
}

#[test]
fn mutation_rejections() {
	fail(["a :: [1, 2]", "a[0] = 5"], "immutable");
	fail(["x := 5", "x[0] = 1"], "not an array");
	fail(r#"a := [1, 2]; a[0] = "hi""#, "type mismatch");
	fail(["a :: [1, 2]", "a << 3"], "immutable");
	fail([r#"x := "hi""#, "x << 1"], "cannot apply `<<`");
	fail(r#"a := [1, 2]; a << "hi""#, "type mismatch");
	fail(r#"a := [1, 2]; b :: ["x"]; a << b"#, "type mismatch");
}

// value semantics (COW)

#[test]
fn value_semantics() {
	let src = indoc! {"
		id :: fn(a: []int) []int { a }
		a := [1, 2, 3]
		b :: a
		c :: b
		a << 4
		a[0] = 99
		print(a, b, c)
		d :: [1, 2, 3]
		e := d
		e[0] = 99
		t :: e[..]
		e[1] = 99
		q := d[..]
		q[0] = 99
		r := id(d)
		r << 4
		print(d, t)
		inner := [1]
		outer := [[9]]
		outer << inner
		inner << 2
		z: []int
		y := z
		y << 1
		print(outer[1], y)
	"};
	check(
		src,
		["[99, 2, 3, 4] [1, 2, 3] [1, 2, 3]", "[1, 2, 3] [99, 2, 3]", "[1] [1]"],
	);
}

// in operator

#[test]
fn in_operator() {
	check(
		[
			"even :: [0, 2, 4, 6, 8]",
			"a := [1, 2]",
			"a << 3",
			"print(6 in even, 5 in even, 3 in a)",
		],
		"true false true",
	);
	fail("5 in 10", "not an array");
	fail(r#"a :: [1, 2]; "x" in a"#, "type mismatch");
}

#[test]
fn fn_returns_array() {
	let src = indoc! {"
		nums :: fn() []int { [10, 20, 30] }
		a :: nums()
		print(nums(), a[1])
	"};
	check(src, "[10, 20, 30] 20");
}

#[test]
fn fn_return_type_mismatch_array() {
	let src = indoc! {"
		bad :: fn() []int { 42 }
		bad()
	"};
	fail(src, "wrong return type");
}

#[test]
fn if_no_else_array_zero() {
	let src = indoc! {"
		a :: if false { [1, 2, 3] }
		a.len
	"};
	check(src, "0");
}

// fixed-size arrays

#[test]
fn fixed() {
	let src = indoc! {r#"
		N :: 3
		a: [3]int
		n: [N]int
		three: [3]string
		three[0] = "larry"
		three[1] = "curly"
		v: [2]int
		w := v
		v[0] = 9
		print(a, a.len, n.len, three, v.0, w[0])
	"#};
	check(src, r#"[0, 0, 0] 3 3 ["larry", "curly", ""] 9 0"#);
}

#[test]
fn fixed_field_survives_return() {
	check(
		indoc! {"
			T :: struct { w: [4]u64 }
			make :: fn() T { T.{ w = .[1 2 3 4] } }
			t := make()
			print(t.w[0])
			print(t.w[3])
		"},
		["1", "4"],
	);
}

#[test]
fn typed_literals() {
	let src = indoc! {"
		a := int.[1, 2]
		b := a
		a[0] = 9
		c := []int.[3, 4]
		c << 5
		print(int.[1, 2], b[0], [2]int.[3, 4], c, []int.[])
	"};
	check(src, "[1, 2] 1 [3, 4] [3, 4, 5] []");
}

#[test]
fn literal_rejections() {
	fail(r#"a :: int.[1, "x"]"#, "must share a type");
	fail("int.[]", "an exact array literal needs elements");
	fail("[2]int.[3]", "expected 2 elements, got 1");
	fail("a: [3]int = .[1 2]", "expected 3 elements, got 2");
	fail("a := .[]", "cannot infer the element type");
}

// anon array literals

#[test]
fn anon_literals() {
	let src = indoc! {"
		three :: fn(xs: [3]int) int { xs.0 }
		a: [3]int = .[1 2 3]
		d: []int = .[1 2]
		primes := .[2 3 5 7]
		f: [2]int = .[1 2]
		g := f
		f[0] = 9
		print(a, d, three(.[1 2 3]), primes.len, g[0])
	"};
	check(src, "[1, 2, 3] [1, 2] 1 4 1");
}

// fixed <-> dynamic

#[test]
fn fixed_coerces_to_dynamic() {
	let src = indoc! {"
		total :: fn(xs: []int) int { xs.len }
		a := i32.[1, 2]
		b: []i32 = a
		b << 3
		print(b, a, total(int.[1, 2, 3]))
	"};
	check(src, "[1, 2, 3] [1, 2] 3");
}

#[test]
fn in_compares_structurally() {
	check(
		indoc! {"
			P :: struct { x: int }
			print(P.{1} in [P.{2}, P.{1}])
			print((1, 2) in [(3, 4)])
		"},
		["true", "false"],
	);
}

#[test]
fn equality_is_structural() {
	check(
		indoc! {"
			print([1, 2, 3] == [1, 2, 3])
			print([1, 2] == [1, 2, 3])
			print([[1, 2], [3]] == [[1, 2], [3]])
		"},
		["true", "false", "true"],
	);
}

#[test]
fn ordering_rejected() {
	fail("[1, 2] < [1, 3]", "only `==` and `!=`");
}
