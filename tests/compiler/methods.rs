use crate::helpers::*;

#[test]
fn instance_method() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< {
			sum :: fn(self) int { self.x + self.y }
			scaled :: fn(self, k: int) int { (self.x + self.y) * k }
		}
		p :: Point.{3, 4}
		f :: p.scaled
		print(p.sum(), Point.{3, 4}.sum(), Point.{3, 4}.scaled(10), f(10))
	"};
	check(src, "7 7 70 70");
}

#[test]
fn static_method() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< {
			origin :: fn() Point { Point.{0, 0} }
			make :: fn(a: int, b: int) Point { Point.{a, b} }
			new :: fn() Self { Self.{} }
			add :: fn(self, other: Self) Self { Self.{self.x + other.x, self.y + other.y} }
			sum :: fn(self) int { self.x + self.y }
			zero :: Point.{0, 0}
		}
		print(Point.origin().sum(), Point.make(3, 4).x, Point.new().sum())
		print(Point.{1, 2}.add(Point.{3, 4}).x, Point.zero.x)
	"};
	check(src, ["0 3 0", "4 0"]);
}

#[test]
fn qualified_mut_self() {
	let src = indoc! {"
		S :: struct { n: int }
		S :< { bump :: fn(mut self) { self.n = self.n + 1 } }
		B[T] :: struct { v: T }
		B[T] :< { set :: fn(mut self, v: T) { self.v = v } }
		f[T] :: fn(mut x: T) { T.bump(mut x) }
		s := S.{n = 4}
		S.bump(mut s)
		f(mut s)
		b := B.{v = 1}
		B.set(mut b, 7)
		r := 0..3
		print(s.n, b.v, Range.next(mut r), r.next())
	"};
	check(src, "6 7 some.(0) some.(1)");
}

#[test]
fn self_outside_impl() {
	fail("Self.{}", "no enclosing impl");
}

#[test]
fn immutable_self_rejects_field_assign() {
	fail(
		indoc! {"
			P :: struct { x: int }
			P :< { bad :: fn(self) { self.x = 9 } }
			P.{1}.bad()
		"},
		"immutably bound",
	);
}

#[test]
fn no_such_method() {
	fail(
		indoc! {"
			P :: struct { x: int }
			p :: P.{1}
			p.nope()
		"},
		"no method `nope`",
	);
}

#[test]
fn wrong_arg_count() {
	fail(
		indoc! {"
			P :: struct { x: int }
			P :< { add :: fn(self, k: int) int { self.x + k } }
			P.{1}.add()
		"},
		"expects 1 argument",
	);
}

#[test]
fn builtin_amendment() {
	let src = indoc! {r#"
		print("".is_empty(), "hi".is_empty())
		print(int.max)
		print(int.min)
		print((0.0).is_nan())
		print(float.epsilon)
		print(i8.max == i8.(127))
		print(isize.min)
		print(u64.max)
		print(usize.max)
		print(u13.max)
		print(i13.min)
	"#};
	check(
		src,
		[
			"true false",
			"9223372036854775807",
			"-9223372036854775808",
			"false",
			"2.220446049250313e-16",
			"true",
			"-9223372036854775808",
			"18446744073709551615",
			"18446744073709551615",
			"8191",
			"-4096",
		],
	);
}

#[test]
fn builtin_amendment_outside_core() {
	fail(
		indoc! {r#"
			string :< {
				nope :: fn(self) bool { true }
			}
			print("hi".nope())
		"#},
		"amended in core",
	);
}
