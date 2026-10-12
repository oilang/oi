use indoc::indoc;

use crate::helpers::*;

#[test]
fn logic_and_precedence() {
	check(
		[
			"print(true && true, false || true, !true)",
			"print(true || true && false, !false && false, 1 < 2 && 4 > 3)",
		],
		["true true false", "true false true"],
	);
}

#[test]
fn and_or_short_circuit() {
	check("print(false && 1 / 0 > 0, true || 1 / 0 > 0)", "false true");
}

#[test]
fn operands_must_be_bool() {
	fail("1 && true", "expected Bool");
	fail(r#"!"hi""#, "expected Bool or an integer");
}

#[test]
fn sized_bools() {
	let src = indoc! {"
		@c S :: struct { a: u8, ok: b32, b: u8 }
		buf: []u8 = .[9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]
		unsafe { buf.ptr.write(S.{ a = 1, ok = true, b = 2 }) }
		s := unsafe { buf.ptr.read[S]() }
		print(buf, S.size)
		print(s.ok, !s.ok && true, s.ok == b32.(true), bool.(s.ok), type(s.ok))
	"};
	check(
		src,
		["[1, 9, 9, 9, 1, 0, 0, 0, 2, 9, 9, 9] 12", "true false true true b32"],
	);
}
