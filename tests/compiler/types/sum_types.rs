use crate::helpers::*;

#[test]
fn tail_return() {
	check(
		indoc! {"
			Status :: :ok | :err
			f :: fn() Status { :err }
			f()
		"},
		"err",
	);
}

#[test]
fn bind_and_print() {
	check(
		indoc! {"
			Status :: :ok | :err
			x : Status : :ok
			x
		"},
		"ok",
	);
}

#[test]
fn zero_value_is_first_variant() {
	check(
		indoc! {"
			Status :: :ok | :err
			x: Status
			x
		"},
		"ok",
	);
}

#[test]
fn eq() {
	check(
		indoc! {"
			Status :: :ok | :err
			a : Status : :ok
			b : Status : :ok
			a == b
		"},
		"true",
	);
	check(
		indoc! {"
			Status :: :ok | :err
			a : Status : :ok
			b : Status : :err
			a == b
		"},
		"false",
	);
	check(
		indoc! {"
			Status :: :ok | :err
			a : Status : :ok
			b : Status : :err
			a != b
		"},
		"true",
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
fn unknown_atom_errors() {
	fail(
		indoc! {"
			Status :: :ok | :err
			x : Status : :nope
		"},
		"has no atom `:nope`",
	);
}

#[test]
fn duplicate_atom_in_type_errors() {
	fail(
		indoc! {"
			Status :: :ok | :ok
			f :: fn() Status { :ok }
			f()
		"},
		"duplicate atom `:ok` in sum type",
	);
}

#[test]
fn ord_gives_tag() {
	check(
		indoc! {"
			Status :: :ok | :err
			x : Status : :ok
			ord(x)
		"},
		"0",
	);
	check(
		indoc! {"
			Status :: :ok | :err
			x : Status : :err
			ord(x)
		"},
		"1",
	);
}

#[test]
fn struct_field_type() {
	check(
		indoc! {"
			Status :: :ok | :err
			Res :: struct { s: Status }
			r :: Res.{ s = :err }
			r.s
		"},
		"err",
	);
}

#[test]
fn anonymous_sum_in_bind() {
	check(
		indoc! {"
			x: :ok | :err : :ok
			x
		"},
		"ok",
	);
}

#[test]
fn anonymous_sum_param_and_return() {
	check(
		indoc! {"
			f :: fn(v: int | string) int | string { v }
			match f(7) {
				n @ int => n + 1,
				string => 0,
			}
		"},
		"8",
	);
	check(
		indoc! {r#"
			f :: fn(v: int | string) int | string { v }
			match f("hi") {
				int => "no",
				s @ string => s,
			}
		"#},
		"hi",
	);
}

#[test]
fn tight_prefix_precedence() {
	check(
		indoc! {"
			V :: :none | []int | :other
			x : V : :other
			ord(x)
		"},
		"2",
	);
	check(
		indoc! {"
			V :: :none | []int | :other
			x : V = :none
			x = [1, 2]
			ord(x)
		"},
		"1",
	);
}

#[test]
fn general_bind_print_and_zero() {
	check(
		indoc! {"
			Id :: int | string
			x : Id : 7
			x
		"},
		"7",
	);
	// zero value is the first member's zero
	check(
		indoc! {"
			Id :: int | string
			x: Id
			x
		"},
		"0",
	);
}

#[test]
fn general_reassign_across_members() {
	check(
		indoc! {r#"
			Id :: int | string
			x : Id = 7
			x = "hi"
			x
		"#},
		"hi",
	);
}

#[test]
fn general_fn_return_and_field() {
	check(
		indoc! {"
			Id :: int | string
			make :: fn() Id { 42 }
			make()
		"},
		"42",
	);
	check(
		indoc! {r#"
			Id :: int | string
			Box :: struct { id: Id }
			Box.{ id = "hey" }.id
		"#},
		"hey",
	);
}

#[test]
fn mixed_atom_and_type() {
	check(
		indoc! {"
			V :: :none | int
			x : V : :none
			x
		"},
		"none",
	);
	check(
		indoc! {"
			V :: :none | int
			x : V = :none
			x = 5
			x
		"},
		"5",
	);
}

#[test]
fn general_eq_is_structural() {
	check(
		indoc! {"
			Id :: int | string
			a : Id : 7
			b : Id : 7
			a == b
		"},
		"true",
	);
	check(
		indoc! {r#"
			Id :: int | string
			a : Id : 7
			b : Id : "x"
			a == b
		"#},
		"false",
	);
	check(
		indoc! {"
			A :: int | string
			B :: int | string
			a : A : 1
			b : B : 1
			a == b
		"},
		"true",
	);
}

#[test]
fn set_identity() {
	check(
		indoc! {"
			A :: int | string
			B :: string | int
			a : A : 7
			b : B : a
			match b {
				n @ int => n + 1,
				string => 0,
			}
		"},
		"8",
	);
	check(
		indoc! {"
			A :: int | string
			B :: string | int
			a : A : 7
			b : B : a
			a == b
		"},
		"true",
	);
	check(
		indoc! {"
			A :: int | string
			a : A : 7
			ord(a)
		"},
		"0",
	);
	check(
		indoc! {"
			A :: int | string
			B :: string | int
			a : A : 7
			b : B : a
			ord(b)
		"},
		"1",
	);
	check(
		indoc! {r#"
			B :: string | int
			x: B
			x == ""
		"#},
		"true",
	);
}

#[test]
fn general_ord_gives_tag() {
	check(
		indoc! {r#"
			Id :: int | string
			x : Id : "x"
			ord(x)
		"#},
		"1",
	);
}

#[test]
fn int_cast_on_sum_errors() {
	fail(
		["Id :: int | string", "x : Id : 4", "int.(x)"],
		"cannot extract a sum member by casting",
	);
}

#[test]
fn duplicate_type_member_errors() {
	fail(
		indoc! {"
			Bad :: int | int
			x: Bad
			x
		"},
		"duplicate member `int` in sum type",
	);
}

#[test]
fn nested_sum_alias_splices() {
	check(
		indoc! {r#"
			Num :: int | f64
			Value :: Num | string
			x : Value : 7
			match x {
				int => 1,
				f64 => 2,
				string => 3,
			}
		"#},
		"1",
	);
	check(
		indoc! {r#"
			Num :: int | f64
			Value :: Num | string
			x : Value : "hi"
			match x {
				int => 1,
				f64 => 2,
				string => 3,
			}
		"#},
		"3",
	);
}

#[test]
fn splice_duplicate_member_errors() {
	fail(
		indoc! {"
			Num :: int | f64
			Bad :: Num | int
			x: Bad
			x
		"},
		"duplicate member `int` in sum type",
	);
}

#[test]
fn single_type_stays_transparent_alias() {
	check(
		indoc! {"
			Score :: int
			x : Score : 5
			x + 1
		"},
		"6",
	);
}

#[test]
fn atom_sums_order() {
	check(
		indoc! {"
			A :: :ok | :err
			B :: :err | :ok
			a : A : :ok
			b : B : a
			ord(b)
		"},
		"1",
	);
	check(
		indoc! {"
			A :: :ok | :err
			B :: :err | :ok
			a : A : :ok
			b : B : a
			a == b
		"},
		"true",
	);
}

#[test]
fn atom_sum_alias_splices_as_member() {
	check(
		indoc! {"
			Status :: :ok | :err
			V :: Status | int
			x : V : :err
			ord(x)
		"},
		"1",
	);
	check(
		indoc! {"
			Status :: :ok | :err
			V :: Status | int
			x : V : 5
			ord(x)
		"},
		"2",
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
