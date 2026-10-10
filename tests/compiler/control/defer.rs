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
fn body_can_be_an_assignment() {
	let src = indoc! {"
		x := 0
		loop {
			defer x += 1
			if x >= 2 { break }
		}
		print(x)
	"};
	check(src, "3");
}

#[test]
fn dollar_is_the_returned_value_or_its_error() {
	let src = indoc! {r#"
		f :: fn(bad: bool) !int {
			defer print($ or -1)
			defer or print($)
			defer and print("ok {$}")
			if bad { return error("boom") }
			1
		}
		print(f(false) or -2)
		print(f(true) or -2)
	"#};
	check(src, ["ok 1", "1", "1", "boom", "-1", "-2"]);
}

#[test]
fn body_cannot_leave_the_scope() {
	fail(
		["f :: fn() int { defer { return 1 }; return 2 }", "f()"],
		"cannot return from a defer body",
	);
	fail("defer break", "outside of a loop");
	fail(
		["f :: fn() int { defer or print(0); return 1 }", "f()"],
		"`defer or` needs a fn returning",
	);
}

#[test]
fn is_a_unit_value() {
	check("print({ print(0); defer print(1) })", ["0", "1", "()"]);
	check(
		[
			"m! :: fn() Ast { `defer print(1)` }",
			"f :: fn() { print(m!()); print(2) }",
			"f()",
		],
		["()", "2", "1"],
	);
}

#[test]
fn and_or_defer() {
	let src = indoc! {"
		f :: fn(o: ?int) {
			o and defer print(1)
			o or defer print(2)
			o or print(3)
			print(0)
		}
		f(5)
		f(none)
		i := 0
		loop { i += 1; if i == 2 do defer print(i); if i > 2 { break } }
	"};
	check(src, ["0", "1", "3", "0", "2", "2"]);
}

#[test]
fn bare_return_reads_named_result_after_defers() {
	let src = indoc! {"
		f :: fn() n: int {
			defer { n = 456 }
			n = 123
			return
		}
		print(f())
	"};
	check(src, "456");
}
