use crate::helpers::*;
use indoc::indoc;

#[test]
fn field_access() {
	check(
		"Point :: struct { x: int, y: int }
		point :: Point.{ x = 1, y = 2 }
		point.x",
		"1",
	);
	check(
		"Point :: struct { x: int, y: int }
		point :: Point.{ x = 1, y = 2 }
		point.y",
		"2",
	);
}

#[test]
fn zero_value() {
	check(
		"Point :: struct { x: int, y: int }
		origin :: Point.{}
		origin.x",
		"0",
	);
	check(
		"User :: struct { name: string, age: int }
		u :: User.{}
		u.age",
		"0",
	);
}

#[test]
fn positional_literal() {
	check(
		"Point :: struct { x: int, y: int }
		p :: Point.{3, 4}
		p.x",
		"3",
	);
	check(
		"Point :: struct { x: int, y: int }
		p :: Point.{3, 4}
		p.y",
		"4",
	);
}

#[test]
fn partial_positional_literal() {
	check(
		"Point :: struct { x: int, y: int }
		p :: Point.{3}
		p.y",
		"0",
	);
	fail(
		"Point :: struct { x: int, y: int }
		Point.{3, 4, 5}",
		"has 2 fields but 3 values were provided",
	);
}

#[test]
fn field_mutation() {
	check(
		"Point :: struct { x: int, y: int }
		p := Point.{}
		p.x = 5
		p.x",
		"5",
	);
	check(
		"Point :: struct { x: int, y: int }
		p := Point.{ x = 10, y = 20 }
		p.y = 99
		p.y",
		"99",
	);
}

#[test]
fn copy_semantics() {
	check(
		"Point :: struct { x: int, y: int }
		a :: Point.{ x = 1, y = 2 }
		b := a
		b.x = 99
		a.x",
		"1",
	);
}

#[test]
fn copy_of_array_field_is_independent() {
	check(
		"Bag :: struct { items: []int }
		s :: Bag.{ items = [1, 2, 3] }
		b := s.items
		b << 4
		s.items",
		"[1, 2, 3]",
	);
}

#[test]
fn struct_lit_copies_array_field() {
	check(
		"Bag :: struct { items: []int }
		a := [1]
		bags :: [Bag.{ items = a }]
		a << 2
		bags[0].items",
		"[1]",
	);
}

#[test]
fn print_struct() {
	check(
		"Point :: struct { x: int, y: int }
		print(Point.{ x = 1, y = 2 })",
		"Point.{x = 1, y = 2}",
	);
	check(
		"Point :: struct { x: int, y: int }
		print(Point.{})",
		"Point.{x = 0, y = 0}",
	);
}

#[test]
fn mixed_field_types() {
	check(
		r#"Foo :: struct { n: int, s: string, f: float }
		v :: Foo.{ n = 42, s = "hi", f = 1.5 }
		v.n"#,
		"42",
	);
	check(
		r#"Foo :: struct { n: int, s: string }
		v :: Foo.{ n = 7, s = "world" }
		v.s"#,
		"world",
	);
}

#[test]
fn fn_return_type_annotation() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		origin :: fn() Point { Point.{} }
		origin()
	"};
	check(src, "Point.{x = 0, y = 0}");
}

#[test]
fn fn_return_type_annotation_mismatch() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		bad :: fn() Point { 42 }
		bad()
	"};
	fail(src, "wrong return type");
}

#[test]
fn fn_param_struct_type() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		sum :: fn(p: Point) int { p.x + p.y }
		sum(Point.{ x = 3, y = 4 })
	"};
	check(src, "7");
}

#[test]
fn if_no_else_struct_zero() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		p :: if false { Point.{ x = 1, y = 2 } }
		p.x
	"};
	check(src, "0");
}

#[test]
fn immutable_field_assign_error() {
	fail(
		"Point :: struct { x: int, y: int }
		p :: Point.{}
		p.x = 5",
		"immutable",
	);
}

#[test]
fn struct_positional_field_access() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		p :: Point.{ 2 4 }
		p.1 == p.y
	"};
	check(src, "true");
}

#[test]
fn omitted_name_coerces_to_struct() {
	check(
		"Point :: struct { x: int, y: int }
		p : Point : .{ x = 2, y = 1 }
		p.x + p.y",
		"3",
	);
	check(
		"Point :: struct { x: int, y: int }
		x :: 5
		y :: 7
		p : Point : .{ x, y }
		p.y",
		"7",
	);
}

#[test]
fn omitted_name_as_call_arg() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		sum :: fn(p: Point) int { p.x + p.y }
		sum(.{ x = 3, y = 4 })
	"};
	check(src, "7");
}

#[test]
fn omitted_name_in_return_position() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		make :: fn() Point { .{ x = 1, y = 2 } }
		make()
	"};
	check(src, "Point.{x = 1, y = 2}");
}

#[test]
fn empty_literal_defaults_struct() {
	check(
		"User :: struct { age: int, swag: int = 5 }
		u : User : .{}
		u.swag",
		"5",
	);
}

