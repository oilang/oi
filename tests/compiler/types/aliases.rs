use crate::helpers::*;

#[test]
fn alias_primitive_in_param() {
	let src = indoc! {"
		Score :: int
		double :: fn(s: Score) Score { s * 2 }
		double(21)
	"};
	check(src, "42");
}

#[test]
fn lowercase_alias() {
	let src = indoc! {"
		score :: int
		double :: fn(s: score) score { s * 2 }
		double(21)
	"};
	check(src, "42");
}

#[test]
fn const_copy_is_not_an_alias() {
	check(["X :: 5", "Y :: X", "print(Y)"], "5");
	check(["x :: 5", "y :: x", "print(y)"], "5");
}

#[test]
fn top_level_index_bind() {
	check(["a :: [10, 20, 30]", "i :: 1", "v :: a[i]", "print(v)"], "20");
}

#[test]
fn alias_primitive_in_return() {
	let src = indoc! {r#"
		Name :: string
		greet :: fn() Name { "hello" }
		greet()
	"#};
	check(src, "hello");
}

#[test]
fn alias_chains() {
	let src = indoc! {"
		Meters :: int
		Distance :: Meters
		add :: fn(a: Distance, b: Distance) Distance { a + b }
		add(3, 4)
	"};
	check(src, "7");
}

#[test]
fn alias_tuple_in_param_and_return() {
	let src = indoc! {"
		Point :: (int, int)
		make :: fn(x: int, y: int) Point { (x, y) }
		p :: make(3, 4)
		print(p.0, p.1)
	"};
	check(src, "3 4");
}

#[test]
fn alias_array_in_param() {
	let src = indoc! {"
		Row :: []int
		first :: fn(r: Row) int { r[0] }
		first([10 20 30])
	"};
	check(src, "10");
}

#[test]
fn alias_in_struct_field() {
	let src = indoc! {"
		Hp :: int
		Unit :: struct { hp: Hp }
		u :: Unit.{ hp = 100 }
		u.hp
	"};
	check(src, "100");
}

#[test]
fn fn_type_alias_parses() {
	// NOTE: function type aliases are not yet supported, but for now they parse
	let src = "Op :: fn (int) int";
	run(src);
}

#[test]
fn alias_of_result_long_form() {
	let src = indoc! {r#"
		Found :: Result[int, Error]
		find :: fn(x: int) Found {
			if x > 0 { return x }
			return error("negative")
		}
		find(5)
	"#};
	check(src, "ok.(5)");
}

#[test]
fn unknown_alias_target_errors() {
	let src = indoc! {"
		Foo :: Nope
		f :: fn(x: Foo) Foo { x }
		f(1)
	"};
	fail_with(src, "unknown type");
}
