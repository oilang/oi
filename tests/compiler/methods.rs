use crate::helpers::*;

#[test]
fn instance_method() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< {
			sum :: fn(self) int { self.x + self.y }
		}
		p :: Point.{3, 4}
		p.sum()
	"};
	check(src, "7");
}

#[test]
fn method_with_args() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< {
			scaled :: fn(self, k: int) int { (self.x + self.y) * k }
		}
		Point.{3, 4}.scaled(10)
	"};
	check(src, "70");
}

#[test]
fn method_on_literal() {
	let src = indoc! {"
		P :: struct { x: int, y: int }
		P :< { sum :: fn(self) int { self.x + self.y } }
		P.{3, 4}.sum()
	"};
	check(src, "7");
}

#[test]
fn static_method() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< {
			origin :: fn() Point { Point.{0, 0} }
			sum :: fn(self) int { self.x + self.y }
		}
		Point.origin().sum()
	"};
	check(src, "0");
}

#[test]
fn static_method_with_args() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< { make :: fn(a: int, b: int) Point { Point.{a, b} } }
		Point.make(3, 4).x
	"};
	check(src, "3");
}

#[test]
fn self_type_and_literal() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< {
			new :: fn() Self { Self.{} }
			sum :: fn(self) int { self.x + self.y }
		}
		Point.new().sum()
	"};
	check(src, "0");
}

#[test]
fn self_param_and_fields() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< {
			add :: fn(self, other: Self) Self { Self.{self.x + other.x, self.y + other.y} }
		}
		Point.{1, 2}.add(Point.{3, 4}).x
	"};
	check(src, "4");
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
	check(r#"print("".is_empty())"#, "true");
	check(r#"print("hi".is_empty())"#, "false");
}

#[test]
fn builtin_amendment_primitives() {
	let src = indoc! {r#"
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
fn associated_const() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< { origin :: Point.{0, 0} }
		print(Point.origin.x)
	"};
	check(src, "0");
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
