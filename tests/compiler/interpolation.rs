use crate::helpers::*;

#[test]
fn interpolates() {
	let src = indoc! {r#"
		who :: "mom"
		P :: struct { x: int }
		p :: P.{7}
		Money :: struct { n: int }
		Money :< { str :: fn(self) string { "$" + self.n.str() } }
		m :: Money.{5}
		print("hi {who}! sum: {2 + 2}")
		print("x is {p.x}, doubled {(p.x * 2).str()}, cost: {m}")
		print("use {{braces}} like {{{who}}}")
		print("{who}{who} {who} end{who}")
	"#};
	check(
		src,
		[
			"hi mom! sum: 4",
			"x is 7, doubled 14, cost: $5",
			"use {braces} like {mom}",
			"mommom mom endmom",
		],
	);
}

#[test]
fn multiline() {
	let src = indoc! {r#"
		who :: "mom"
		amount :: 5
		print("""
			dear {who},
			you owe:
				{amount}
			""")
	"#};
	check(src, ["dear mom,", "you owe:", "\t5"]);
}

#[test]
fn unterminated_fails() {
	fail(r#"print("oops {who")"#, "");
}

#[test]
fn interpolates_types() {
	let src = indoc! {r#"
		P :: struct { x: int }
		f[T] :: fn() { print("{T}") }
		g[T] :: fn() { comp match T { []E => print("{E}"), _ => {} } }
		f[i33]()
		g[[]u11]()
		print("{int} {P}")
	"#};
	check(src, ["i33", "u11", "int P"]);
}
