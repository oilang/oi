use crate::helpers::*;

#[test]
fn int_eq() {
	check("2 == 2", "true");
}

#[test]
fn int_ne() {
	check("2 != 3", "true");
}

#[test]
fn int_lt() {
	check("2 < 3", "true");
}

#[test]
fn int_gt() {
	check("2 > 3", "false");
}

#[test]
fn int_le() {
	check("3 <= 3", "true");
}

#[test]
fn int_ge() {
	check("2 >= 3", "false");
}

#[test]
fn float_cmp() {
	check("1.5 < 2.0", "true");
}

#[test]
fn bool_eq() {
	check("true == true", "true");
}

#[test]
fn looser_than_arithmetic() {
	// parses as (1 + 2) < (2 + 2)
	check("1 + 2 < 2 + 2", "true");
}

#[test]
fn equality_looser_than_relational() {
	// parses as (1 < 2) == (3 < 4) -> true == true
	check("1 < 2 == 3 < 4", "true");
}

#[test]
fn mismatched_types() {
	fail(r#"1 < "x""#, "cannot compare");
}

#[test]
fn promoted_cmp() {
	check("1.0 == 1", "true");
	check("1 == 1.0", "true");
	check("1 < 2.0", "true");
}
