use crate::helpers::*;

#[test]
fn unit_literal_top_level() {
	check("()", "");
}

#[test]
fn unit_equality() {
	check("() == ()", "true");
}

#[test]
fn unit_inequality() {
	check("() != ()", "false");
}

#[test]
fn unit_in_binding() {
	check("x :: ()\nx", "");
}

#[test]
fn empty_fn_returns_unit() {
	let src = indoc! {"
		nada :: fn() {}
		nada()
	"};
	check(src, "");
}

#[test]
fn fn_explicit_unit_return() {
	let src = indoc! {"
		nope :: fn() {
			()
		}
		nope()
	"};
	check(src, "");
}

#[test]
fn fn_return_unit_keyword() {
	let src = indoc! {"
		no_way :: fn() {
			return ()
		}
		no_way()
	"};
	check(src, "");
}

#[test]
fn fn_bare_return_is_unit() {
	let src = indoc! {"
		nuh_uh :: fn() {
			return
		}
		nuh_uh()
	"};
	check(src, "");
}

#[test]
fn fn_explicit_unit_ret_annotation() {
	let src = indoc! {"
		zilch :: fn() () {}
		zilch()
	"};
	check(src, "");
}

#[test]
fn unit_fns_compare_equal() {
	let src = indoc! {"
		nada :: fn() {}
		zilch :: fn() () {}
		nada() == zilch()
	"};
	check(src, "true");
}

#[test]
fn empty_main_prints_nothing() {
	let src = indoc! {"
		main :: fn() {}
	"};
	check(src, "");
}

#[test]
fn main_discards_its_tail_value() {
	let src = indoc! {"
		main :: fn() {
			print(1)
			2 + 3
		}
	"};
	check(src, "1");
}

#[test]
fn print_unit() {
	let src = indoc! {"
		print(())
		42
	"};
	check(src, ["()", "42"]);
}

#[test]
fn unit_in_tuple() {
	check("((), ())", "((), ())");
}
