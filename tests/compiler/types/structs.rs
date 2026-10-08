use crate::common::Project;
use crate::helpers::*;
use indoc::indoc;

#[test]
fn literals() {
	check(
		indoc! {r#"
			Point :: struct { x: int, y: int }
			Foo :: struct { n: int, s: string, f: float }
			User :: struct { name: string, age: int }
			p :: Point.{ x = 1, y = 2 }
			q :: Point.{3, 4}
			r :: Point.{ 2 4 }
			v :: Foo.{ n = 42, s = "hi", f = 1.5 }
			print(p.x, p.y, q.x, q.y, Point.{3}.y, r.1 == r.y, Point.{ 3, y = 9 }.y, User.{}.age, v.s, v.f)
			print(Point.{}, p)
		"#},
		["1 2 3 4 0 true 9 0 hi 1.5", "Point.{x = 0, y = 0} Point.{x = 1, y = 2}"],
	);
}

#[test]
fn literal_rejections() {
	fail(
		["Point :: struct { x: int, y: int }", "Point.{3, 4, 5}"],
		"has 2 fields but 3 values were provided",
	);
	fail(
		["Point :: struct { x: int, y: int }", "p :: Point.{ 3, x = 9 }"],
		"`x` was already set positionally",
	);
	fail(
		["Point :: struct { x: int, y: int }", "p : Point : .{ z = 1 }"],
		"no field `z`",
	);
	fail(["F :: struct { x: int }", "F.{ x = 1, x = 2 }"], "`x` is repeated");
	fail("p := .{ x = 1, x = 2 }", "`x` is repeated");
}

#[test]
fn mutation_and_copies() {
	check(
		indoc! {"
			Point :: struct { x: int, y: int }
			Bag :: struct { items: []int }
			p := Point.{ x = 10, y = 20 }
			p.y = 99
			b := p
			b.x = 5
			s :: Bag.{ items = [1, 2, 3] }
			items := s.items
			items << 4
			a := [1]
			bags :: [Bag.{ items = a }]
			a << 2
			print(p, b.x, s.items, bags[0].items)
		"},
		"Point.{x = 10, y = 99} 5 [1, 2, 3] [1]",
	);
	fail(
		["Point :: struct { x: int, y: int }", "p :: Point.{}", "p.x = 5"],
		"immutable",
	);
}

#[test]
fn in_fns() {
	check(
		indoc! {"
			Point :: struct { x: int, y: int }
			sum :: fn(p: Point) int { p.x + p.y }
			make :: fn() Point { .{ x = 1, y = 2 } }
			x :: 3
			y :: 4
			p : Point : .{ x = 2, y = 1 }
			q : Point : .{ x, y }
			print(sum(Point.{ x = 3, y = 4 }), sum(.{ x = 3, y = 4 }), sum(.{ x, y }), make(), p.x + p.y, q.y)
		"},
		"7 7 7 Point.{x = 1, y = 2} 3 4",
	);
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
fn if_no_else_struct_zero() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		p :: if false { Point.{ x = 1, y = 2 } }
		p.x
	"};
	check(src, "0");
}

#[test]
fn default_field_value() {
	check(
		indoc! {"
			User :: struct { age: int, name: string, swag: int = 5 }
			a : User : .{}
			print(User.{}.swag, User.{ age = 30 }.swag, User.{ swag = 99 }.swag, User.{}.age, a.swag)
		"},
		"5 5 99 0 5",
	);
}

#[test]
fn named_call_args() {
	check(
		indoc! {"
			Options :: struct { foo: int, bar: bool }
			User :: struct {}
			User :< {
				with_options :: fn(self, opt: Options) { print(opt.bar) }
			}
			f :: fn(o: Options) { print(o.foo) }
			g :: fn(x: int, o: Options) { print(x + o.foo) }
			f(bar = true, foo = 4)
			User.{}.with_options(bar = true, foo = 4)
			g(1, foo = 2)
		"},
		["4", "true", "3"],
	);
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
		Wallet :: struct { cash: Money }
		Money :: struct { amount: int }
		w := Wallet.{ cash = Money.{ amount = 5 } }
		print(w.cash.amount)
		print(w)
		w.cash = Money.{ amount = 9 }
		w.cash.amount
	"};
	check(src, ["5", "Wallet.{cash = Money.{amount = 5}}", "9"]);
}

#[test]
fn def_rejections() {
	fail("A :: struct { a: A }", "recurses for ever ever");
	fail(
		["A :: struct { b: B }", "B :: struct { a: A }"],
		"recurses for ever ever",
	);
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
		Point :: struct { x: int, y: int }
		u :: User.{ name = "abc", age = 23 }
		p :: Point.{ x = 1, y = 2 }
		q :: .{ ..p, y = 9 }
		print(u.is_registered, register(u).name, register(u).is_registered)
		print(Point.{ ..p, y = 9 }.y, q.x + q.y)
	"#};
	check(src, ["false abc true", "9 10"]);
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
			g :: fn() struct { x: int } { .{ x = 4 } }
			h :: fn(xs: []struct { x: int }) { print(xs[0].x + xs[1].x) }
			f(.{ x = 4 })
			print(g().x)
			h(.[ .{ x = 1 }, .{ x = 2 } ])
		"},
		["4", "4", "3"],
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
			f :: fn(p: struct { x: int, y: int }) { print(p.x + p.y) }
			pos := .{ x = 1, y = 2 }
			print(pos.x + pos.y)
			f(pos)
		"},
		["3", "3"],
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

#[test]
fn open_structs_gain_fields_from_modules() {
	Project::new()
		.file(
			"cfg.oi",
			["module cfg", "@open", "pub Config :: struct { name: string }"],
		)
		.file(
			"physics.oi",
			["module physics", "use cfg.{ Config }", "Config :< { dt: float = 0.5 }"],
		)
		.file(
			"game.oi",
			["module game", "use cfg.{ Config }", "Config :< { level: int = 1 }"],
		)
		.file(
			"main.oi",
			[
				"use cfg.{ Config }",
				"use physics",
				"use game",
				"c :: Config.{}",
				"print(c.dt, c.level)",
			],
		)
		.check("0.5 1");
}

#[test]
fn closed_structs_reject_fields() {
	fail("Context :< { x: int = 0 }", "`core::Context` can't gain fields");
}

#[test]
fn typed_literals() {
	check(
		indoc! {r#"
			Point :: struct { x: int, y: int }
			Q :: Point
			M :: struct (int, string)
			p : ?Point = .{ x = 1 }
			r : ^Point = &.{ x = 2 }
			f :: fn() !Point { .{ y = 3 } }
			print(p, r.x, f(), !Point.{ x = 4 }, Q.{}, M.{1, "a"}, M.{}, ?Point.{}, int.{})
		"#},
		r#"some.(Point.{x = 1, y = 0}) 2 ok.(Point.{x = 0, y = 3}) ok.(Point.{x = 4, y = 0}) Point.{x = 0, y = 0} M(1, "a") M(0, "") none 0"#,
	);
	fail("print(^int.{})", "a reference must be initialized");
}

#[test]
fn field_append() {
	let src = indoc! {"
		Bag :: struct { xs: []int = [] }
		Bag :< { push :: fn(mut self, x: int) { self.xs << x << x + 1 } }
		b := Bag.{}
		b.push(1)
		b.xs << 5
		b.xs
	"};
	check(src, "[1, 2, 5]");
	let src = indoc! {"
		Bag :: struct { xs: []int = [] }
		b :: Bag.{}
		b.xs << 1
	"};
	fail(src, "cannot append to immutable `b`");
}

#[test]
fn field_index_assign() {
	let src = indoc! {"
		Bag :: struct { xs: []int = [] }
		Bag :< {
			bump :: fn(mut self, i: int) {
				self.xs[i] += 10
			}
		}
		b := Bag.{ xs = [1, 2, 3] }
		old := b.xs
		b.bump(0)
		b.xs[2] = 7
		print(old)
		b.xs
	"};
	check(src, ["[1, 2, 3]", "[11, 2, 7]"]);
	let src = indoc! {"
		Bag :: struct { xs: []int = [] }
		b :: Bag.{}
		b.xs[0] = 1
	"};
	fail(src, "cannot assign to element of immutable `b`");
}
