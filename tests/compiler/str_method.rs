use crate::helpers::*;

#[test]
fn primitives() {
	check("42.str()", "42");
	check("3.14.str()", "3.14");
	check("true.str()", "true");
	check(r#""hi".str()"#, "hi");
}

#[test]
fn composites() {
	check(r#"(1, "a").str()"#, r#"(1, "a")"#);
	check("[1, 2].str()", "[1, 2]");
}

#[test]
fn variants() {
	check(["Color :: enum { Red, Green }", "Color.Red.str()"], "Red");
	check(["o :: ?int.(none)", "o.str()"], "none");
	check(["r :: !int.(42)", "r.str()"], "ok.(42)");
}

#[test]
fn derived_struct() {
	let src = indoc! {"
		Bag :: struct { items: []int }
		Bag.{[1, 2, 3]}.str()
	"};
	check(src, "Bag.{items = [1, 2, 3]}");
}

#[test]
fn user_str_wins() {
	let src = indoc! {r#"
		Money :: struct { n: int }
		Money :< { str :: fn(self) string { "$" } }
		Money.{5}.str()
	"#};
	check(src, "$");
}

#[test]
fn print_uses_user_str() {
	let src = indoc! {r#"
		Money :: struct { n: int }
		Money :< { str :: fn(self) string { "$" + self.n.str() } }
		m :: Money.{5}
		print(m)
		print([m, m])
	"#};
	check(src, ["$5", "[$5, $5]"]);
}

#[test]
fn print_uses_generic_user_str() {
	let src = indoc! {r#"
		Box[T] :: struct { v: T }
		Box[T] :< { str :: fn(self) string { "box!" } }
		b :: Box.{5}
		print(b)
	"#};
	check(src, "box!");
}

#[test]
fn user_str_nested_in_derived_render() {
	let src = indoc! {r#"
		Money :: struct { n: int }
		Money :< { str :: fn(self) string { "$" + self.n.str() } }
		[Money.{5}, Money.{7}].str()
	"#};
	check(src, "[$5, $7]");
}
