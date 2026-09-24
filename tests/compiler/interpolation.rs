use crate::helpers::*;

#[test]
fn simple_ident() {
	check(
		r#"
			who :: "mom"
			print("hi {who}!")
		"#,
		"hi mom!",
	);
}

#[test]
fn arithmetic_expr() {
	check(r#"print("sum: {2 + 2}")"#, "sum: 4");
}

#[test]
fn field_and_method() {
	let src = indoc! {r#"
		P :: struct { x: int }
		p :: P.{7}
		print("x is {p.x}, doubled {(p.x * 2).str()}")
	"#};
	check(src, "x is 7, doubled 14");
}

#[test]
fn escapes() {
	check(r#"print("use {{braces}} like this")"#, "use {braces} like this");
	check(
		r#"
			who :: "mom"
			print("{{{who}}}")
		"#,
		"{mom}",
	);
}

#[test]
fn multiline() {
	let src = indoc! {r#"
		who :: "mom"
		amount :: 5
		print("
		dear {who},
		you owe {amount}.
		")
	"#};
	check(src, ["", "dear mom,", "you owe 5.", ""]);
}

#[test]
fn user_str_impl() {
	let src = indoc! {r#"
		Money :: struct { n: int }
		Money :< { str :: fn(self) string { "$" + self.n.str() } }
		m :: Money.{5}
		print("cost: {m}")
	"#};
	check(src, "cost: $5");
}

#[test]
fn positions() {
	check(
		r#"
			a :: "x"
			b :: "y"
			print("{a}{b} {a} end{b}")
		"#,
		"xy x endy",
	);
}

#[test]
fn unterminated_fails() {
	fail(r#"print("oops {who")"#, "");
}
