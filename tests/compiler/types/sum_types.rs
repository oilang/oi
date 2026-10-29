use crate::helpers::*;

#[test]
fn atom_sum_basics() {
	check(
		indoc! {"
			Status :: :ok | :err
			Res :: struct { s: Status }
			f :: fn() Status { :err }
			x : Status : :ok
			z: Status
			e : Status : :err
			y: :ok | :err : :ok
			print(f(), x, z, Res.{ s = :err }.s, y)
			print(x == z, x == e, x != e)
		"},
		["err ok ok err ok", "true false true"],
	);
}

#[test]
fn matching() {
	check(
		indoc! {r#"
			Status :: :ok | :err
			x : Status : :err
			match x {
				:ok => "good",
				:err => "bad",
			}
		"#},
		"bad",
	);
	check(
		indoc! {r#"
			Status :: :ok | :err
			x : Status : :err
			match x {
				:ok => "good",
				_ => "fallback",
			}
		"#},
		"fallback",
	);
	fail(
		indoc! {r#"
			Status :: :ok | :err
			x : Status : :err
			match x {
				:ok => "good",
			}
		"#},
		"non-exhaustive match, missing: err",
	);
}

#[test]
fn rejections() {
	fail(["Status :: :ok | :err", "x : Status : :nope"], "has no atom `:nope`");
	fail(
		["Status :: :ok | :ok", "f :: fn() Status { :ok }", "f()"],
		"duplicate atom `:ok` in sum type",
	);
	fail(
		["Bad :: int | int", "x: Bad", "x"],
		"duplicate member `int` in sum type",
	);
	fail(
		["Num :: int | f64", "Bad :: Num | int", "x: Bad", "x"],
		"duplicate member `int` in sum type",
	);
	fail(
		["Id :: int | string", "x : Id : 4", "int.(x)"],
		"cannot extract a sum member by casting",
	);
}

#[test]
fn anonymous_sum_param_and_return() {
	check(
		indoc! {r#"
			f :: fn(v: int | string) int | string { v }
			a := match n := f(7) {
				is int => n + 1,
				is string => 0,
			}
			b := match s := f("hi") {
				is int => "no",
				is string => s,
			}
			print(a, b)
		"#},
		"8 hi",
	);
}

#[test]
fn tight_prefix_precedence() {
	check(
		indoc! {"
			V :: :none | []int | :other
			x : V : :other
			y : V = :none
			y = [1, 2]
			print(x, y)
		"},
		"other [1, 2]",
	);
}

#[test]
fn general_basics() {
	check(
		indoc! {r#"
			Id :: int | string
			V :: :none | int
			Box :: struct { id: Id }
			make :: fn() Id { 42 }
			x : Id : 7
			z: Id
			r : Id = 7
			r = "hi"
			v : V : :none
			w : V = :none
			w = 5
			print(x, z, r, make(), Box.{ id = "hey" }.id, v, w, r == "hi")
		"#},
		"7 0 hi 42 hey none 5 true",
	);
}

#[test]
fn general_eq_is_structural() {
	check(
		indoc! {r#"
			Id :: int | string
			A :: int | string
			B :: int | string
			a : Id : 7
			b : Id : 7
			c : Id : "x"
			d : A : 1
			e : B : 1
			print(a == b, a == c, d == e)
		"#},
		"true false true",
	);
}

#[test]
fn set_identity() {
	check(
		indoc! {r#"
			A :: int | string
			B :: string | int
			C :: :ok | :err
			D :: :err | :ok
			a : A : 7
			b : B : a
			c : C : :ok
			d : D : c
			z: B
			n := match b {
				is int => b + 1,
				is string => 0,
			}
			print(n, a == b, z == "", d == :ok, c == d)
		"#},
		"8 true true true true",
	);
}

#[test]
fn sum_alias_splices() {
	check(
		indoc! {r#"
			Num :: int | f64
			Value :: Num | string
			Status :: :ok | :err
			V :: Status | int
			kind :: fn(x: Value) int {
				match x {
					is int => 1,
					is f64 => 2,
					is string => 3,
				}
			}
			e : V : :err
			n : V : 5
			print(kind(7), kind("hi"), e == :err, n == 5)
		"#},
		"1 3 true true",
	);
}

#[test]
fn literals_take_the_hint_through_a_sum() {
	check(
		indoc! {r#"
			Arr :: []Json
			Obj :: [string]Json
			Json :: :null | bool | float | string | Arr | Obj
			a : Json : [true, 1.5]
			m : Json : ["k" = true]
			n := match a { is Arr => a.len(), _ => -1 }
			k := match m { is Obj => m.len(), _ => -1 }
			print(n, k)
		"#},
		"2 1",
	);
}

#[test]
fn recursive_members() {
	check(
		indoc! {"
			Arr :: []Json
			Json :: :null | bool | float | string | Arr | [string]Json
			size :: fn(j: Json) int {
				match j {
					is Arr => j.len(),
					_ => 0,
				}
			}
			b : Json : true
			inner : Json : [b, b, b]
			top : Json : [inner]
			match top {
				is Arr => size(top[0]),
				_ => -1,
			}
		"},
		"3",
	);
}

#[test]
fn printing_a_recursive_sum_terminates() {
	check(
		indoc! {"
			Json :: :null | bool | float | string | []Json | [string]Json
			b : Json : true
			top : Json : [b]
			print(top)
		"},
		"[true]",
	);
}

#[test]
fn container_members_match_as_types() {
	check(
		indoc! {r#"
			Json :: :null | bool | float | string | []Json | [string]Json
			show :: fn(j: Json) string {
				match j {
					is []Json => "arr of {j.len()}",
					is [string]Json => "map of {j.len()}",
					_ => "other",
				}
			}
			b : Json : true
			arr : Json : [b, b]
			map : Json : ["a" = b]
			print(show(arr))
			print(show(map))
			print(show(:null))
		"#},
		["arr of 2", "map of 1", "other"],
	);
}

#[test]
fn a_member_returns_through_a_result() {
	check(
		indoc! {"
			Json :: :null | bool | string
			f :: fn() !Json { true }
			print(f() or { :null })
		"},
		"true",
	);
}

#[test]
fn sums_as_errors() {
	let types = indoc! {"
		A :: struct { m: string }
		B :: struct { n: int }
		E :: A | B
	"};

	check(
		[
			types,
			indoc! {r#"
				pick :: fn(x: int) E!int {
					if x < 0 { return A.{ m = "neg" } }
					if x == 0 { return B.{ n = 0 } }
					x * 2
				}
				show :: fn(x: int) int {
					pick(x) or match $ {
						is A => { print($.m) 1 }
						is B => { print($.n) 2 }
					}
				}
				print(show(3))
				print(show(-1))
				print(show(0))
			"#},
		],
		["6", "neg", "1", "0", "2"],
	);
	check(
		[
			types,
			indoc! {r#"
				inner :: fn() A!int { return A.{ m = "boom" } }
				outer :: fn() E!int { inner()? + 1 }
				print(outer() or match $ {
					is A => { print($.m) 1 }
					is B => 2,
				})
			"#},
		],
		["boom", "1"],
	);
	check([types, "f :: fn() (A | B)!int { 2 }"], "");
}

#[test]
fn is_variant() {
	check(
		indoc! {"
			Id :: int | string
			id: Id = 7
			x: ?int = 3
			print(id is int, id is not string, x is .some, x is .none)
		"},
		"true true true false",
	);
}

#[test]
fn is_unwrapping() {
	check(
		indoc! {r#"
			P :: struct { x: int, y: int }
			f :: fn(x: int | string | bool) string {
				if x is int { return "int {x + 1}" }
				if x is not string { "other {x}" } else { "str {x.len()}" }
			}
			print(f(4), f("abc"), f(true))
			v: P | string = P.{ x = 1, y = 2 }
			w := v
			if v is P && v.x == 1 { v.x = 9 }
			a: any = "x"
			loop a is string && a.len() < 3 { a = a + "y" }
			print(v, w, a)
		"#},
		["int 5 str 3 other true", "P.{x = 9, y = 2} P.{x = 1, y = 2} xyy"],
	);
	fail(
		["y: int | string = 5", r#"if y is int { y = "s" }"#],
		"cannot assign string to `y`, which is int",
	);

	check(
		indoc! {r#"
			Id :: int | string | []int
			g :: fn() Id { [1, 2] }
			print(match n := g() { is int => n + 1, is []int => n.len(), is string => n.len() })
		"#},
		"2",
	);
}

#[test]
fn place_paths_unwrap() {
	let defs = indoc! {"
		Shape :: enum { circle { radius: int } point }
		P :: struct { q: struct { shape: Shape } }
		p := P.{ q = .{ shape = .circle.{ radius = 2 } } }
	"};
	check([defs, "if p.q.shape is .circle { print(p.q.shape.radius) }"], ["2"]);
	let reassigned = "if p.q.shape is .circle { p = P.{ q = .{ shape = .point } }\n print(p.q.shape.radius) }";
	fail([defs, reassigned], "cannot access a field of Shape");
	let looped =
		"loop { if p.q.shape is .circle { p = P.{ q = .{ shape = .point } } } else { print(p.q.shape.radius) } }";
	fail([defs, looped], "cannot access a field of Shape");
}

#[test]
fn unwrapped_field_writes_land_in_place() {
	check(
		indoc! {"
			Shape :: enum { circle { radius: int } point }
			s: Shape = .circle.{ radius = 2 }
			if s is .circle { s.radius = 5 }
			print(s)
		"},
		"circle.{radius = 5}",
	);
}

#[test]
fn early_exit_unwraps_the_rest() {
	check(
		indoc! {r#"
			Id :: int | string | bool
			g :: fn(x: Id) string {
				if x is int { return "int" }
				if x is bool { return "bool {x}" }
				"str {x.len()}"
			}
			print(g(1), g(true), g("abc"))
		"#},
		"int bool true str 3",
	);
}
