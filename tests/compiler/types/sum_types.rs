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
			print(x == z, x == e, x != e, ord(x), ord(e))
		"},
		["err ok ok err ok", "true false true 0 1"],
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
			a := match f(7) {
				n @ int => n + 1,
				string => 0,
			}
			b := match f("hi") {
				int => "no",
				s @ string => s,
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
			print(ord(x), ord(y))
		"},
		"2 1",
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
			print(x, z, r, make(), Box.{ id = "hey" }.id, v, w, ord(r))
		"#},
		"7 0 hi 42 hey none 5 1",
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
				n @ int => n + 1,
				string => 0,
			}
			print(n, a == b, ord(a), ord(b), z == "", ord(d), c == d)
		"#},
		"8 true 0 1 true 1 true",
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
					int => 1,
					f64 => 2,
					string => 3,
				}
			}
			e : V : :err
			n : V : 5
			print(kind(7), kind("hi"), ord(e), ord(n))
		"#},
		"1 3 1 2",
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
			n := match a { x @ Arr => x.len, _ => -1 }
			k := match m { x @ Obj => x.len, _ => -1 }
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
					a @ Arr => a.len,
					_ => 0,
				}
			}
			b : Json : true
			inner : Json : [b, b, b]
			top : Json : [inner]
			match top {
				a @ Arr => size(a[0]),
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
					a @ []Json => "arr of {a.len}",
					m @ [string]Json => "map of {m.len}",
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
						a @ A => { print(a.m) 1 }
						b @ B => { print(b.n) 2 }
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
					a @ A => { print(a.m) 1 }
					b @ B => 2,
				})
			"#},
		],
		["boom", "1"],
	);
	check([types, "f :: fn() (A | B)!int { 2 }"], "")
}
