use crate::helpers::*;

#[test]
fn int_ops() {
	check(
		indoc! {"
			print(2 + 3, 10 - 4, 3 * 4, 10 / 3, 10 % 7)
			print(-10 % 7, 10 % -7, 1 + 10 % 7)
			print(1.5 + 2.0, -5)
		"},
		["5 6 12 3 3", "-3 3 4", "3.5 -5"],
	);
}

#[test]
fn mod_float_unsupported() {
	fail("10.0 % 3.0", "not yet supported on floats");
}

#[test]
fn pow() {
	check("print(2 ** 10, 2 ** 3 ** 2, -2 ** 2, 2.0 ** -1.0)", "1024 512 -4 0.5");
	fail_rt("2 ** -1", "negative exponent");
}

#[test]
fn const_folding() {
	check(
		indoc! {r#"
			A :: int.min
			B :: u8.max
			C :: "a" + "b"
			D := 2 ** 63
			print(A, B, C, D)
		"#},
		"-9223372036854775808 255 ab -9223372036854775808",
	);
}
