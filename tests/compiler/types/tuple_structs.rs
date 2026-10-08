use crate::helpers::*;

#[test]
fn zero_values() {
	check(
		indoc! {"
			Money :: struct (int)
			Point :: struct (x: float, y: float)
			Foo :: struct (int, y: bool)
			UserId :: struct (int | string)
			m: Money
			p: Point
			f: Foo
			u: UserId
			print(m.0, p.x, f.y, u.0)
			print(m, p)
		"},
		["0 0.0 false 0", "Money(0) Point(x = 0.0, y = 0.0)"],
	);
}

#[test]
fn no_such_field() {
	fail(["Money :: struct (int)", "m: Money", "m.1"], "");
	fail(["Point :: struct (x: float, y: float)", "p: Point", "p.z"], "");
}

#[test]
fn construct() {
	check(
		indoc! {"
			Money :: struct (int)
			Point :: struct (x: float, y: float)
			p :: Point(x = 1.0, y = 2.0)
			print(Money(500).0, Point(1.0, 2.0).y, p.0 == p.x)
		"},
		"500 2.0 true",
	);
}

#[test]
fn nominal_in_signatures() {
	check(
		indoc! {"
			Money :: struct (int)
			pay :: fn(m: Money) int { m.0 }
			pay(Money(500))
		"},
		"500",
	);
	fail(
		indoc! {"
			Money :: struct (int)
			pay :: fn(m: Money) int { m.0 }
			pay(500)
		"},
		"",
	);
}

#[test]
fn methods_and_self() {
	check(
		indoc! {"
			Money :: struct (int)
			Money :< {
				double :: fn(self) Self {
					Money(self.0 * 2)
				}
			}
			Money(5).double().0
		"},
		"10",
	);
}

#[test]
fn str_override() {
	check(
		indoc! {r#"
			Money :: struct (int)
			Money :< {
				str :: fn(self) string {
					"money!"
				}
			}
			print(Money(5))
			Money(5).str()
		"#},
		["money!", "money!"],
	);
}

#[test]
fn construct_into_sum_member() {
	check(
		indoc! {r#"
			UserId :: struct (int | string)
			UserId("abc").0
		"#},
		"abc",
	);
}

#[test]
fn wrong_arity_and_type() {
	fail(
		indoc! {"
			Money :: struct (int)
			Money(1, 2)
		"},
		"",
	);
	fail(
		indoc! {r#"
			Money :: struct (int)
			Money("x")
		"#},
		"",
	);
}

#[test]
fn equality_compares_inner() {
	check(
		indoc! {"
			Money :: struct (int)
			print(Money(5) == Money(5))
			Money(5) == Money(6)
		"},
		["true", "false"],
	);
}

#[test]
fn wraps_an_array() {
	check(
		indoc! {"
			Handle :: struct ([]int)
			h :: Handle([1, 2, 3])
			g :: h
			g.0[1]
		"},
		"2",
	);
}

#[test]
fn builtin_name_errors_at_def() {
	fail("int :: struct (bool)", "is a builtin type");
	fail("f32 :: struct (float)", "is a builtin type");
	fail("uint :: struct (int)", "is a builtin type");
}

#[test]
fn dot_tuple_construct() {
	check(
		indoc! {"
			Money :: struct (int)
			Point :: struct (x: float, y: float)
			pay :: fn(m: Money) int { m.0 }
			m: Money = .(500)
			p: Point = .(1.0, 2.0)
			print(m.0, p.x, pay(.(7)))
		"},
		"500 1.0 7",
	);
}

#[test]
fn dot_tuple_rejections() {
	fail(["Money :: struct (int)", "m: Money = .(1, 2)"], "takes 1 field(s)");
	fail("m := .(1)", "cannot infer");
}

#[test]
fn ops_pass_through_in_own_methods() {
	check(
		indoc! {r#"
			Money :: struct (int)
			Money :< {
				str :: fn(self) string { "${self.0}" }
				double :: fn(self) Self { self * 2 }
			}
			print(Money(5).double())
		"#},
		"$10",
	);
}

#[test]
fn ops_stay_closed_outside() {
	fail(
		indoc! {"
			Money :: struct (int)
			Money(5) * 2
		"},
		"cannot apply `*` to Money",
	);
}

#[test]
fn unary_claims_dispatch() {
	check(
		indoc! {"
			Flag :: struct (bool)
			Flag : Not < {
				not :: fn(self) Self { Flag(!self.0) }
			}
			Money :: struct (int)
			Money : Neg < {
				neg :: fn(self) Self { Money(-self.0) }
			}
			print((-Money(5)).0, (!Flag(true)).0)
		"},
		"-5 false",
	);
}
