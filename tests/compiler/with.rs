use crate::helpers::*;

#[test]
fn members() {
	let src = indoc! {"
		P :: struct { x: int, y: int }
		P :< {
			sum :: fn(self) int { self.x + self.y }
			bump :: fn(with mut self) { x += y }
		}
		print(with P.{ 3, 4 } { x * y })
		print(with P.{ 5, 6 } do sum())
		with p := P.{ 1, 2 }
		x = 10
		p.bump()
		y := 7
		print(p.x)
		print(y)
	"};
	check(src, ["12", "11", "12", "7"]);
}

#[test]
fn ambiguous() {
	let src = indoc! {"
		A :: struct { x: int }
		B :: struct { x: int }
		a := A.{ 1 }
		b := B.{ 2 }
		with a, b { print(x) }
	"};
	fail(src, "`x` is ambiguous");
}
