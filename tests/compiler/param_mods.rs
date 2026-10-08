use crate::helpers::*;

#[test]
fn inout() {
	check(
		indoc! {r#"
			C :: struct { n: int }
			C :< {
				take :: fn(self, mut xs: []int) { xs << self.n }
				bump :: fn(mut self) { self.n = self.n + 1 }
			}
			push9 :: fn(mut xs: []int) { xs << 9 }
			swap :: fn(mut xs: []int) { xs = [7, 8] }
			setk :: fn(mut m: [string]int) { m["k"] = 1 }
			bump :: fn(mut c: C) { c.n = c.n + 1 }
			push[T] :: fn(mut xs: []T, v: T) []T {
				xs << v
				xs
			}
			a := [1]
			b := [1]
			g := [1]
			m := ["a" = 0]
			c := C.{n = 1}
			push9(mut a)
			swap(mut b)
			setk(mut m)
			bump(mut c)
			c.bump()
			push(mut g, 9)
			C.{n = 7}.take(mut g)
			print(a, b, m["k"], c.n, g)
		"#},
		"[1, 9] [7, 8] 1 3 [1, 9, 7]",
	);
}

#[test]
fn slice_projection_element_write() {
	check(
		indoc! {"
			set :: fn(mut a: []int) { a[0] = 9 }
			xs := [1, 2, 3, 4]
			set(mut xs[1..3])
			xs
		"},
		"[1, 9, 3, 4]",
	);
}

#[test]
fn slice_projection_is_leak_free() {
	// the callee frees the copy, the caller frees its replacement
	assert_clean(indoc! {"
		swap :: fn(mut a: []int) { a = [7, 8] }
		xs := [1, 2, 3, 4]
		swap(mut xs[1..3])
		print(xs)
	"});
}

#[test]
fn inout_is_leak_free() {
	assert_clean(indoc! {"
		push9 :: fn(mut xs: []int) { xs << 9 }
		swap :: fn(mut xs: []int) { xs = [7, 8] }
		a := [1]
		push9(mut a)
		swap(mut a)
		print(a)
	"});
}

// errors

#[test]
fn missing_mut_at_callsite() {
	fail(
		["f :: fn(mut xs: []int) {}", "a := [1]", "f(a)"],
		"missing `mut` at the callsite",
	);
}

#[test]
fn mut_on_non_mut_param() {
	fail(["f :: fn(xs: []int) {}", "a := [1]", "f(mut a)"], "not `mut`");
}

#[test]
fn immutable_binding_lent() {
	fail(["f :: fn(mut xs: []int) {}", "a :: [1]", "f(mut a)"], "immutably bound");
	fail(
		["f :: fn(mut a: []int) {}", "xs :: [1, 2, 3]", "f(mut xs[1..3])"],
		"immutably bound",
	);
}

#[test]
fn non_place_lent() {
	fail(["f :: fn(mut xs: []int) {}", "f(mut [1, 2])"], "only a mutable binding");
}

#[test]
fn exclusivity() {
	for src in [
		indoc! {"
			f :: fn(mut xs: []int, ys: []int) {}
			a := [1]
			f(mut a, a)
		"},
		indoc! {"
			f :: fn(mut xs: []int, n: int) {}
			a := [1]
			f(mut a, a[0])
		"},
		indoc! {"
			f :: fn(mut a: []int, b: int) {}
			xs := [1, 2, 3]
			f(mut xs[1..3], xs[0])
		"},
		indoc! {"
			C :: struct { xs: []int }
			C :< { take :: fn(self, mut xs: []int) {} }
			c := C.{xs = [1]}
			c.take(mut c)
		"},
	] {
		fail(src, "while it is lent `mut`");
	}
}

#[test]
fn mut_self_needs_mut_binding() {
	fail(
		indoc! {"
			C :: struct { n: int }
			C :< { bump :: fn(mut self) { self.n = self.n + 1 } }
			c :: C.{n = 1}
			c.bump()
		"},
		"needs a `mut` binding",
	);
}

#[test]
fn slice_projection_length_change_panics() {
	fail_rt(
		indoc! {"
			grow :: fn(mut a: []int) { a = [7, 8, 9] }
			xs := [1, 2, 3, 4]
			grow(mut xs[1..3])
		"},
		"projection changed length",
	);
}

#[test]
fn scalar_inout() {
	check(
		indoc! {"
			bump :: fn(mut n: int, by: int) { n += by }
			half :: fn(mut x: f32) { x /= 2.0 }
			twice :: fn(mut n: int, f: fn(mut int, int)) {
				f(mut n, 1)
				f(mut n, 1)
			}
			n := 1
			twice(mut n, bump)
			x : f32 = 3.0
			half(mut x)
			print(n, x)
		"},
		"3 1.5",
	);
}

#[test]
fn unlendable_mut_param_rejected() {
	fail("f :: fn(mut s: string) {}", "has no address to lend");
}

#[test]
fn callee_cannot_mutate_plain_param() {
	fail(["f :: fn(xs: []int) { xs << 1 }", "f([1])"], "immutable");
}

#[test]
fn mut_fn_typed_param_lends_through_callback() {
	check(
		indoc! {"
			apply :: fn(mut xs: []int, f: fn(mut []int) int) int { f(mut xs) }
			g :: fn(mut ys: []int) int { ys[0] = 42
				0
			}
			a := [1, 2]
			apply(mut a, g)
			a
		"},
		"[42, 2]",
	);
}

#[test]
fn mut_closure_rejected_for_plain_fn_param() {
	let src = indoc! {"
		h :: fn(f: fn([]int) int) int { 0 }
		g :: fn(mut ys: []int) int { 0 }
		h(g)
	"};
	fail(src, "wrong argument type");
}
