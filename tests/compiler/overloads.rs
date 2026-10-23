use crate::helpers::*;

#[test]
fn struct_ops_dispatch_to_the_fill() {
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
		Point : Neg < {
			neg :: fn(self) Self { Self.{ -self.x, -self.y } }
		}
		assert!(Point.{1, 0} + Point.{2, 3} == Point.{3, 3})
		print(Point.{1, 0} + .{2, 3})
		print(Point.{1, 0} != .{1, 0}, -Point.{1, 2})
	"};
	check(src, ["Point.{x = 3, y = 3}", "false Point.{x = -1, y = -2}"]);
}

#[test]
fn anon_literal_on_the_lhs_borrows_the_other_side() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point : Add < { add :: fn(self, other: Self) Self { Self.{ self.x + other.x, self.y + other.y } } }
		Point : Eq < { eq :: fn(self, other: Self) bool { self.x == other.x && self.y == other.y } }
		Point : Mul < { mul :: fn(self; k: i8) Self { self } }
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
		Frac :: struct { num: int, den: int }
		Frac : Eq < {
			eq :: fn(self, other: Self) bool { self.num * other.den == other.num * self.den }
		}
		print(a == b, a != c, Frac.{1, 2} == Frac.{2, 4})
	"#};
	check(src, "true true true");
}

#[test]
fn orderings_derive_from_cmp() {
	let src = indoc! {"
		Frac :: struct { num: int, den: int }
		Frac : Cmp < {
			cmp :: fn(self, other: Self) Ordering {
				lhs :: self.num * other.den
				rhs :: other.num * self.den
				if lhs < rhs { .less } else if lhs > rhs { .greater } else { .equal }
			}
		}
		a :: Frac.{1, 3}
		b :: Frac.{1, 2}
		print(a < b, b > a, a <= b, b >= a)
		print(a > b, a >= b, a <= a, a >= a)
	"};
	check(src, ["true true true true", "false false true true"]);

	let src = indoc! {"
		P :: struct { x: int }
		P : PartialCmp < {
			partial_cmp :: fn(self, o: Self) ?Ordering { if self.x == o.x { .equal } else { none } }
		}
		print(P.{1} < P.{2}, P.{1} <= P.{1}, P.{1} >= P.{2})
	"};
	check(src, "false true false");
}

#[test]
fn unordered_struct_is_rejected() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point.{1, 2} < Point.{3, 4}
	"};
	fail(src, "claim `Cmp` for `Point` to define ordering");
}

#[test]
fn incomparable_field_is_rejected() {
	let src = indoc! {"
		Holder :: struct { f: fn(int) int }
		a := Holder.{ fn(x: int) int { x } }
		a == a
	"};
	fail(src, "cannot compare Holder: contains fn(int) int");
}

#[test]
fn unclaimed_struct_is_rejected() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point.{1, 2} - Point.{3, 4}
	"};
	fail(src, "cannot apply `-` to Point");
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
		Rev : PartialCmp < { partial_cmp :: fn(self, other: Self) ?Ordering { .less } }
		P :: struct { x: int }
		E :: enum { a(P), b }
		print(Rev.a == Rev.b, Rev.b < Rev.a)
		print(E.a.(P.{1}) == E.a.(P.{1}), E.a.(P.{1}) == E.a.(P.{2}))
	"};
	check(src, ["true true", "true false"]);
}

#[test]
fn fill_must_match_the_trait() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point : Eq < {
			eq :: fn(self; other: int) bool { true }
		}
	"};
	fail(
		src,
		"`Point.eq` is `fn(Point, int) bool`, trait `core::Eq` declares `fn(Point, Point) bool`",
	);
}

