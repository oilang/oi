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
