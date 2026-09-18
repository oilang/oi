use crate::helpers::*;

#[test]
fn runs_lifo_on_every_exit() {
	let src = indoc! {"
		i := 0
		loop {
			i += 1
			defer print(i)
			defer { print(0) }
			if i < 2 { continue }
			break
		}
		print(9)
	"};
	check(src, ["0", "1", "0", "2", "9"]);
	// the binding is the one in scope at the defer
	check(["x := 1", "defer print(x)", "x := 2", "print(0)"], ["0", "1"]);
}

#[test]
fn dollar_is_the_returned_value_or_its_error() {
	let src = indoc! {r#"
		f :: fn(bad: bool) !int {
			defer print($ or -1)
			defer or print($)
			if bad { return error("boom") }
			1
		}
		print(f(false) or -2)
		print(f(true) or -2)
	"#};
	check(src, ["1", "1", "boom", "-1", "-2"]);
}

#[test]
fn body_cannot_leave_the_scope() {
	fail_with(
		["f :: fn() int { defer { return 1 }; return 2 }", "f()"],
		"cannot return from a defer body",
	);
	fail_with("defer break", "outside of a loop");
	fail_with(
		["f :: fn() int { defer or print(0); return 1 }", "f()"],
		"`defer or` needs a fn returning",
	);
}
