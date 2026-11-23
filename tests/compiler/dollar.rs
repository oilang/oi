use crate::helpers::*;

#[test]
fn dollar() {
	check(
		oi! {
			scalar :: fn(x: int) int { assert!(x == $); $ }
			one :: fn(x: int,) int { assert!(x == $.0); $.0 }
			two :: fn(x: int, y: int) int { assert!(x == $.0); assert!(y == $.1); $.0 + $.1 }
			pair :: fn(x: int, y: int) (int, int) { $ }
			unit :: fn() bool { $ == () }
			print(scalar(7), one(5), two(3, 4), pair(3, 4), unit())
		},
		"7 5 7 (3, 4) true",
	);
}

#[test]
fn dollar_rejects() {
	fail("f :: fn(x: int) int { $.0 }; f(9)", "cannot access a field of int");
	fail("f :: fn(x: int, y: int) int { $.5 }; f(1, 2)", "out of range");
}

#[test]
fn literal_dollar_takes_mut_param() {
	let src = oi! {
		P :: struct { pos: int, vel: int }
		each :: fn(mut ps: []P, f: fn(mut P)) { loop mut p in ps { f(mut p) } }
		ps := [P.{ pos = 1, vel = 2 }, P.{ pos = 5, vel = 1 }]
		each(mut ps) { $.pos += $.vel }
		print(ps)
	};
	check(src, "[P.{pos = 3, vel = 2}, P.{pos = 6, vel = 1}]");
}
