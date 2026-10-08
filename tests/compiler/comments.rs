use crate::helpers::*;

#[test]
fn comments() {
	let src = indoc! {"
		# line comment
		## a note
		#{
			this is a
			block comment
		}#
		#{ a } b }# #{ outer #{ inner }# still outer }#
		print(1 + #{ skip this }# 1) # trailing
		2 + 3 #{ skipped }#
	"};
	check(src, ["2", "5"]);
}

#[test]
fn doc_comments() {
	let src = indoc! {"
		## Doc comments.
		##
		## # support markdown
		## ```json
		## [ 2, 4, 6 ]
		## ```
		## - item
		## 1. one
		add :: fn(a: int, b: int) int {
			## intermediate step
			x :: a * 6
			x + b
		}
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
		T :: trait {
			## area
			area: fn(self) int
		}
		e := E.red
		p := P.{ 1 }
		add(7, p.x)
	"};
	check(src, "43");
}