#[test]
fn arithmetic_fills_narrow_and_commute() {
	let src = indoc! {"
		Scale :: struct { f: int }
		Scale : Mul < {
			mul :: fn(self; other: int) Self { Self.{ self.f * other } }
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
			div :: fn(self; other: int) Self { Self.{ self.f / other } }
		}
		print(2 / Scale.{3})
	"};
	fail(src, "cannot apply `/` to int and Scale");
}

#[test]
fn contains_drives_in() {
	let src = indoc! {r#"
		Bag :: struct { items: []int }
		Bag : Contains[int] < {
			contains :: fn(self; v: int) bool { v in self.items }
		}
		b :: Bag.{ items = [1 2 3] }
		print(2 in b, 9 in b)
	"#};
	check(src, "true false");
}

#[test]
fn index_traits_drive_brackets() {
	let src = indoc! {"
		Grid :: struct { cells: []int = [0 0 0 0] }
		Grid : Index[(int, int), int] < {
			index :: fn(self; p: (int, int)) int { self.cells[p.0 * 2 + p.1] }
		}
		Grid : IndexAssign[(int, int), int] < {
			index_assign :: fn(mut self; p: (int, int), v: int) {
				c := self.cells
				c[p.0 * 2 + p.1] = v
				self.cells = c
			}
		}
		g := Grid.{}
		g[(1, 0)] = 7
		print(g[(1, 0)], g.cells)
	"};
	check(src, "7 [0, 0, 7, 0]");
}

#[test]
fn a_type_argument_picks_the_operator_claim() {
	let src = indoc! {"
		V :: struct { x: int }
		V : Mul < { mul :: fn(self; other: V) V { V.{ self.x * other.x } } }
		V : Mul[int] < { mul :: fn(self; other: int) V { V.{ self.x * other } } }
		v :: V.{3}
		print((v * v).x, (v * 2).x, (2 * v).x)
	"};
	check(src, "9 6 6");

	let src = indoc! {"
		I :: struct { ns: int }
		D :: struct { ns: int }
		I : Sub < {
			Out :: D
			sub :: fn(self, o: Self) Out { .{ self.ns - o.ns } }
		}
		I : Sub[D] < {
			sub :: fn(self; d: D) Self { .{ self.ns - d.ns } }
		}
		print((I.{9} - I.{4}).ns, (I.{9} - D.{4}).ns)
	"};
	check(src, "5 5");

	let src = indoc! {"
		G[T] :: struct { x: T }
		G[T] : Mul < { mul :: fn(self, o: Self) Self { .{ self.x * o.x } } }
		G[T] : Mul[T] < { mul :: fn(self; s: T) Self { .{ self.x * s } } }
		g :: G[float].{3.0}
		print((g * g).x, (g * 2.0).x, (2.0 * g).x)
	"};
	check(src, "9.0 6.0 6.0");
}

#[test]
fn an_int_literal_takes_the_operator_claims_int_type() {
	let src = indoc! {"
		S :: struct { v: i33 }
		S : Mul < { mul :: fn(self; o: S) S { S.{ self.v * o.v } } }
		S : Mul[i33] < { mul :: fn(self; k: i33) S { S.{ self.v * k } } }
		print((S.{3} * 2).v, (S.{3} * -2).v, (2 * S.{3}).v)
	"};
	check(src, "6 -6 6");
}

#[test]
fn an_operator_claim_must_cover_the_right_operand() {
	let src = indoc! {"
		V :: struct { x: int }
		V : Mul[int] < { mul :: fn(self; other: int) V { V.{ self.x * other } } }
		f := 2.0
		print((V.{3} * f).x)
	"};
	fail(src, "`V` claims no `core::Mul[float]`");
}

#[test]
fn send_and_recv_drive_arrows() {
	let src = indoc! {r#"
		Pipe :: struct { cap: int }
		Pipe : Send[int] < {
			send :: fn(self; move v: int) !() {
				if v > self.cap { return error("full") }
				print("sent {v}")
			}
		}
		Pipe : Recv[int] < { recv :: fn(self) ?int { self.cap } }
		p :: Pipe.{ 9 }
		p <- 2 + 3 or print("dropped")
		p <- 20 or print("dropped")
		p<-3 or print("dropped")
		print(<-p or 0)
	"#};
	check(src, ["sent 5", "dropped", "sent 3", "9"]);
}
