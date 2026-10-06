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

#[test]
fn control_binds() {
	let src = indoc! {"
		P :: struct { x: int, y: int }
		loop with p in .[P.{ 1, 2 }, P.{ 3, 4 }] { print(x + y) }
		o: ?P = P.{ 5, 6 }
		if with p := o do print(x * y)
		f :: fn(v: P | int) int { match v { with q @ P => x - y, int => 0 } }
		print(f(P.{ 9, 1 }))
	"};
	check(src, ["3", "7", "30", "8"]);
}

#[test]
fn item_subjects() {
	let src = indoc! {"
		use math
		P :: struct { x: int }
		P :< { origin :: fn() P { P.{ 4 } } of[T] :: fn(v: T) P { P.{ 5 } } }
		with math { print(max(2, 7)) }
		with P { print(origin().x + of(true).x) }
	"};
	check(src, ["7", "9"]);
}