#[test]
fn unknown_field_error() {
	fail(
		"Point :: struct { x: int, y: int }
		p : Point : .{ z = 1 }",
		"no field `z`",
	);
}

#[test]
fn mixed_positional_and_named_fields() {
	check(
		"Point :: struct { x: int, y: int }
		p :: Point.{ 3, y = 9 }
		p.x + p.y",
		"12",
	);
	fail(
		"Point :: struct { x: int, y: int }
		p :: Point.{ 3, x = 9 }",
		"`x` was already set positionally",
	);
}

#[test]
fn default_field_value() {
	// empty literal uses the default
	check(
		"User :: struct { age: int, name: string, swag: int = 5 }
		u :: User.{}
		u.swag",
		"5",
	);
	// partial named literal
	check(
		"User :: struct { age: int, swag: int = 5 }
		u :: User.{ age = 30 }
		u.swag",
		"5",
	);
	// explicit value overrides the default
	check(
		"User :: struct { age: int, swag: int = 5 }
		u :: User.{ swag = 99 }
		u.swag",
		"99",
	);
	// non-defaulted fields still zero-init
	check(
		"User :: struct { age: int, swag: int = 5 }
		u :: User.{}
		u.age",
		"0",
	);
}

#[test]
fn named_call_args() {
	check(
		"Options :: struct { foo: int, bar: bool }
		f :: fn(o: Options) { print(o.foo) }
		f(bar = true, foo = 4)",
		"4",
	);
}

#[test]
fn named_method_args() {
	check(
		"Options :: struct { foo: int, bar: bool }
		User :: struct {}
		User :< {
			with_options :: fn(self, opt: Options) { print(opt.bar) }
		}
		user :: User.{}
		user.with_options(bar = true, foo = 4)",
		"true",
	);
}

#[test]
fn mixed_positional_and_named_args() {
	check(
		"Options :: struct { foo: int }
		g :: fn(x: int, o: Options) { print(x + o.foo) }
		g(1, foo = 2)",
		"3",
	);
}

#[test]
fn named_before_positional_error() {
	fail(
		"Options :: struct { foo: int }
		g :: fn(x: int, o: Options) {}
		g(foo = 1, 2)",
		"positional args go before named args",
	);
}

