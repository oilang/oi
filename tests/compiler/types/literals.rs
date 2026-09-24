use crate::helpers::*;

#[test]
fn float_sci_upper_e() {
	check("42E1", "420.0");
}

#[test]
fn float_sci_decimal_with_exp() {
	check("1.5e2", "150.0");
}

#[test]
fn bool_true() {
	check("true", "true");
}

#[test]
fn string_literal() {
	check(r#""hello""#, "hello");
}

// ranges

#[test]
fn range_literal_prints() {
	check("1..5", "1..5");
}

#[test]
fn range_negative_start() {
	check("-4..4", "-4..4");
}

#[test]
fn range_in_binding() {
	let src = indoc! {"
		r :: 1..10
		r
	"};
	check(src, "1..10");
}

#[test]
fn range_arithmetic_bounds() {
	check("1 + 1..2 + 3", "2..5");
}

#[test]
fn range_as_fn_return() {
	let src = indoc! {"
		make :: fn(lo: int, hi: int) Range { lo..hi }
		make(3, 7)
	"};
	check(src, "3..7");
}

#[test]
fn range_in_for_loop() {
	let src = indoc! {"
		sum := 0
		loop i in 1..5 {
			sum = sum + i
		}
		sum
	"};
	check(src, "10");
}

#[test]
fn range_stored_then_iterated() {
	let src = indoc! {"
		r :: 0..4
		sum := 0
		loop i in r {
			sum = sum + i
		}
		sum
	"};
	check(src, "6");
}

#[test]
fn range_bound_must_be_int() {
	fail("0..true", "must be Int");
}
