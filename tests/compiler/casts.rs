use crate::helpers::*;

#[test]
fn float_to_int_truncates_toward_zero() {
	check("int.(2.9)", "2");
	check("int.(-2.9)", "-2");
	check("u8.(f32.(3.7))", "3");
}

#[test]
fn struct_casts() {
	check(
		indoc! {"
			Point :: struct { x: int, y: int }
			p :: Point.{x = 1, y = 2}
			print(Point.(p).y)
		"},
		"2",
	);
	check(
		indoc! {"
			Money :: struct (int)
			m :: Money(5)
			print(Money.(500).0, Money.(m).0)
		"},
		"500 5",
	);
	check(
		indoc! {"
			Pair :: struct (x: float, y: float)
			print(Pair.(1.0, 2.0).y)
		"},
		"2.0",
	);
}

#[test]
fn widening_casts() {
	check("?int.(42)", "some(42)");
	check(["Handle :: int | string", "print(Handle.(5))"], "5");
	check(
		indoc! {"
			Shape :: trait { area : fn(self) int }
			Sq :: struct { s: int }
			Sq : Shape < { area :: fn(self) int { self.s * self.s } }
			d :: Shape.(Sq.{s = 3})
			print(d.area())
		"},
		"9",
	);
}

#[test]
fn alias_cast() {
	check(
		indoc! {"
			Operation :: fn(int) int
			double :: fn(n: int) int { n * 2 }
			f :: Operation.(double)
			print(f(21))
		"},
		"42",
	);
}

#[test]
fn strings_do_not_parse() {
	fail_with(r#"int.("42")"#, "cannot cast string to int");
	fail_with(r#"float.("2.5")"#, "`float.try_from(...)` parses strings");
}
