use crate::helpers::*;

#[test]
fn builtins() {
	let src = indoc! {r#"
		Color :: enum { Red, Green }
		o :: ?int.(none)
		r :: !int.(42)
		print(42.str(), 3.14.str(), true.str(), "hi".str())
		print((1, "a").str(), [1, 2].str())
		print(Color.Red.str(), o.str(), r.str())
	"#};
	check(src, ["42 3.14 true hi", r#"(1, "a") [1, 2]"#, "Red none ok.(42)"]);
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
		Money :< { str :: fn(self) string { "$" + self.n.str() } }
		print(Money.{5}.str(), [Money.{5}, Money.{7}].str())
	"#};
	check(src, "$5 [$5, $7]");
}

#[test]
fn print_uses_user_str() {
	let src = indoc! {r#"
		Money :: struct { n: int }
		Money :< { str :: fn(self) string { "$" + self.n.str() } }
		Box[T] :: struct { v: T }
		Box[T] :< { str :: fn(self) string { "box!" } }
		m :: Money.{5}
		print(m)
		print([m, m])
		print(Box.{5})
	"#};
	check(src, ["$5", "[$5, $5]", "box!"]);
}
