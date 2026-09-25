use crate::helpers::*;

#[test]
fn comment_line() {
	let src = indoc! {"
		# this is a comment
		2
	"};
	check(src, "2");
}

#[test]
fn comment_inline() {
	let src = indoc! {"
		2 # this is a comment
	"};
	check(src, "2");
}

#[test]
fn doc_before_fn() {
	let src = indoc! {"
		## Adds two numbers.
		add :: fn(a: int, b: int) int { a + b }
		add(3, 4)
	"};
	check(src, "7");
}

#[test]
fn doc_markdown() {
	let src = indoc! {"
		## Doc comments.
		##
		## # support markdown
		## ```json
		## [ 2, 4, 6 ]
		## ```
		## - item
		## - item
		## 1. one
		## 1. two
		## 1. three
		greet :: fn() int { 1 }
		greet()
	"};
	check(src, "1");
}

#[test]
fn doc_inside_fn() {
	let src = indoc! {"
		compute :: fn() int {
			## intermediate step
			x :: 6
			x * 7
		}
		compute()
	"};
	check(src, "42");
}

#[test]
fn doc_on_struct_and_enum_members() {
	let src = indoc! {"
		E :: enum {
			## the red one
			red,
			## record variant
			rgb {
				## channel
				r: int
			},
		}
		P :: struct {
			## x coord
			x: int,
		}
		e := E.red
		p := P.{ 1 }
		p.x
	"};
	check(src, "1");
}

#[test]
fn doc_top_level() {
	check("## a note\n1 + 1", "2");
}

#[test]
fn block_comment_before_expr() {
	check("#{ skipped }# 2 + 3", "5");
}

#[test]
fn block_comment_after_expr() {
	check("2 + 3 #{ skipped }#", "5");
}

#[test]
fn block_comment_multiline() {
	let src = indoc! {"
		#{
			this is a
			block comment
		}#
		2 + 3
	"};
	check(src, "5");
}

#[test]
fn block_comment_brace_inside() {
	// `}` not followed by `#` is fine inside the comment
	check("#{ a } b }# 1 + 1", "2");
}

#[test]
fn block_comment_inline() {
	check("1 + #{ skip this }# 1", "2");
}

#[test]
fn block_comment_nested() {
	check("#{ outer #{ inner }# still outer }# 1 + 1", "2");
}
