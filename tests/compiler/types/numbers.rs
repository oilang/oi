use crate::helpers::*;

// TODO: move most of these to oi land

#[test]
fn int_literals() {
	check(
		"print(1_000_000_000, 1_2_3_4_5, 0x7B, 0xFF_00, 0b01111011, 0b1_1111_1111, 0o173, 0o7_5_5)",
		"1000000000 12345 123 65280 123 511 123 493",
	);
	check("x : u64 : 0xFF80_0000_0000_0000", "18410715276690587648");
}

#[test]
fn aliases() {
	check(
		"print(int.max == i64.max, uint.max == u64.max, float.max == f64.max)",
		"true true true",
	);
}

#[test]
fn int_casts() {
	check(
		indoc! {"
			print(i8.(127), i8.(128), i8.(-129), i16.(32768), i16.(-32769), i32.(10000000000), 10_000 == i32.(10_000))
			print(i64.(10000000000), 10_000_000_000 == i64.(10_000_000_000))
			print(u8.(256), u8.(-1), u16.(65536), u16.(-1), u32.(5_000_000_000), u32.(-1_000_000))
			print(u64.(9223372036854775807), u64.(-1), u64.(-1_000_000))
			print(isize.(-1), isize.(i32.(50)), isize.(u64.(42)), usize.(-1), usize.(u32.(255)), usize.(i32.(10)))
		"},
		[
			"127 -128 127 -32768 32767 1410065408 true",
			"10000000000 true",
			"0 255 0 65535 705032704 4293967296",
			"9223372036854775807 18446744073709551615 18446744073708551616",
			"-1 50 42 18446744073709551615 255 10",
		],
	);
}

#[test]
fn float() {
	check(
		"print(123.0000, 10_000.22, 2e0, 10e+2, 10e-2, 42E1, 1.5e2)",
		"123.0 10000.22 2.0 1000.0 0.1 420.0 150.0",
	);
}

#[test]
fn f32() {
	check("print(f32.(123.0), f32.(123.0) == f32.(123.0))", "123.0 true");
}

#[test]
fn sized_arithmetic() {
	check(
		indoc! {"
			print(u32.(10) + u32.(20), u32.(100) - u32.(40), u32.(6) * u32.(7), u32.(100) / u32.(4), u32.(17) % u32.(5))
			print(u32.(10) == u32.(10), u32.(10) != u32.(20), u32.(5) < u32.(10), u32.(10) > u32.(5))
			print(u64.(100) <= u64.(100), u64.(100) >= u64.(50))
			print(isize.(100) - isize.(1), isize.(6) * isize.(7), isize.(5) < isize.(10))
			print(usize.(100) / usize.(4), usize.(10) == usize.(10), usize.(5) < usize.(10))
		"},
		[
			"30 60 42 25 2",
			"true true true true",
			"true true",
			"99 42 true",
			"25 true true",
		],
	);
}

#[test]
fn literal_takes_operand_type() {
	check(["x: u8 = 97", "print(x >= 97, x - 32)"], "true 65");
	check(["x: u8 = 97", "print(97 == x)"], "true");
	fail(["x: u8 = 97", "x == -1"], "out of range for u8");
}

#[test]
fn arb_width() {
	check(
		indoc! {"
			print(i3.(3), i3.(4), i3.(-4), i3.(-5), i7.(64), i7.(-65), i13.(4096), i34.(8_589_934_592))
			print(u3.(7), u3.(8), u3.(-1), u7.(128), u34.(17_179_869_184))
			print(i3.(3) + i3.(1), i3.(-4) - i3.(1), u3.(7) + u3.(1), i7.(30) + i7.(30))
		"},
		["3 -4 -4 3 -64 63 -4096 -8589934592", "7 0 7 0 0", "-4 3 0 60"],
	);
}

#[test]
fn promotion() {
	check("2 + 1.0 == 3.0", "true");
	check("2 + 1.0", "3.0");
	check("1.0 + 2", "3.0");
	check("i64.(2) + i8.(3)", "5");
	check("u8.(200) + u16.(1000)", "1200");
	check("f32.(1.5) + 2.0", "3.5");
	fail("i8.(1) + u8.(1)", "cannot apply");
}

#[test]
fn f16_f128_not_yet_supported() {
	fail("f16.(1.0)", "f16 casts are not yet supported");
	fail("f16.(123)", "f16 casts are not yet supported");
	fail("f128.(1.0)", "f128 casts are not yet supported");
	fail("f128.(123)", "f128 casts are not yet supported");
}

#[test]
fn cast_syntax() {
	check("int.(u8.(200))", "200");
	fail("int(3)", "undefined function `int`");
}
