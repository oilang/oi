use crate::helpers::*;

const ANIMAL_DOG: &str = indoc! {r#"
	Animal :: trait { speak : fn(self) string }
	Dog :: struct {}
	Dog : Animal < { speak :: fn(self) string { "woof" } }
"#};

const ANIMAL_DOG_KIND: &str = indoc! {r#"
	Animal :: trait { speak : fn(self) string }
	Dog :: struct { kind: string }
	Dog : Animal < { speak :: fn(self) string { "woof" } }
"#};

const ANIMAL_KIND: &str = indoc! {r#"
	Animal :: trait {
		kind: string
		speak : fn(self) string
	}
	Dog :: struct { kind: string }
	Dog : Animal < { speak :: fn(self) string { "woof" } }
"#};

#[test]
fn marker_impl() {
	let src = indoc! {r#"
		Marker :: trait {}
		Animal :: trait { kind: string }
		Dog :: struct { kind: string }
		Dog :< Marker
		Dog :< Animal
		Dog.{ "Collie" }.kind
	"#};
	check(src, "Collie");
}

#[test]
fn supertraits() {
	let src = indoc! {"
		Foo :: trait {}
		Baz : Foo, Bar : trait {}
	"};
	check(src, "");

	let src = indoc! {"
		Eq :: trait {}
		Ord : Eq : trait {}
		X :: struct {}
		X :< Eq
		X :< Ord
		print(X is Ord)
	"};
	check(src, "true");

	fail(
		indoc! {"
			Eq :: trait {}
			Ord :: trait is Eq {}
			X :: struct {}
			X :< Ord
		"},
		"",
	);
}

#[test]
fn defaults_claim_supertraits() {
	let src = indoc! {r#"
		Show :: trait { show: fn(self) string }
		Named : Show : trait {
			name: fn(self) string
			show :: fn(self) string { "<{self.name()}>" }
		}
		Dog :: struct {}
		Dog : Named < { name :: fn(self) string { "dog" } }
		print(Dog is Show, Dog.{}.show())
	"#};
	check(src, "true <dog>");
	fail(&src.replace("show ::", "loud ::"), "must also implement `Show`");
}

#[test]
fn default_methods() {
	let src = indoc! {r#"
		Animal :: trait {
			speak : fn(self) string
			shout :: fn(self) string { self.speak() + "!" }
		}
		Dog :: struct {}
		Dog : Animal < { speak :: fn(self) string { "woof" } }
		Dog.{}.shout()
	"#};
	check(src, "woof!");
	let src = indoc! {r#"
		Animal :: trait {
			speak : fn(self) string
			shout :: fn(self) string { self.speak() + "!" }
		}
		Dog :: struct {}
		Dog : Animal < {
			speak :: fn(self) string { "woof" }
			shout :: fn(self) string { "WOOF" }
		}
		Dog.{}.shout()
	"#};
	check(src, "WOOF");
}

#[test]
fn default_methods_name_trait_params() {
	let src = indoc! {r#"
		Source[T] :: trait {
			next: fn(mut self) ?T
			first :: fn(mut self) ?T { got: ?T = self.next()  got }
		}
		Count :: struct { n: int }
		Count : Source[int] < { next :: fn(mut self) ?int { self.n += 1  self.n } }
		Wrap[E] :: struct { item: E }
		Wrap[E] : Source[E] < { next :: fn(mut self) ?E { self.item } }
		c := Count.{ n = 0 }
		w := Wrap[string].{ item = "hi" }
		print(c.first() or 0, w.first() or "")
	"#};
	check(src, "1 hi");
}

#[test]
fn empty_default_method() {
	let src = indoc! {r#"
		Counter :: trait {
			bump: fn(mut self) int
			reset :: fn(mut self) {}
		}
		Tally :: struct { n: int }
		Tally : Counter < { bump :: fn(mut self) int { self.n += 1; self.n } }
		t := Tally.{ 5 }
		t.reset()
		t.bump()
	"#};
	check(src, "6");
}

#[test]
fn is_expression() {
	let src = indoc! {"
		Animal :: trait {}
		Dog :: struct {}
		Cat :: struct {}
		Dog :< Animal
		D :: Dog
		print(Dog is Animal, Dog is not Animal, D is Animal)
		print(Cat is Animal, Cat is not Animal)
	"};
	check(src, ["true false true", "false true"]);
}

#[test]
fn is_on_type_of() {
	let src = indoc! {"
		x := 3.7
		print(type(x) is Float, type(3) is Float, int is Float, type(x) is not Float)
		print(float == type(x), int == type(x))
	"};
	check(src, ["true false false false", "true false"]);
}

#[test]
fn implicit_trait() {
	let src = indoc! {"
		@implicit
		Fruit :: trait { seeds: bool }
		Kiwi :: struct { seeds := true }
		Bike :: struct { wheels := 2 }
		print(Kiwi is Fruit)
		print(Bike is Fruit)
	"};
	check(src, ["true", "false"]);
}

#[test]
fn every_type_is_display() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Color :: enum { red, green }
		show[T: Display] :: fn(v: T) string { v.str() }
		print(show(Point.{1, 2}), show(Color.red), show(3))
		print(Point is Display, Color is Display, int is Display)
	"};
	check(src, ["Point.{x = 1, y = 2} red 3", "true true true"]);
}

#[test]
fn display_objects_render_their_own_way() {
	let src = indoc! {r#"
		Point :: struct { x: int }
		Name :: struct { n: string }
		Name :< { str :: fn(self) string { "name {self.n}" } }
		Tag :: struct { t: int }
		Tag : Display < { str :: fn(self) string { "tag {self.t}" } }
		loop x in Display.[Point.{1}, Name.{"a"}, Tag.{7}] { print(x) }
		t :: Tag.{2}
		print(t, "{t}", t.str())
	"#};
	check(src, ["Point.{x = 1}", "name a", "tag 7", "tag 2 tag 2 tag 2"]);
}

#[test]
fn fmt_overrides_display() {
	let src = indoc! {r#"
		Hex :: struct { n: int }
		Hex :< { fmt :: fn(self; mut w: Write) { w.write("0x{self.n}") } }
		h :: Hex.{255}
		print(h, "{h}", h.str())
		loop x in Display.[h] { print(x) }
	"#};
	check(src, ["0x255 0x255 0x255", "0x255"]);
}

#[test]
fn every_type_is_debug() {
	let src = indoc! {r#"
		Point :: struct { x: int, s: string }
		Hex :: struct { n: int }
		Hex :< { debug :: fn(self; mut w: Write) { w.write("Hex({self.n})") } }
		print("hi".repr(), 3.repr(), Point.{1, "a"}.repr(), Hex.{255}.repr())
	"#};
	check(src, r#""hi" 3 Point.{x = 1, s = "a"} Hex(255)"#);
}

#[test]
fn generic_fmt_into_one_buffer() {
	let src = indoc! {r#"
		Point :: struct { x: int }
		Name :: struct { n: string }
		Name :< { str :: fn(self) string { "name {self.n}" } }
		put[T: Display] :: fn(x: T, mut b: Buffer) { x.fmt(mut b) }
		b := Buffer.{}
		put(Point.{1}, mut b)
		put(3, mut b)
		put(Name.{"a"}, mut b)
		print(b)
	"#};
	check(src, "Point.{x = 1}3name a");
}

#[test]
fn is_on_a_trait_object() {
	let src = indoc! {r#"
		Animal :: trait {}
		Dog :: struct {}
		Dog :< Animal
		Cat :: struct {}
		Cat :< Animal
		Bad :: struct {}
		Bad : Error < { message :: fn(self) string { "bad" } }
		a := Animal.(Dog.{})
		e := Error.(Bad.{})
		print(a is Dog, a is Cat, a is not Dog, e is Bad)
	"#};
	check(src, "true false false true");
}

#[test]
fn rejects_bad_claims() {
	fail(
		indoc! {r#"
			Dog :: struct {}
			Dog : Animal < { speak :: fn(self) string { "woof" } }
		"#},
		"",
	);
	fail(
		indoc! {r#"
			Animal :: trait { speak: fn(self) string }
			Dog :: struct {}
			Dog : Animal < {}
		"#},
		"",
	);
	fail(
		indoc! {r#"
			Animal :: trait { kind: string }
			Dog :: struct {}
			Dog :< Animal
		"#},
		"",
	);
	fail(
		indoc! {"
			Animal :: trait { speak :: fn(self) string }
			Animal :: trait { bark :: fn(self) string }
		"},
		"",
	);
	fail(["Animal :: trait {}", "print(Ghost is Animal)"], "");
	fail([ANIMAL_DOG, "Rock :: struct {}", "a : Animal : Rock.{}"], "");
	fail(
		indoc! {r#"
			Animal :: trait { speak: fn(self) string }
			Dog :: struct {}
			Dog : Animal < { speak :: fn(self; loud: bool) string { "woof" } }
		"#},
		"wrong signature",
	);
	fail(
		indoc! {"
			A :: trait { f: fn(self) int }
			B :: trait { f: fn(self) string }
			S :: struct {}
			S : A, B < { f :: fn(self) int { 9 } }
		"},
		"declares",
	);
	fail(
		indoc! {"
			A :: trait { f :: fn(self) int { 1 } }
			B :: trait { f :: fn(self) int { 2 } }
			S :: struct {}
			S :< A, B
		"},
		"takes default `f` from both",
	);
	fail(
		indoc! {"
			S :: struct { f :: fn(self) int { 1 } }
			S :< { f :: fn(self) int { 2 } }
		"},
		"duplicate fill",
	);
	fail(
		indoc! {"
			Dbl :: trait { dbl: fn(n: int) int }
			Foo :: struct {}
			Foo : Dbl < { nope :: { 1 } }
		"},
		"no trait method `nope` supplies a signature",
	);
}

#[test]
fn allows_extra_helper_fills() {
	let src = indoc! {r#"
		Animal :: trait { speak: fn(self) string }
		Dog :: struct {}
		Dog : Animal < {
			speak :: fn(self) string { self.noise() }
			noise :: fn(self) string { "woof" }
		}
		Dog.{}.speak()
	"#};
	check(src, "woof");
}

#[test]
fn fills_satisfy_traits() {
	let src = indoc! {"
		A :: trait { f: fn(self) int }
		B :: trait { f: fn(self) int }
		C :: trait { f :: fn(self) int { 1 } }
		D :: trait { f :: fn(self) int { 2 } }
		S :: struct {}
		S : A, B < { f :: fn(self) int { 9 } }
		T :: struct { f :: fn(self) int { 3 } }
		T :< C, D
		U :: struct { f :: fn(self) int { 7 } }
		U :< A
		print(S.{}.f(), T.{}.f(), U.{}.f())
	"};
	check(src, "9 3 7");
}

#[test]
fn dyn_dispatch_zoo() {
	let src = indoc! {r#"
		Cat :: struct { legs: int, kind: string }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		zoo : []Animal : [ Dog.{ "collie" }, Cat.{ 4, "mau" } ]
		loop a in zoo { print("a " + a.kind + " says " + a.speak()) }
	"#};
	check([ANIMAL_KIND, src], ["a collie says woof", "a mau says meow"]);
}

const CAR_HORN: &str = indoc! {r#"
	Horn :: struct { kind: string }
	Horn : Animal < { speak :: fn(self) string { "honk" } }
	Car :: struct { Horn }
"#};

#[test]
fn via_delegation() {
	// via through embedded struct
	let src = indoc! {r#"
		Car :< Animal via Horn
		zoo : []Animal : [ Car.{ Horn = Horn.{ kind = "vroom" } } ]
		loop a in zoo { print("a {a.kind} says {a.speak()}") }
	"#};
	check([ANIMAL_KIND, CAR_HORN, src], "a vroom says honk");

	let over = r#"Car : Animal via Horn < { speak :: fn(self) string { "HONK HONK" } }
	Car.{ kind = "vroom" }.speak()"#;
	check([ANIMAL_KIND, CAR_HORN, over], "HONK HONK");

	// via through named field
	let named = indoc! {r#"
		Wagon :: struct { horn: Horn, kind: string }
		Wagon :< Animal via horn
		print(Wagon.{ Horn.{ "vroom" }, "wagon" }.speak())
	"#};
	check([ANIMAL_KIND, CAR_HORN, named], "honk");
}

#[test]
fn embedded_field_satisfies_requirement() {
	let src = indoc! {r#"
		Meta :: struct { kind: string, id: int }
		Enemy :: struct { Meta; hp: int }
		Enemy : Animal < { speak :: fn(self) string { "rawr" } }
		zoo : []Animal : [ Enemy.{ Meta = Meta.{ kind = "goblin" } } ]
		loop a in zoo { print("a {a.kind} says {a.speak()}") }
	"#};
	check([ANIMAL_KIND, src], "a goblin says rawr");
}

#[test]
fn trait_object_array_literal() {
	let src = indoc! {r#"
		Cat :: struct { kind: string }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		animals :: Animal.[ Dog.{ "collie" }, Cat.{ "mau" } ]
		loop a in animals { print("{a.kind}: {a.speak()}") }
		print(animals)
	"#};
	check(
		[ANIMAL_KIND, src],
		[
			"collie: woof",
			"mau: meow",
			r#"[Dog.{kind = "collie"}, Cat.{kind = "mau"}]"#,
		],
	);
}

#[test]
fn dyn_trait_param_and_default() {
	let src = indoc! {r#"
		Animal :: trait {
			speak : fn(self) string
			shout :: fn(self) string { self.speak() + "!" }
		}
		Dog :: struct {}
		Cat :: struct {}
		Dog : Animal < { speak :: fn(self) string { "woof" } }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		greet :: fn(a: Animal) string { a.shout() }
		relay :: fn(a: Animal) string { greet(a) }
		print(greet(Dog.{}))
		print(relay(Cat.{}))
	"#};
	check(src, ["woof!", "meow!"]);
}

#[test]
fn mut_trait_objects_write_through() {
	let src = indoc! {r#"
		Sink :: trait { put: fn(mut self; s: string) }
		Log :: struct { lines: []string }
		Log : Sink < { put :: fn(mut self; s: string) do self.lines << s }
		Wrap :: struct { log: Log }
		put :: fn(mut s: Sink) do s.put("a")
		w := Wrap.{Log.{[]}}
		put(mut w.log)
		print(w.log.lines)
	"#};
	check(src, r#"["a"]"#);
	assert_clean(src);
}

#[test]
fn generic_trait_objects() {
	let src = indoc! {r#"
		Conv[T] :: trait { conv: fn(self) T }
		N :: struct { n: int }
		N : Conv[string] < { conv :: fn(self) string do "n{self.n}" }
		N : Conv[int] < { conv :: fn(self) int do self.n * 2 }
		s :: fn(c: Conv[string]) string do c.conv()
		i :: fn(c: Conv[int]) int do c.conv()
		put :: fn(mut w: Write) do w.write("hi")
		b := Buffer.{}
		put(mut b)
		print(s(N.{3}), i(N.{4}), b.str())
	"#};
	check(src, "n3 8 hi");

	fail("f :: fn(c: Contains) {}", "`Contains` needs type arguments");
}

#[test]
fn scalar_trait_objects() {
	let src = indoc! {r#"
		Sink :: trait { write: fn(mut self; s: string) }
		int : Sink < { write :: fn(mut self; s: string) do self += s.len() }
		Shape :: trait { area: fn(self) float }
		float : Shape < { area :: fn(self) float do self * self }
		put :: fn(mut w: Sink) do w.write("hello")
		n := 1
		put(mut n)
		x : Shape = 3.0
		print(n, x.area(), x)
	"#};
	check(src, "6 9.0 3.0");
	let twice = [
		"Twice :: trait { twice: fn(self) Self }",
		"int : Twice < { twice :: fn(self) Self do self }",
		"x : Twice = 1",
	];
	fail(twice, "can't be a `Twice` object yet");
}

#[test]
fn trait_typed_struct_field() {
	let src = indoc! {r#"
		Pen :: struct { pet: Animal }
		p :: Pen.{ pet = Dog.{} }
		print(p.pet.speak())
	"#};
	check([ANIMAL_DOG, src], "woof");
}

#[test]
fn self_return_reboxes_through_object() {
	let src = indoc! {r#"
		Pet :: trait {
			name : fn(self) string
			eq : fn(self, other: Self) bool
			clone : fn(self) Self
		}
		Dog :: struct { n: string }
		Dog : Pet < {
			name :: fn(self) string { self.n }
			eq :: fn(self, other: Self) bool { self.n == other.n }
			clone :: fn(self) Self { Dog.{ "{self.n}2" } }
		}
		p : Pet : Dog.{ "rex" }
		print(p.clone().name(), Dog.{ "a" }.clone().name())
	"#};
	check(src, "rex2 a2");
}

#[test]
fn option_self_return_reboxes_through_object() {
	let src = indoc! {r#"
		Pet :: trait {
			name : fn(self) string
			next : fn(self) ?Self
		}
		Dog :: struct { n: string }
		Dog : Pet < {
			name :: fn(self) string { self.n }
			next :: fn(self) ?Self { if self.n == "rex" { Dog.{ "max" } } else { none } }
		}
		p : Pet : Dog.{ "rex" }
		match p.next() {
			.some.(q) => print(q.name(), (q.next() or Dog.{ "end" }).name()),
			else => print("none"),
		}
	"#};
	check(src, "max end");
}

#[test]
fn rejects_self_param_call_through_object() {
	fail(
		indoc! {r#"
			Pet :: trait { eq : fn(self, other: Self) bool }
			Dog :: struct {}
			Dog : Pet < { eq :: fn(self, other: Self) bool { true } }
			p : Pet : Dog.{}
			print(p.eq(p))
		"#},
		"cannot call `eq` through a `Pet` object",
	);
}

#[test]
fn trait_object_renders_concrete_struct() {
	let src = indoc! {r#"
		a : Animal : Dog.{ "collie" }
		print(a)
		print("says {a}")
		print(a.str())
	"#};
	check(
		[ANIMAL_DOG_KIND, src],
		[
			r#"Dog.{kind = "collie"}"#,
			r#"says Dog.{kind = "collie"}"#,
			r#"Dog.{kind = "collie"}"#,
		],
	);
}

#[test]
fn trait_object_uses_str_override() {
	let src = indoc! {r#"
		Dog :< { str :: fn(self) string { "a " + self.kind + " dog" } }
		a : Animal : Dog.{ "collie" }
		print(a)
	"#};
	check([ANIMAL_DOG_KIND, src], "a collie dog");
}

#[test]
fn trait_declared_str_dyn_dispatches() {
	let src = indoc! {r#"
		Animal :: trait { str : fn(self) string }
		Dog :: struct { kind: string }
		Dog : Animal < { str :: fn(self) string { "custom-" + self.kind } }
		a : Animal : Dog.{ "collie" }
		print(a)
		print(a.str())
	"#};
	check(src, ["custom-collie", "custom-collie"]);
}

#[test]
fn append_boxes_struct_literal_into_trait_array() {
	let src = indoc! {r#"
		Cat :: struct {}
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		zoo: []Animal = [ Dog.{} ]
		zoo << Cat.{}
		loop a in zoo { print(a.speak()) }
	"#};
	check([ANIMAL_DOG, src], ["woof", "meow"]);
}

#[test]
fn fill_headers_come_from_the_trait() {
	let src = indoc! {"
		Dbl :: trait { dbl: fn(n: int) int }
		Foo :: struct {}
		Bar :: struct {}
		Foo : Dbl < { dbl :: fn(i) int { i * 2 } }
		Bar : Dbl < { dbl :: { $ * 2 } }
		print(Foo.dbl(3))
		print(Bar.dbl(4))
	"};
	check(src, ["6", "8"]);
}

#[test]
fn branches_box_into_the_expected_trait_object() {
	let src = indoc! {"
		Shape :: trait { area: fn(self) int }
		Sq :: struct { s: int }
		Sq : Shape < { area :: { self.s * self.s } }
		Ci :: struct { r: int }
		Ci : Shape < { area :: { 3 * self.r * self.r } }
		pick :: fn(big: bool) Shape { if big { Ci.{2} } else { Sq.{2} } }
		print(pick(true).area(), pick(false).area())
	"};
	check(src, "12 4");
}

#[test]
fn fill_and_default_satisfy_trait_fields() {
	let src = indoc! {r#"
		Animal :: trait {
			name: string
			greeting: string = "hi"
			greet :: fn(self) string { "{self.name} says {self.greeting}" }
		}
		Rock :: struct {}
		Rock : Animal < { name :: "Bob" }
		Rock.{}.greet()
	"#};
	check(src, "Bob says hi");
}

#[test]
fn dyn_field_from_const() {
	let src = indoc! {r#"
		Animal :: trait {
			name: string
			legs: int = 4
			depth: int = -7
			speak : fn(self) string
		}
		Cat :: struct { name: string }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		Rock :: struct {}
		Rock : Animal < {
			name :: "Bob"
			speak :: fn(self) string { "..." }
		}
		zoo : []Animal : [ Cat.{ "Tom" }, Rock.{} ]
		loop a in zoo { print("{a.name} {a.legs} {a.depth} {a.speak()}") }
	"#};
	check(src, ["Tom 4 -7 meow", "Bob 4 -7 ..."]);
}

#[test]
fn int_fill_of_float_field() {
	let src = indoc! {"
		Deep :: trait {
			depth: float
			get :: fn(self) float { self.depth }
		}
		Rock :: struct {}
		Rock : Deep < { depth :: 3 }
		d : Deep : Rock.{}
		print(Rock.{}.get())
		print(d.depth)
	"};
	check(src, ["3.0", "3.0"]);
}

#[test]
fn atom_fill_of_enum_field() {
	let src = indoc! {"
		Size :: enum { small, big }
		Sized :: trait { size: Size = :small }
		Rock :: struct {}
		Rock : Sized < { size :: :big }
		Pebble :: struct {}
		Pebble :< Sized
		zoo : []Sized : [Rock.{}, Pebble.{}]
		loop z in zoo { print(z.size) }
		print(Rock.size == .big)
	"};
	check(src, ["big", "small", "true"]);
	fail(
		"Shape :: enum { dot, circle(float) }\nT :: trait { s: Shape }\nR :: struct {}\nR : T < { s :: :dot }",
		"must be a `Shape` literal",
	);
}

#[test]
fn rejects_wrong_type_fill() {
	fail(
		indoc! {"
			Animal :: trait { name: string }
			Rock :: struct {}
			Rock : Animal < { name :: 5 }
		"},
		"must be a `string` literal to satisfy trait",
	);
	fail(
		indoc! {r#"
			Animal :: trait { greeting: string = "hi" }
			Rock :: struct { greeting: int }
			Rock :< Animal
		"#},
		"missing field",
	);
}

#[test]
fn a_trait_takes_type_arguments() {
	let src = indoc! {r#"
		Money :: struct { cents: int }
		Money : From[int] < { from :: fn(n: int) Self { Money.{ n * 100 } } }
		Money : From[string] < { from :: fn(s: string) Self { Money.{ int.try_from(s) or 0 } } }
		print(Money.from(5).cents)
		print(Money.from("7").cents)
	"#};
	check(src, ["500", "7"]);
}

#[test]
fn a_literal_coerces_through_from() {
	let src = indoc! {r#"
		Money :: struct { cents: int }
		Money : From[int] < { from :: fn(n: int) Self { Money.{ n * 100 } } }
		Money : From[string] < { from :: fn(s: string) Self { Money.{ int.try_from(s) or 0 } } }
		cents :: fn(m: Money) int { m.cents }
		Glyph :: struct { code: rune }
		Glyph : From[rune] < { from :: fn(r: rune) Self { Glyph.{ r } } }
		Flag :: struct { on: bool }
		Flag : From[bool] < { from :: fn(b: bool) Self { Flag.{ b } } }
		m: Money = 5
		g: Glyph = 'a'
		f: Flag = true
		Money : From[[]int] < { from :: fn(xs: []int) Self { Money.{ xs.len() } } }
		b: Money = .[3]
		P :: struct { n: int }
		Money : From[P] < { from :: fn(p: P) Self { Money.{ p.n } } }
		r: Money = .{ n = 2 }
		print(m.cents, cents("7"), g.code, f.on, b.cents, r.cents)
	"#};
	check(src, "500 7 a true 1 2");
}

#[test]
fn a_type_argument_fills_a_field_slot() {
	let src = indoc! {"
		Wrap[T] :: trait {
			value: T
			get : fn(self) T
		}
		Box :: struct { value: int }
		Box : Wrap[int] < { get :: fn(self) int { self.value } }
		print(Box.{ 7 }.get())
	"};
	check(src, "7");
	fail(
		indoc! {r#"
			Wrap[T] :: trait { value: T }
			Box :: struct { value: string }
			Box : Wrap[int] < {}
			print(Box.{ "x" }.value)
		"#},
		"missing field `value int`",
	);
}

#[test]
fn boxed_error_outlives_its_scope() {
	let src = indoc! {r#"
		Bad :: struct { line: int }
		Bad : Error < { message :: fn(self) string { "at " + self.line.str() } }
		parse :: fn() !int { return Bad.{ line = 4 } }
		print(parse() or { print($.message()); 0 })
	"#};
	check(src, ["at 4", "0"]);
}

#[test]
fn is_unwraps_trait_objects() {
	let src = indoc! {r#"
		Bad :: struct { line: int }
		Bad : Error < { message :: fn(self) string { "bad" } }
		Net :: enum { timeout refused }
		Net : Error < { message :: fn(self) string { "net" } }
		load :: fn(n: int) !int {
			if n == 0 { return Bad.{ line = 4 } }
			return Net.refused
		}
		print(load(0) or { if $ is Bad { $.line } else { -1 } })
		print(load(1) or match $ { is Bad => $.line, is Net => int.($), _ => -1 })
		e: Error = Bad.{ line = 7 }
		if e is Bad { print(e.line) }
	"#};
	check(src, ["4", "1", "7"]);
}

#[test]
fn embedded_err_promotes_error_claim() {
	let src = indoc! {r#"
		Parse :: struct { Err; line: int }
		Parse :< { message :: fn(self) string { "parse error at line {self.line}" } }
		Io :: struct { Err }
		boom :: fn(n: int) !int {
			if n == 0 { return Parse.{ line = 4 } }
			return Io.{ Err = Err.{ msg = "disk on fire" } }
		}
		discard loop n in 0..2 { boom(n) or { print($.message()); 0 } }
	"#};
	check(src, ["parse error at line 4", "disk on fire"]);
}

#[test]
fn embedded_trait_object_promotes_its_claim() {
	let src = indoc! {r#"
		Io :: struct { Error }
		Ctx :: struct { Error; where: string }
		Ctx : Error < { message :: fn(self) string { "{self.where}: {self.Error.message()}" } }
		boom :: fn(n: int) !int {
			if n == 0 { return Io.{ Error = error("disk on fire") } }
			return Ctx.{ Error = error("disk on fire"), where = "save" }
		}
		discard loop n in 0..2 { boom(n) or { print($.message()); 0 } }
	"#};
	check(src, ["disk on fire", "save: disk on fire"]);
}

#[test]
fn embedding_promotes_a_parameterized_claim() {
	let src = indoc! {r#"
		P :: struct { b: int }
		P : From[int] < { from :: fn(v: int) P { P.{ v } } }
		C :: struct { P; c: int }
		print(C.from(7).P.b)
	"#};
	check(src, "7");
}

#[test]
fn a_claim_binds_its_trait_type_params_in_fills() {
	let src = indoc! {r#"
		Pair[T] :: trait { pair: fn(self; v: T) []T }
		P :: struct { b: int }
		P : Pair[int] < { pair :: fn(self; v) []int { [self.b v] } }
		print(P.{7}.pair(8))
	"#};
	check(src, "[7, 8]");
}

#[test]
fn associated_types() {
	let src = indoc! {r#"
		Iterator :: trait {
			Item: type
			next: fn(mut self) ?Item
		}
		Count :: struct { n: int, end: int }
		Count : Iterator < {
			Item :: int
			next :: fn(mut self) ?int {
				if self.n >= self.end { return none }
				self.n = self.n + 1
				self.n - 1
			}
		}
		c := Count.{ n = 1, end = 4 }
		loop {
			match c.next() {
				.some.(v) => print(v),
				.none => break,
			}
		}
	"#};
	check(src, ["1", "2", "3"]);

	let src = indoc! {r#"
		It :: trait {
			Item: type
			get: fn(self) Item
		}
		R :: struct { n: int }
		R : It < {
			Item :: int
			get :: fn(self) string { "no" }
		}
		print(R.{1}.get())
	"#};
	fail(src, "`R.get` is `fn(R) string`, trait `It` declares `fn(R) int`");

	let src = indoc! {r#"
		Tr :: trait {
			Out :: Self
			k :: 5
			j : int : 6
			f: fn(self) Out
		}
		A :: struct { n: int, k :: 7 }
		B :: struct {}
		A : Tr < { f :: fn(self) Out { self } }
		A :< { g :: fn(self) Out { self } }
		B : Tr < { Out :: int, j :: 9, f :: fn(self) int { 3 } }
		print(A.{2}.f().n, B.{}.f(), A.k, A.j, B.k, B.j, A.{4}.g().n)
	"#};
	check(src, "2 3 7 6 5 9 4");

	let src = indoc! {r#"
		Tr[R] :: trait {
			Out: type
			f: fn(self; r: R) Out
		}
		A :: struct {}
		A : Tr[int] < { Out :: int, f :: fn(self; r: int) int { r } }
		A : Tr[float] < { Out :: float, f :: fn(self; r: float) float { r } }
		print(A.{}.f(2), A.{}.f(2.5))
	"#};
	check(src, "2 2.5");
	fail(r#"A :: struct { k : int : "s" }"#, "`A::k` must be a `int` literal");
}

#[test]
fn blanket_claims() {
	let src = indoc! {r#"
		Named :: trait { name: fn(self) string }
		Greet :: trait { greet: fn(self) string }
		[T: Named] T : Greet < { greet :: fn(self) string { "hi {self.name()}" } }
		Dog :: struct {}
		Dog : Named < { name :: fn(self) string { "rex" } }
	"#};
	check(
		&format!("{src}print(Dog.{{}}.greet(), Dog is Greet, int is Greet)"),
		"hi rex true false",
	);
	fail(&format!("{src}Dog : Greet < {{}}"), "`Dog` claims `Greet` twice");
}

#[test]
fn bounds_with_trait_args() {
	let src = indoc! {r#"
		Get[T] :: trait { get: fn(self) T }
		Box[T] :: struct { v: T }
		Box[T] : Get[T] < { get :: fn(self) T { self.v } }
		twice[T; W: Get[T]] :: fn(w: W) (T, T) { (w.get(), w.get()) }
		print(twice(Box.{ v = "a" }))
	"#};
	check(src, r#"("a", "a")"#);
}

#[test]
fn generic_claims() {
	let src = indoc! {r#"
		Box[T] :: struct { v: T }
		Box[T] : Add < {
			add :: fn(self, o: Self) Self { Box.{ v = self.v + o.v } }
		}
		Box[T: Cmp] : Eq < {
			eq :: { !(self.v < $.1.v) && !($.1.v < self.v) }
		}
		Foo :: struct { x: int }
		same[T: Eq] :: fn(a: T, b: T) bool { a == b }
		print((Box.{ v = 1 } + Box.{ v = 2 }).v, (Box.{ v = "a" } + Box.{ v = "b" }).v)
		print(Box[int] is Eq, Box[Foo] is Eq, same(Box.{ v = 2 }, Box.{ v = 2 }))
	"#};
	check(src, ["3 ab", "true false true"]);

	let src = indoc! {r#"
		Sh :: trait {
			s: fn(self) string
			loud :: fn(self) string { self.s().upper() }
		}
		Box[T] :: struct { v: T }
		Box[T] : Sh < { s :: fn(self) string { "{self.v} box" } }
		Box.{ 1 }.loud()
	"#};
	check(src, "1 BOX");

	let src = indoc! {r#"
		Tr :: trait {
			Out :: Self
			f: fn(self) Out
			g :: fn(self) Out { self.f() }
		}
		Box[T] :: struct { v: T }
		Box[T] : Tr < { Out :: T, f :: fn(self) Out { self.v } }
		Box[T] : Sub < { Out :: int, sub :: { 5 } }
		print(Box.{ "a" }.g(), Box.{ 9 } - Box.{ 2 })
	"#};
	check(src, "a 5");
}

#[test]
fn length_claims() {
	let src = indoc! {r#"
		size[T: Length] :: fn(x: T) int { x.len() }
		Bag :: struct { n: int }
		Bag : Length < { len :: fn(self) int { self.n } }
		print(size("abc"), size([1 2]), size([1 = 2]), size(Bag.{ 4 }))
	"#};
	check(src, "3 2 1 4");
}

#[test]
fn ordinal_claims() {
	let src = indoc! {"
		Dir :: struct { deg: int }
		Dir : Ordinal < {
			count :: 4
			index :: fn(self) int { self.deg / 90 }
			from_index :: fn(i: int) ?Dir { if i < 4 { Dir.{ i * 90 } } else { none } }
			ord :: fn(self) int { self.deg }
		}
		P :: enum { a b c }
		last[T: Ordinal] :: fn() ?T { T.from_index(T.count - 1) }
		print(ord(Dir.{180}), last[Dir]()?.deg, last[P](), last[bool](), last[u8](), rune.count)
	"};
	check(src, "180 270 some.(c) some.(true) some.(255) 1114112");
	fail("print(ord(1))", "does not claim `core::Ordinal`");
}
