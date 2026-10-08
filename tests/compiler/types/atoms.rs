use crate::helpers::*;

#[test]
fn atom_literal() {
	check(":foo", ":foo");
	check(":2", ":2");
	check(":28days_later", ":28days_later");
}

#[test]
fn atom_equality() {
	check(
		[
			"a :: :thing",
			"b :: :thing",
			"print(a, a == b, :foo == :bar, :foo != :bar)",
		],
		":thing true false true",
	);
}

#[test]
fn atom_in_match() {
	check(
		r#"x :: :ok
match x {
	:ok => "yes",
	else => "no",
}"#,
		"yes",
	);
}

#[test]
fn atom_return() {
	check("f :: fn() :ok { :ok }\nf()", "ok");
}

#[test]
fn atom_return_type() {
	check("f :: fn() atom { :ok }\nf()", ":ok");
}

#[test]
fn atom_field_type() {
	check(
		indoc! {"
			S :: struct { tag: atom = :none }
			S.{}
		"},
		"S.{tag = :none}",
	);
}