#[test]
fn struct_typed_field() {
	let src = indoc! {"
		Money :: struct { amount: int }
		Wallet :: struct { cash: Money }
		w :: Wallet.{ cash = Money.{ amount = 5 } }
		print(w.cash.amount)
		print(w)
	"};
	check(src, ["5", "Wallet.{cash = Money.{amount = 5}}"]);
}

#[test]
fn struct_typed_field_out_of_order() {
	let src = indoc! {"
		Wallet :: struct { cash: Money }
		Money :: struct { amount: int }
		Wallet.{ cash = Money.{ amount = 7 } }.cash.amount
	"};
	check(src, "7");
}

#[test]
fn struct_typed_field_reassign() {
	let src = indoc! {"
		Money :: struct { amount: int }
		Wallet :: struct { cash: Money }
		w := Wallet.{ cash = Money.{ amount = 5 } }
		w.cash = Money.{ amount = 9 }
		w.cash.amount
	"};
	check(src, "9");
}

#[test]
fn self_recursive_struct_error() {
	fail("A :: struct { a: A }", "recurses for ever ever");
}

#[test]
fn mutually_recursive_structs_error() {
	fail(
		"A :: struct { b: B }
		B :: struct { a: A }",
		"recurses for ever ever",
	);
}

#[test]
fn unknown_field_type_error() {
	fail("Wallet :: struct { cash: Money }", "unknown type `Money`");
}

#[test]
fn append_infers_anon_literal_from_element_type() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		pts := [ Point.{ 1, 2 } ]
		pts << .{ 3, 4 }
		pts[1].y
	"#};
	check(src, "4");
}

#[test]
fn omitted_literal_as_call_arg() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		sum :: fn(p: Point) int { p.x + p.y }
		x :: 3
		y :: 4
		sum(.{ x, y })
	"};
	check(src, "7");
}

#[test]
fn struct_update_spread() {
	let src = indoc! {r#"
		User :: struct {
			name: string
			age: int
			is_registered: bool
		}
		register :: fn(u: User) User {
			return User.{
				..u
				is_registered = true
			}
		}
		u :: User.{ name = "abc", age = 23 }
		print(u.is_registered)
		print(register(u).name)
		register(u).is_registered
	"#};
	check(src, ["false", "abc", "true"]);
}

#[test]
fn spread_is_overwritten_by_later_fields() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		p :: Point.{ x = 1, y = 2 }
		Point.{ ..p, y = 9 }.y
	"};
	check(src, "9");
}

#[test]
fn spread_of_other_struct_error() {
	fail(
		"A :: struct { x: int }
		B :: struct { x: int }
		A.{ ..B.{ x = 1 } }.x",
		"cannot spread B into `A`",
	);
}

#[test]
fn embedded_structs() {
	let src = indoc! {r#"
		Options :: struct { foo: int, bar: int = 7 }
		Profile :: struct {
			Options
			name: string
		}
		profile := Profile.{ foo = 4, name = "one cool dude" }
		print(profile.foo == profile.Options.foo)
		print(profile.bar)
		profile.Options = Options.{ foo = 1 }
		print(profile.foo)
		profile.bar = 9
		profile.bar
	"#};
	check(src, ["true", "7", "1", "9"]);
}

#[test]
fn embedded_method_promotion() {
	let src = indoc! {"
		Options :: struct { foo: int }
		Options :< {
			show :: fn(self) int { self.foo }
			bump :: fn(mut self) { self.foo = self.foo + 1 }
		}
		Profile :: struct { Options }
		p := Profile.{ foo = 4 }
		print(p.show())
		p.bump()
		p.foo
	"};
	check(src, ["4", "5"]);
}

#[test]
fn embedded_two_levels() {
	let src = indoc! {"
		A :: struct { x: int }
		A :< { hi :: fn(self) int { self.x + 1 } }
		B :: struct { A, y: int }
		C :: struct { B, z: int }
		c := C.{ x = 1, y = 2, z = 3 }
		print(c.x)
		print(c.hi())
		c.x = 5
		c.hi()
	"};
	check(src, ["1", "2", "6"]);
}

#[test]
fn embedded_via_alias() {
	let src = indoc! {"
		Widget :: struct { x: int = 3 }
		W :: Widget
		Button :: struct { W }
		assert!(Button.{}.x == Button.{}.W.x)
		Button.{}.x
	"};
	check(src, "3");
}

#[test]
fn embedded_ambiguous_field() {
	fail(
		"A :: struct { x: int }
		B :: struct { x: int }
		C :: struct { A, B }
		C.{}.x",
		"`x` is ambiguous, found in embedded `A` and `B`",
	);
}

#[test]
fn anonymous_field_type() {
	let src = indoc! {r#"
		Food :: struct {
			name: string
			nutrition: struct {
				calories: int
			}
		}
		apple :: Food.{ name = "apple", nutrition = .{ calories = 4 } }
		pear :: Food.{ name = "pear", nutrition = .{ 5 } }
		print(apple.nutrition.calories)
		print(pear.nutrition)
	"#};
	check(src, ["4", ".{calories = 5}"]);
}

#[test]
fn anonymous_type_positions() {
	check(
		indoc! {"
			f :: fn(p: struct { x: int }) { print(p.x) }
			f(.{ x = 4 })
		"},
		"4",
	);
	check(
		indoc! {"
			f :: fn() struct { x: int } { .{ x = 4 } }
			f().x
		"},
		"4",
	);
	check(
		indoc! {"
			f :: fn(xs: []struct { x: int }) { print(xs[0].x + xs[1].x) }
			f(.[ .{ x = 1 }, .{ x = 2 } ])
		"},
		"3",
	);
}

#[test]
fn anonymous_type_rejected_as_middle() {
	fail("x : struct { a: int }", "");
}

#[test]
fn anonymous_structural_identity() {
	let src = indoc! {"
		A :: struct { n: struct { calories: int } }
		B :: struct { m: struct { calories: int } }
		a :: A.{ n = .{ 4 } }
		b := B.{ m = .{ 9 } }
		x := a.n
		b.m = x
		b.m.calories
	"};
	check(src, "4");
}

#[test]
fn anonymous_inferred_from_the_literal() {
	check(
		indoc! {"
			pos := .{ x = 1, y = 2 }
			pos.x + pos.y
		"},
		"3",
	);
	check(
		indoc! {"
			f :: fn(p: struct { x: int, y: int }) { print(p.x + p.y) }
			pos := .{ x = 1, y = 2 }
			f(pos)
		"},
		"3",
	);
	fail("p := .{ 5 }", "cannot infer the struct type");
}

#[test]
fn nested_struct_survives_return() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		Config :: struct { origin: Point, name: string }
		make :: fn() Config { Config.{ Point.{ 3, 4 }, "grid" } }
		C := make()
		print("{C.origin.y}")
	"#};
	check(src, "4");
}

#[test]
fn nested_struct_field_copy_is_independent() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		Config :: struct { origin: Point, name: string }
		p :: Config.{ Point.{ 3, 4 }, "grid" }
		q := p
		q.origin = Point.{ 99, 4 }
		p.origin.x
	"#};
	check(src, "3");
}

#[test]
fn duplicate_field_errors() {
	fail(
		indoc! {"
			F :: struct { x: int }
			F.{ x = 1, x = 2 }
		"},
		"`x` is repeated",
	);
	fail("p := .{ x = 1, x = 2 }", "`x` is repeated");
}

#[test]
fn quoted_struct_def_in_fn_body_errors() {
	fail(
		indoc! {r#"
			mk! :: fn() Ast { `P :: struct { x: int }` }
			f :: fn() { mk!() }
			f()
		"#},
		"definitions are only allowed at the top level",
	);
}
