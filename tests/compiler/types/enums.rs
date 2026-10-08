use crate::helpers::*;

#[test]
fn fieldless() {
	check(
		indoc! {"
			MyOption :: enum { none some }
			MyResult :: enum { ok err }
			Color :: enum { red green blue }
			Stat :: enum { health mana stamina }
			User :: struct { s: Stat }
			fav :: fn() Color { Color.blue }
			c := Color.red
			c = Color.blue
			d: Color
			print(MyOption.none, MyResult.ok, Color.red, c, d, Color.{}, fav(), User.{ s = Stat.mana }.s)
			print(Color.red == Color.red, Color.red == Color.blue, Color.red != Color.blue)
		"},
		["none ok red blue red red blue mana", "true false true"],
	);
}

#[test]
fn in_match() {
	check(
		indoc! {r#"
			Color :: enum { red green blue }
			c :: Color.green
			print(match c { Color.red => "r", Color.green => "g", else => "?" })
			match c {
				.red => "r",
				.green => "g",
				else => "?",
			}
		"#},
		["g", "g"],
	);
}

#[test]
fn shorthand() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			Stat :: enum { health mana stamina }
			User :: struct { s: Stat }
			Pen :: struct { ink: Color }
			c := Color.green
			c = .red
			b : Color : .blue
			p := Pen.{}
			p.ink = .blue
			cs := [Color.red]
			cs[0] = .green
			print(c, b, c == .red, c != .blue, User.{ s = .mana }.s, User.{ .stamina }.s, p.ink, cs[0])
		"},
		"red blue true true mana stamina blue green",
	);
}

#[test]
fn fieldless_rejections() {
	fail(["Color :: enum { red green blue }", "Color.{ red }"], "only supports");
	fail(
		["Color :: enum { red green blue }", "Color.purple"],
		"no variant `purple`",
	);
	fail(
		["Color :: enum { red green blue }", "c :: Color.red", "c == .purple"],
		"no variant `purple`",
	);
	fail(
		["Color :: enum { red green blue }", "c : Color : :purple"],
		"no variant `purple`",
	);
	fail(
		["Color :: enum { red green blue }", ".red"],
		"cannot infer the enum type",
	);
	fail(
		["Color :: enum { red green blue }", "Color.red.hex()"],
		"has no method `hex`",
	);
}

#[test]
fn disc_rejections() {
	fail("E :: enum { a = 2, b, c = 2 }", "discriminant value `2`");
	fail("E :: enum { a = 5, b, c = 6 }", "discriminant value `6`");
	fail("E : bool : enum { a }", "not an enum-able type");
	fail("E : u8 : enum { a = 300 }", "out of range for its backing type");
	fail("E : u8 : enum { a some(int) }", "cannot have payload");
}

#[test]
fn const_expr_disc() {
	check(
		[
			"SHIFT :: 2",
			"Perm :: enum { ayy = 0 << SHIFT, bee = 1 << SHIFT, cee = (1 << SHIFT) | 1 }",
			r#""{ord(Perm.bee)} {ord(Perm.cee)}""#,
		],
		"4 5",
	);
}

#[test]
fn discriminants() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			Status :: enum { ok = 200, err = 500 }
			Backed : u8 : enum { ok = 200, err = 250 }
			E :: enum { a = 5, b c }
			Opt :: enum { nope some(int) }
			x: E
			print(int.(Color.blue), int.(Status.err), u8.(Backed.ok), x, ord(Color.blue), ord(Opt.some.(1)), Color.blue.str())
		"},
		"2 500 200 a 2 1 blue",
	);
}

