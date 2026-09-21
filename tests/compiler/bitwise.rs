use crate::helpers::*;

#[test]
fn ops() {
	check(
		indoc! {r#"
			print(0b1100 & 0b1010)
			print(0b1100 | 0b1010)
			print(0b1100 ~ 0b1010)
			print(0xFF00 & 0xF0F0)
			print(!0)
		"#},
		["8", "14", "6", "61440", "-1"],
	);
}

#[test]
fn narrow_widths() {
	check(
		indoc! {"
			x := u8.(0b1111_0000)
			y := u7.(0)
			print(!x)
			print(!y)
		"},
		["15", "127"],
	);
}

#[test]
fn precedence() {
	check(
		indoc! {"
			print(6 & 4 == 4)
			print(3 & 1 + 1)
			print(2 | 6 & 4)
		"},
		["true", "2", "6"],
	);
}

#[test]
fn shifts() {
	check(
		indoc! {"
			print(1 << 4)
			print(-8 >> 1)
			print(u8.(0b1000_0000) >> 1)
			print(1 << 3 - 1)
			print(1 << 4 | 3)
		"},
		["16", "-4", "64", "4", "19"],
	);
}

#[test]
fn rejects_floats() {
	fail_with("1.5 & 2.0", "bitwise operators need integer operands");
}
