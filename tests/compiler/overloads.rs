use crate::helpers::*;

#[test]
fn struct_op_dispatches() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point : Add < {
			add :: fn(self, other: Self) Self {
				Self.{ self.x + other.x, self.y + other.y }
			}
		}
		print(Point.{1, 0} + .{2, 3})
	"};
	check(src, "Point.{x = 3, y = 3}");
}

#[test]
fn eq_dispatches_to_the_fill() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point : Add < {
			add :: fn(self, other: Self) Self {
				Self.{ self.x + other.x, self.y + other.y }
			}
		}
		Point : Eq < {
			eq :: fn(self, other: Self) bool { self.x == other.x && self.y == other.y }
		}
		assert!(Point.{1, 0} + Point.{2, 3} == Point.{3, 3})
		print(Point.{1, 0} != .{1, 0})
	"};
	check(src, "false");
}

#[test]
fn anon_literal_on_the_lhs_borrows_the_other_side() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point : Add < { add :: fn(self, other: Self) Self { Self.{ self.x + other.x, self.y + other.y } } }
		Point : Eq < { eq :: fn(self, other: Self) bool { self.x == other.x && self.y == other.y } }
		Point : Mul < { mul :: fn(self, k: i8) Self { self } }
		p :: Point.{1, 2}
		print(.{2, 3} + p, .{1, 2} == p, p * 3)
	"};
	check(src, "Point.{x = 3, y = 5} true Point.{x = 1, y = 2}");
}

#[test]
fn eq_is_structural_by_default() {
	let src = indoc! {r#"
		Inner :: struct { n: int }
		Rec :: struct { a: int, s: string, f: float, i: Inner }
		a := Rec.{1, "hi", 1.5, Inner.{2}}
		b := Rec.{1, "hi", 1.5, Inner.{2}}
		c := Rec.{1, "hi", 1.5, Inner.{3}}
		print(a == b)
		print(a != c)
	"#};
	check(src, ["true", "true"]);
}

#[test]
fn claimed_eq_beats_the_structural_default() {
	let src = indoc! {"
		Frac :: struct { num: int, den: int }
		Frac : Eq < {
			eq :: fn(self, other: Self) bool { self.num * other.den == other.num * self.den }
		}
		print(Frac.{1, 2} == Frac.{2, 4})
	"};
	check(src, "true");
}

#[test]
fn neg_dispatches_to_the_fill() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point : Neg < {
			neg :: fn(self) Self { Self.{ -self.x, -self.y } }
		}
		print(-Point.{1, 2})
	"};
	check(src, "Point.{x = -1, y = -2}");
}

#[test]
fn orderings_derive_from_ord() {
	let src = indoc! {"
		Frac :: struct { num: int, den: int }
		Frac : Ord < {
			lt :: fn(self, other: Self) bool { self.num * other.den < other.num * self.den }
		}
		a :: Frac.{1, 3}
		b :: Frac.{1, 2}
		print(a < b, b > a, a <= b, b >= a)
		print(a > b, a >= b, a <= a, a >= a)
	"};
	check(src, ["true true true true", "false false true true"]);
}

#[test]
fn unordered_struct_is_rejected() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point.{1, 2} < Point.{3, 4}
	"};
	fail_with(src, "claim `Ord` for `Point` to define ordering");
}

#[test]
fn incomparable_field_is_rejected() {
	let src = indoc! {"
		Holder :: struct { f: fn(int) int }
		a := Holder.{ fn(x: int) int { x } }
		a == a
	"};
	fail_with(src, "cannot compare Holder: contains fn(int) int");
}

#[test]
fn unclaimed_struct_is_rejected() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point.{1, 2} - Point.{3, 4}
	"};
	fail_with(src, "cannot apply `-` to Point");
}

#[test]
fn enum_ops_dispatch() {
	let src = indoc! {"
		Dir :: enum { up, down }
		Dir : Add < {
			add :: fn(self, other: Self) Self { if self == .up { other } else { .down } }
		}
		Dir : Neg < {
			neg :: fn(self) Self { if self == .up { Dir.down } else { Dir.up } }
		}
		print(Dir.up + Dir.down, -Dir.up)
	"};
	check(src, "down down");
}

#[test]
fn enum_claims_beat_structural_defaults() {
	let src = indoc! {"
		Rev :: enum { a, b }
		Rev : Eq < { eq :: fn(self, other: Self) bool { true } }
		Rev : Ord < { lt :: fn(self, other: Self) bool { true } }
		print(Rev.a == Rev.b, Rev.b < Rev.a)
	"};
	check(src, "true true");
}

#[test]
fn enum_payloads_compare_structurally() {
	let src = indoc! {"
		P :: struct { x: int }
		E :: enum { a(P), b }
		print(E.a.(P.{1}) == E.a.(P.{1}), E.a.(P.{1}) == E.a.(P.{2}))
	"};
	check(src, "true false");
}

#[test]
fn fill_must_match_the_trait() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point : Eq < {
			eq :: fn(self, other: int) bool { true }
		}
	"};
	fail_with(
		src,
		"`Point.eq` is `fn(Point, int) bool`, trait `core::Eq` declares `fn(Point, Point) bool`",
	);
}

#[test]
fn arithmetic_fills_narrow_and_commute() {
	let src = indoc! {"
		Scale :: struct { f: int }
		Scale : Mul < {
			mul :: fn(self, other: int) Self { Self.{ self.f * other } }
		}
		print((Scale.{3} * 2).f, (2 * Scale.{3}).f)
	"};
	check(src, "6 6");
}

#[test]
fn non_commuting_ops_never_reverse() {
	let src = indoc! {"
		Scale :: struct { f: int }
		Scale : Div < {
			div :: fn(self, other: int) Self { Self.{ self.f / other } }
		}
		print(2 / Scale.{3})
	"};
	fail_with(src, "cannot apply `/` to int and Scale");
}
