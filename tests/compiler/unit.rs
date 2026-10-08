use crate::helpers::*;

#[test]
fn unit_literal() {
	check(
		["print(() == (), () != (), ((), ()))", "x :: ()", "x"],
		"true false ((), ())",
	);
}

#[test]
fn unit_fns() {
	let src = indoc! {"
		nada :: fn() {}
		nope :: fn() { () }
		no_way :: fn() { return () }
		nuh_uh :: fn() { return }
		zilch :: fn() () {}
		print(nada(), nope(), no_way(), nuh_uh(), zilch())
		print(nada() == zilch())
		nada()
	"};
	check(src, ["() () () () ()", "true"]);
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
