use crate::helpers::*;

#[test]
fn alias_primitives() {
	let src = indoc! {r#"
		Score :: int
		score :: int
		Name :: string
		Meters :: int
		Distance :: Meters
		double :: fn(s: Score) Score { s * 2 }
		half :: fn(s: score) score { s / 2 }
		greet :: fn() Name { "hello" }
		add :: fn(a: Distance, b: Distance) Distance { a + b }
		print(double(21), half(8), greet(), add(3, 4))
	"#};
	check(src, "42 4 hello 7");
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
fn alias_compound_types() {
	let src = indoc! {"
		Point :: (int, int)
		Row :: []int
		Hp :: int
		Unit :: struct { hp: Hp }
		make :: fn(x: int, y: int) Point { (x, y) }
		first :: fn(r: Row) int { r[0] }
		p :: make(3, 4)
		print(p.0, p.1, first([10 20 30]), Unit.{ hp = 100 }.hp)
	"};
	check(src, "3 4 10 100");
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
	fail(src, "unknown type");
}

#[test]
fn generic_alias_is_transparent() {
	let src = indoc! {"
		Grid[T] :: [][]T
		g : Grid[int] = [[1 2] [3 4]]
		print(g, Grid[int].[[5]])
	"};
	check(src, "[[1, 2], [3, 4]] [[5]]");
}
