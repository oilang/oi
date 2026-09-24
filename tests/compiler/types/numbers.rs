use crate::helpers::*;

// TODO: move most of these to oi land

#[test]
fn int() {
	check("123", "123");
	check("1_000_000_000", "1000000000");
	check("1_2_3_4_5", "12345");
}

#[test]
fn hex() {
	check("0x7B", "123");
	check("0xFF", "255");
	check("0xFF_00", "65280");
	// TODO: add once we support 64-bit numbers
	// check("0xFF80_0000_0000_0000", "0xFF80000000000000");
}

#[test]
fn binary() {
	check("0b01111011", "123");
	check("0b1_1111_1111", "511");
}

#[test]
fn octal() {
	check("0o173", "123");
	check("0o7_5_5", "493");
}

#[test]
fn aliases() {
	check("int.max == i64.max", "true");
	check("uint.max == u64.max", "true");
	check("float.max == f64.max", "true");
}

#[test]
fn int_cast() {
	check("i32.(50_000)", "50000");
	check("i32.(2_000_000_000)", "2000000000");
	check("i32.(10000000000)", "1410065408");
	check("10_000 == i32.(10_000)", "true");
}

#[test]
fn i64_cast() {
	check("i64.(50_000)", "50000");
	check("i64.(2_000_000_000)", "2000000000");
	check("i64.(10000000000)", "10000000000");
	check("10_000_000_000 == i64.(10_000_000_000)", "true");
	check("10_000_000_000", "10000000000");
}

#[test]
fn float() {
	check("123.0", "123.0");
	check("123.0000", "123.0");
	check("10_000.22", "10000.22");
}

#[test]
fn float_exp() {
	check("2e0", "2.0");
	check("10e1", "100.0");
	check("10e+2", "1000.0");
	check("10e-2", "0.1");
}

#[test]
fn f32() {
	check("f32.(123.0)", "123.0");
	check("f32.(123.0) == f32.(123.0)", "true");
}

#[test]
fn u32_cast() {
	check("u32.(0)", "0");
	check("u32.(4_000_000_000)", "4000000000");
	check("u32.(5_000_000_000)", "705032704");
	check("u32.(-1)", "4294967295");
	check("u32.(-1_000_000)", "4293967296");
}

#[test]
fn u64_cast() {
	check("u64.(0)", "0");
	check("u64.(10_000_000_000)", "10000000000");
	check("u64.(9223372036854775807)", &(i64::MAX as u64).to_string());
	check("u64.(-1)", "18446744073709551615");
	check("u64.(-1_000_000)", "18446744073708551616");
}

#[test]
fn uint_arithmetic() {
	check("u32.(10) + u32.(20)", "30");
	check("u32.(100) - u32.(40)", "60");
	check("u32.(6) * u32.(7)", "42");
	check("u32.(100) / u32.(4)", "25");
	check("u32.(17) % u32.(5)", "2");
}

#[test]
fn uint_cmp() {
	check("u32.(10) == u32.(10)", "true");
	check("u32.(10) != u32.(20)", "true");
	check("u32.(5) < u32.(10)", "true");
	check("u32.(10) > u32.(5)", "true");
	check("u64.(100) <= u64.(100)", "true");
	check("u64.(100) >= u64.(50)", "true");
}

#[test]
fn i8_cast() {
	check("i8.(0)", "0");
	check("i8.(127)", "127");
	check("i8.(128)", "-128");
	check("i8.(-128)", "-128");
	check("i8.(-129)", "127");
}

#[test]
fn i16_cast() {
	check("i16.(0)", "0");
	check("i16.(32767)", "32767");
	check("i16.(32768)", "-32768");
	check("i16.(-32768)", "-32768");
	check("i16.(-32769)", "32767");
}

#[test]
fn u8_cast() {
	check("u8.(0)", "0");
	check("u8.(255)", "255");
	check("u8.(256)", "0");
	check("u8.(-1)", "255");
}

#[test]
fn literal_takes_operand_type() {
	check(["x: u8 = 97", "print(x >= 97, x - 32)"], "true 65");
	check(["x: u8 = 97", "print(97 == x)"], "true");
	fail(["x: u8 = 97", "x == -1"], "out of range for u8");
}

#[test]
fn u16_cast() {
	check("u16.(0)", "0");
	check("u16.(65535)", "65535");
	check("u16.(65536)", "0");
	check("u16.(-1)", "65535");
}

#[test]
fn arb_width_signed() {
	// i3 [-4..3]
	check("i3.(0)", "0");
	check("i3.(3)", "3");
	check("i3.(4)", "-4");
	check("i3.(-4)", "-4");
	check("i3.(-5)", "3");
	// i7 [-64..63]
	check("i7.(63)", "63");
	check("i7.(64)", "-64");
	check("i7.(-64)", "-64");
	check("i7.(-65)", "63");
	// i13 [-4096..4095]
	check("i13.(4095)", "4095");
	check("i13.(4096)", "-4096");
	// i34
	check("i34.(8_589_934_592)", "-8589934592");
}

#[test]
fn arb_width_unsigned() {
	// u3 [0..7]
	check("u3.(0)", "0");
	check("u3.(7)", "7");
	check("u3.(8)", "0");
	check("u3.(-1)", "7");
	// u7 [0..127]
	check("u7.(127)", "127");
	check("u7.(128)", "0");
	// i34
	check("u34.(17_179_869_184)", "0");
}

#[test]
fn arb_width_arithmetic() {
	// wrapping outside range
	check("i3.(3) + i3.(1)", "-4");
	check("i3.(-4) - i3.(1)", "3");
	check("u3.(7) + u3.(1)", "0");
	// no wrapping within range
	check("i7.(30) + i7.(30)", "60");
}

#[test]
fn isize_cast() {
	check("isize.(0)", "0");
	check("isize.(100)", "100");
	check("isize.(-1)", "-1");
	check("isize.(i32.(50))", "50");
	check("isize.(u64.(42))", "42");
}

#[test]
fn usize_cast() {
	check("usize.(0)", "0");
	check("usize.(100)", "100");
	check("usize.(-1)", "18446744073709551615");
	check("usize.(u32.(255))", "255");
	check("usize.(i32.(10))", "10");
}

#[test]
fn isize_arithmetic() {
	check("isize.(10) + isize.(20)", "30");
	check("isize.(100) - isize.(1)", "99");
	check("isize.(6) * isize.(7)", "42");
	check("isize.(10) == isize.(10)", "true");
	check("isize.(5) < isize.(10)", "true");
}

#[test]
fn usize_arithmetic() {
	check("usize.(10) + usize.(20)", "30");
	check("usize.(100) / usize.(4)", "25");
	check("usize.(10) == usize.(10)", "true");
	check("usize.(5) < usize.(10)", "true");
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
fn f16_not_yet_supported() {
	fail("f16.(1.0)", "f16 casts are not yet supported");
	fail("f16.(123)", "f16 casts are not yet supported");
}

#[test]
fn f128_not_yet_supported() {
	fail("f128.(1.0)", "f128 casts are not yet supported");
	fail("f128.(123)", "f128 casts are not yet supported");
}

#[test]
fn cast_syntax() {
	check("int.(u8.(200))", "200");
	fail("int(3)", "undefined function `int`");
}
