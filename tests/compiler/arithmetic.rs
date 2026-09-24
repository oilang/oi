use crate::helpers::*;

#[test]
fn int_add() {
	check("2 + 3", "5");
}

#[test]
fn int_sub() {
	check("10 - 4", "6");
}

#[test]
fn int_mul() {
	check("3 * 4", "12");
}

#[test]
fn int_div() {
	check("10 / 3", "3");
}

#[test]
fn int_mod() {
	check("10 % 7", "3");
}

#[test]
fn mod_negative_dividend() {
	check("-10 % 7", "-3");
}

#[test]
fn mod_negative_divisor() {
	check("10 % -7", "3");
}

#[test]
fn mod_binds_like_mul() {
	check("1 + 10 % 7", "4");
}

#[test]
fn mod_float_unsupported() {
	fail("10.0 % 3.0", "not yet supported on floats");
}

#[test]
fn float_add() {
	check("1.5 + 2.0", "3.5");
}

#[test]
fn negation() {
	check("-5", "-5");
}

#[test]
fn pow() {
	check("2 ** 10", "1024");
	check("2 ** 3 ** 2", "512");
	check("-2 ** 2", "-4");
	check("2.0 ** -1.0", "0.5");
	fail_rt("2 ** -1", "negative exponent");
}

#[test]
fn const_folding() {
	check(["A :: int.min", "A"], "-9223372036854775808");
	check(["A :: u8.max", "A"], "255");
	check([r#"A :: "a" + "b""#, "A"], "ab");
	check(["A := 2 ** 63", "A"], "-9223372036854775808");
}
