use crate::helpers::*;

#[test]
fn coerce_and_match() {
	check(
		indoc! {r#"
			xs: []any = [1, "two"]
			loop x in xs {
				match x {
					is int => print(x + 1),
					is string => print(x),
					else => print("?"),
				}
			}
		"#},
		["2", "two"],
	);
}

#[test]
fn zero_value_falls_to_else() {
	check(
		indoc! {"
			x: any
			match x {
				is int => x,
				else => -1,
			}
		"},
		"-1",
	);
}

#[test]
fn needs_else() {
	fail(
		indoc! {"
			x: any = 7
			match x { is int => x }
		"},
		"needs `else`",
	);
}

#[test]
fn assertion_casts() {
	check(["x: any = 7", "int.(x)"], "some.(7)");
	check(["x: any = 7", "string.(x)"], "none");
}

#[test]
fn print_dispatches_on_typeid() {
	check(
		indoc! {r#"
			P :: struct { x: int }
			xs: []any = [3, "hi", P.{ x = 4 }]
			loop x in xs { print(x) }
			y: any
			print(y)
		"#},
		["3", "hi", "P.{x = 4}", "<any>"],
	);
}

#[test]
fn eq_dispatches_on_typeid() {
	check(
		indoc! {r#"
			P :: struct { x: int }
			a: any = P.{ x = 1 }
			b: any = P.{ x = 1 }
			c: any = "1"
			z: any
			print(a == b, a == c, z == z, typeid.(a), typeid.(a) == typeid.(3))
		"#},
		"true false true P false",
	);
}

#[test]
fn ref_payload_cycle_is_collected() {
	assert_clean(["N :: struct { v: any }", "a := &N.{}", "a.v = a", "0"]);
}

#[test]
fn every_payload_kind_is_released() {
	assert_clean(indoc! {r#"
		P :: struct { s: string }
		E :: enum { A(string), B }
		xs: []any = [P.{s = "p"}, (1, "t"), [2]string.["a", "b"], E.A.("e")]
		print(xs.len())
	"#});
}

#[test]
fn a_resource_moves_into_any_and_drops_once() {
	check(
		indoc! {r#"
			File :: struct { fd: int }
			File : Drop < { drop :: fn(mut self) { print("drop", self.fd) } }
			f := File.{fd = 1}
			x: any = f
			r := &File.{fd = 2}
			print("end")
		"#},
		["end", "drop 2", "drop 1"],
	);
}

#[test]
fn a_kept_lent_any_outlives_the_caller() {
	let src = indoc! {r#"
		P :: struct { s: string }
		keep :: fn(x: any) []any { [x] }
		pack :: fn(xs: ..any) []any { xs }
		make :: fn() []any {
			p := P.{ s = "p" + "!" }
			[..keep(p), ..pack(p)]
		}
		main :: fn() { print(make()) }
	"#};
	check(src, r#"[P.{s = "p!"}, P.{s = "p!"}]"#);
	assert_clean(src);
}

#[test]
fn a_resource_lent_as_any_drops_once() {
	check(
		indoc! {r#"
			File :: struct { fd: int }
			File : Drop < { drop :: fn(mut self) { print("drop", self.fd) } }
			show :: fn(xs: ..any) { print(xs) }
			f := File.{fd = 1}
			show(f, 2)
			print("end")
		"#},
		["[File.{fd = 1}, 2]", "end", "drop 1"],
	);
}

#[test]
fn type_of_is_static() {
	check(
		indoc! {r#"
			f :: fn() rune { print("ran"); 'a' }
			main :: fn() {
				a: any = 1
				zs: []type(f()) = ['x']
				print(type(a) == any, typeid.(a) == int, type(f()) == rune, typeid.(rune), zs)
			}
		"#},
		"true true true rune [x]",
	);
}

#[test]
fn local_type_binds_alias() {
	check(
		indoc! {"
			main :: fn() {
				x := 3
				t :: type(x)
				b :: []t
				y: b = .[1, 2]
				print(y)
			}
		"},
		"[1, 2]",
	);
}

#[test]
fn type_of_static_in_signatures() {
	check(
		indoc! {"
			x := 3
			f :: fn(a: type(x)) type(x) { a + 1 }
			S :: struct { v: type(x) }
			print(f(2), S.{4}.v)
		"},
		"3 4",
	);
}

#[test]
fn types_are_typeid_values() {
	let src = indoc! {r#"
		B[E] :: struct { e: E }
		B[E] :< { g :: fn(self) { comp if E == []int { print("ints") } else { print("other") } } }
		a: any = [1]
		t: typeid = ()
		print(typeid.(a) == []int, typeid.(a) == [2]int, t == (), int == (), []())
		B.{ e = [1] }.g()
	"#};
	check(src, ["true false true false []()", "ints"]);
}