#[test]
fn payload_basics() {
	check(
		indoc! {"
			Shape :: enum { point triangle(f64, f64, f64) }
			Opt :: enum { nope some(int) }
			o : Opt : .nope
			d: Opt
			print(Shape.triangle.(3.0, 4.0, 5.0), Opt.nope, o, d, Opt.{})
		"},
		"triangle.(3.0, 4.0, 5.0) nope nope nope nope",
	);
}

#[test]
fn payload_rejections() {
	fail(
		["Opt :: enum { nope some(int) }", "int.(Opt.some.(1))"],
		"no backing value",
	);
	fail(
		["Opt :: enum { nope some(int) }", "Opt.some.(3.0)"],
		"expected int, got float",
	);
	fail(
		["Opt :: enum { nope some(int) }", "Opt.some.()"],
		"takes 1 field(s), got 0",
	);
	fail(
		["Opt :: enum { nope some(int) }", "Opt.some.(1) < Opt.some.(2)"],
		"claim `Ord` for `Opt` to define ordering",
	);
	fail("A :: enum { wrap(NoSuchType) }", "unknown type");
}

#[test]
fn payload_match() {
	check(
		indoc! {"
			Opt :: enum { nope some(int) }
			Shape :: enum { rect(int, int) tri(int, int, int) }
			get :: fn(o: Opt) int {
				match o {
					.some.(n) => n,
					.nope => -1,
				}
			}
			s :: Shape.rect.(3, 4)
			o : Opt : .some.(5)
			print(get(Opt.some.(7)), get(.nope), get(o))
			match s {
				.rect.(w, h) => w * h,
				.tri.(a, b, c) => a + b + c,
			}
		"},
		["7 -1 5", "12"],
	);
}

#[test]
fn payload_eq() {
	check(
		indoc! {r#"
			Opt :: enum { nope some(int) }
			Msg :: enum { quit say(string) }
			print(Opt.some.(1) == Opt.some.(1), Opt.some.(1) == Opt.some.(2), Opt.nope == Opt.some.(1), Opt.nope != Opt.some.(1))
			print(Msg.say.("hi") == Msg.say.("hi"), Msg.say.("hi") == Msg.say.("bye"))
		"#},
		["true false false true", "true false"],
	);
}

#[test]
fn struct_payload() {
	check(
		indoc! {r#"
			Point :: struct { x: int, y: int }
			Shape :: enum { dot rect(Point) }
			s :: Shape.rect.(Point.{ x = 3, y = 4 })
			match s {
				.rect.(p) => print(p),
				.dot => {}
			}
		"#},
		"Point.{x = 3, y = 4}",
	);
}

#[test]
fn enum_payload() {
	check(
		indoc! {r#"
			A :: enum { one two }
			B :: enum { wrap(A) empty }
			b :: B.wrap.(A.two)
			match b {
				.wrap.(a) => match a {
					.one => "one",
					.two => "two",
				},
				.empty => "none",
			}
		"#},
		"two",
	);
}

#[test]
fn struct_form_construct_and_match() {
	check(
		indoc! {r#"
			Shape :: enum {
				circle { radius: f64 }
				rectangle { width: f64, height: f64 }
				triangle(f64, f64, f64)
				point
			}
			s :: Shape.circle.{ radius = 5.0 }
			match s {
				.circle.{ radius } => radius * 2.0,
				.rectangle.{ width, height } => width * height,
				.triangle.(a, b, c) => a + b + c,
				.point => 0.0,
			}
		"#},
		"10.0",
	);
}

#[test]
fn struct_form_shorthand_and_rename() {
	check(
		indoc! {r#"
			Shape :: enum { circle { radius: f64 } rectangle { width: f64, height: f64 } }
			mk :: fn() Shape { .rectangle.{ width = 3.0, height = 4.0 } }
			match mk() {
				.rectangle.{ width = w, height } => w * height,
				else => 0.0,
			}
		"#},
		"12.0",
	);
}

#[test]
fn struct_form_defaults() {
	check(
		indoc! {"
			Shape :: enum { circle { radius: f64 } rectangle { width: f64, height: f64 } }
			S :: enum { rect { w: f64, h: f64 } }
			a :: match Shape.{} { .circle.{ radius } => radius, else => -1.0 }
			b :: match S.rect.{ h = 2.0 } { .rect.{ w, h } => w + h }
			print(a, b, Shape.circle.{ 1.0 })
		"},
		"0.0 2.0 circle.{radius = 1.0}",
	);
}

#[test]
fn struct_form_rejections() {
	fail(
		["S :: enum { circle { radius: f64 } }", "S.circle.{ r = 1.0 }"],
		"no field `r`",
	);
	fail(
		["S :: enum { tri(f64, f64) }", "S.tri.{ a = 1.0 }"],
		"takes positional fields",
	);
}

#[test]
fn alias_payload() {
	check(
		indoc! {"
			Meters :: f64
			Dist :: enum { unknown known(Meters) }
			d :: Dist.known.(5.0)
			match d {
				.known.(m) => m,
				.unknown => 0.0,
			}
		"},
		"5.0",
	);
}

#[test]
fn atom_coerces() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			Stat :: enum { health mana stamina }
			User :: struct { s: Stat }
			name :: fn(c: Color) string { c.str() }
			b : Color : :blue
			c := Color.green
			c = :red
			print(b, c, c == :red, Color.blue == :blue, User.{ s = :mana }.s, name(:blue))
		"},
		"blue red true true mana blue",
	);
}

#[test]
fn from() {
	check(
		indoc! {r#"
			Color :: enum { red green blue }
			Shape :: enum { point triangle(f64, f64, f64) }
			print(Color.from(1) or { Color.red }, Color.from(9) or .red, Color.from("blue") or { Color.red }, Color.from(:blue) or { Color.red })
			print(Shape.from(1) or { Shape.point })
			Color.from(9) or { print($) Color.red }
			Color.from("purple") or { print($) Color.red }
			Color.from(:purple) or { print($) Color.red }
		"#},
		[
			"green red blue blue",
			"triangle.(0.0, 0.0, 0.0)",
			"no matching variant",
			"no matching variant",
			"no matching variant",
			"red",
		],
	);
	fail(
		["Color :: enum { red green blue }", "Color.from(true)"],
		"needs an int, string, or atom",
	);
}

#[test]
fn shorthand_coerces_through_branches() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			name :: fn(c: Color) string { c.str() }
			tail_if :: fn(pick: bool) Color {
				if pick { .blue } else { .red }
			}
			tail_match :: fn(n: int) Color {
				match n {
					1 => .red,
					else => .blue,
				}
			}
			n :: 9
			a : Color : if false { .red } else { .blue }
			b : Color : match n {
				1 => .red,
				else => .blue,
			}
			print(name(.blue), tail_if(false), tail_match(9), a, b)
		"},
		"blue red blue blue blue",
	);
}

#[test]
fn print_payloads() {
	check(
		indoc! {"
			Shape :: enum {
				point
				circle { radius: f64 }
				triangle(f64, f64, f64)
			}
			print(Shape.point)
			print(Shape.circle.{ radius = 5.0 })
			print(Shape.triangle.(3.0, 4.0, 5.0))
		"},
		["point", "circle.{radius = 5.0}", "triangle.(3.0, 4.0, 5.0)"],
	);
}

#[test]
fn backed_arrays_pack() {
	check(
		indoc! {"
			Status : u8 : enum { ok = 200, err = 250 }
			a := [Status.ok, Status.err]
			a[0] = Status.err
			a << Status.ok
			print(a)
			loop s in a { print(s) }
			print(Status.err in a)
			print(match a { [x, y, z] => z, else => Status.err })
			f: [3]Status
			f[1] = Status.err
			print(f[1])
			f[0] == Status.ok
		"},
		["[err, err, ok]", "err", "err", "ok", "true", "ok", "err", "true"],
	);
}

#[test]
fn string_backed_raws() {
	check(
		indoc! {r#"
			Suit : string : enum { hearts = "♥" spades = "♠" }
			print(string.(Suit.spades))
			print(string.(Suit.hearts))
			print(Suit.spades.str())
			print(ord(Suit.spades))
			print(Suit.hearts == Suit.hearts)
			a :: [Suit.spades, Suit.hearts]
			print(string.(a[1]))
			match Suit.spades { .spades => "s", else => "?" }
		"#},
		["♠", "♥", "spades", "1", "true", "♥", "s"],
	);
	check(["S : string : enum { a b }", "string.(S.b)"], "b");
}

#[test]
fn string_backed_errors() {
	fail(["S : string : enum { a b }", "int.(S.a)"], "cannot cast string");
	fail(r#"S :: enum { a = "x" }"#, "needs a string backing");
	fail(r#"S : string : enum { a = 2 }"#, "uses raw values");
	fail(r#"S : string : enum { a = "x" b = "x" }"#, "assigned more than once");
	fail(r#"S : string : enum { a b = "a" }"#, "assigned more than once");
}

#[test]
fn backed_array_signed_sextends() {
	check(
		indoc! {r#"
			Delta : i8 : enum { down = -3, up = 4 }
			a :: [Delta.up, Delta.down]
			d :: a[1]
			print(int.(d))
			print(match d { Delta.down => "yes", else => "no" })
			d == Delta.down
		"#},
		["-3", "yes", "true"],
	);
}

#[test]
fn fills() {
	check(
		indoc! {r##"
			Color :: enum { red green blue }
			Color :< {
				DEFAULT :: Color.green
				hex :: fn(self) string {
					match self {
						.red => "#f00",
						.green => "#0f0",
						.blue => "#00f",
					}
				}
				is_warm :: fn(self) bool { self == .red }
				primary :: fn() Color { .red }
			}
			print(Color.DEFAULT, Color.green.hex(), Color.red.is_warm(), Color.blue.is_warm(), Color.primary(), Color.green)
		"##},
		"green #0f0 true false red green",
	);
}

#[test]
fn method_on_payload_enum() {
	check(
		indoc! {r#"
			Shape :: enum { point triangle(f64, f64, f64) }
			Shape :< {
				perimeter :: fn(self) f64 {
					match self {
						.triangle.(a, b, c) => a + b + c,
						.point => 0.0,
					}
				}
			}
			Shape.triangle.(2.0, 3.0, 4.0).perimeter()
		"#},
		"9.0",
	);
}

#[test]
fn str_fill_overrides_derived() {
	check(
		indoc! {r#"
			E :: enum { a b }
			E :< { str :: fn(self) string { "custom" } }
			print(E.a.str(), E.b)
		"#},
		"custom custom",
	);
}

#[test]
fn variant_holes_from_a_macro() {
	check(
		indoc! {r"
			def! :: fn() Ast {
				vs := [`A`, `B(int)`]
				`E :: enum { %{..vs} }`
			}
			def!()
			print(E.A)
			print(match E.B.(7) { .A => 0, .B.(n) => n })
		"},
		["A", "7"],
	);
}
