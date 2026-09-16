---
id: syntax
aliases: []
tags: []
---
# [[Oi|../]]
> NOTE: I wrote this file in Obsidian. It craps out and looks like poo in Neovim because of comments with markdown.

```rust
## comments

# Single line comments
# (can be stacked)

#{ Block comments
	#{ (can be nested) }#
}#

## Doc comments.
##
## # support markdown
## ```
## # code block language defaults to Oi
## ```

## modules

#{
	- directory == module
	- every .oi file in the dir contributes to the module
	- modules looks for single adjacent files if no folder exists (`use foo` finds `./foo.oi`)
	- no mod/index files are needed, so `mod.oi` is merely a file
}#

# declare module
module module_name

# imports are dot-separated paths
use os
use fs.stream

# selective imports
use os.{ input }
use fs.{ stream, sock :: socket }

# import aliases
# `use` is an expression, and thus it may be bound
mysha256 :: use hash.sha256

# visibility

# any immutable expression can be made public with a `pub` modifier

# re-export modules
pub use math

# definitions
pub quack :: fn() string { "quack!" }
pub Foo :: struct { bar: bool }
pub foo :: Foo.{ true }
pub strange :: "dr strange love"

# a module level `:=` is a static binding

## FFI

# `foreign` binds a value that lives outside the program
oi_str_contains : fn(s: string, sub: string) bool : foreign
oi_panic : fn(msg: string) int : foreign
errno : i32 : foreign

# `@link` names the library to load and/or the symbol to bind
@link.{"z"}
zlibVersion : fn() cstr : foreign
@link.{"z", name = "zlibVersion"}
version : fn() cstr : foreign

# `ptr` is an untyped address
# `unsafe` is required for reaching through pointers and calling `foreign` fns
memset : fn(p: ptr, c: int, n: usize) : foreign
unsafe memset(buf.ptr, 0, 4)
assert!(unsafe { "hello".ptr.offset(1).string(2) } == "el")

# `@unsafe` colors a function and callers need to use it
@unsafe
first :: fn(p: ptr) i32 { unsafe p.array[i32](1)[0] }

# `@export` makes a fn callable from C
@export
add :: fn(a: int, b: int) int { a + b }
@export.{"godot_init"}
init :: fn(p: ptr) bool { !p.is_null() }

# `@c` lets structs cross C ABI
@c
Header :: struct { tag: u8, len: u32 }
h := unsafe { p.read[Header]() }
unsafe p.write(h)

# `@c` on a fn makes it callable by C
# `@export` is `@c` plus a public name
@c
on_init :: fn(get: ptr) bool { !get.is_null() }

# `@c` on a fn type is a C fn pointer
# a foreign decl's fn params are implicitly `@c`
GetProc :: @c fn(name: cstr) ptr
atexit : fn(cb: @c fn()) i32 : foreign

# a `@c fn` can cast a `ptr`
create : fn(init: @c fn(get: GetProc) bool) ptr : foreign
create(fn(get: GetProc) bool { !get("iface").is_null() })
proc := unsafe { GetProc.(dlsym(lib, "get_proc")) }

## functions

# private within module by default
foo :: fn() {
	print("foo")
}

# use `pub` modifier to make visible to outside modules
# TODO: likely `pub(scope)` a la Rust
pub bar :: fn() {
	print("bar")
}

# param modifiers

# a plain param is a read-only borrow
# `mut` is an exclusive borrow
pub baz :: fn(mut i: int) {
	i += 2
}

# callsites specify modifiers too
n := 1
baz(mut n)
assert!(n == 3)

# while a mutable variable is lent, nothing else may touch it for the call
# f(mut a, a) # error

# implicit return

add :: fn(x: int, y: int) int {
	x + y
}

random_user :: fn() User {
	user := User.{}
	user.name = "I Dunno"
	user
}

# implicit input data

# `$` is the data passed to a function
# `$` inherits mutability from the fn signature

# `$` directly matches the call signature, so it is strongly typed and enforceable by the compiler
single_val :: fn(x: int) {
	assert!(x == $)
}
oneple :: fn(x: int,) {
	assert!(x == $.0)
}
twople :: fn(x: int, y: int) {
	assert!(x == $.0)
	assert!(y == $.1)
}

# named returns

## Bound name is initialized as a mutable zero-value of the specified return type.
## I feel like it's ergonomic and not too magical, but maybe others will disagree.
##
## Although Oi looks like V and in turn Go, and Go has a named return feature itself (which V interestingly did not copy),
## I really attribute this more to Nim's implicit `result`:
## - https://nim-by-example.github.io/variables/result/
## - 
## Nim's `result` is great, but I don't like that it's magic.
## So I opted to make you opt-in by naming it explicitly.

# building on what Go/Odin do, a bare `return` updates the bound values
two :: fn() result: int {
	assert!(result == 0)
	result = 2
	return
}
assert!(two() == 2)

random_user :: fn() u: User {
	print(u) # User.{}
	u.name = "I Dunno"
	return
}
ru :: random_user()
assert!(ru.name == "I Dunno")

# default values
three :: fn() r := 2 {
	r = 3
	return
}

# trailing params can default too, evaluated at the callsite
# a default may reference any earlier param, including `self`
wrap :: fn(s: string, left: string, right: string = left) string { left + s + right }
assert!(wrap("hi", "|") == "|hi|")

# `:=` takes the type from the default
step :: fn(x: int, by := 1) int { x + by }
assert! step(1) == 2

# named arguments

# an argument may be named, to reorder or to skip over a default
win :: fn(title: string, x := 0, y := 0, w := 854, h := 480) string { "{title} {w}x{h}" }
assert! win("hi", h = 360) == "hi 854x360"

# this really just skips the step of explicitly initializing a zeroed var
divmod :: fn(a: int, b: int) out: (int, int) {
	out.0 = a / b
	out.1 = a % b
	return
}

# varargs

sum :: fn(xs: ..int) int { xs.fold(0, +) }
print :: fn(args: ..any)

between :: fn(open: string, items: ..string, close: string) string
between("[", "a", "b", "c", "]") # ["a", "b", "c"]
between("[", "]") # []
join :: fn(parts: ..string, sep := ",") # defaults after varargs are named-only
join("a", "b", sep = "/")

log(:info, ..parts) # a slice spreads into the vararg slot, no copy
log(:info, "pre", ..parts, "post")

## pure functions

# `@pure` is a compiler-verified contract for deterministic functions with no side effects.
# A @pure fn may not perform IO, read/write globals or module-level state, or call non-pure functions.
# @pure implies non-capturing (no enclosing locals), so [] is redundant alongside it.
# Can be applied to both named and anonymous functions.

@pure
add :: fn(x: int, y: int) int { x + y }

@pure
clamp :: (value: f64, low: f64, high: f64) f64 {
	match true {
		value < low => low,
		value > high => high,
		else => value,
	}
}

# @pure fns may call other @pure fns
@pure
sum_of_squares :: fn(a: int, b: int) int {
	add(square(a), square(b))
}

# anon @pure
# useful when passing to a higher-order fn that requires a purity guarantee
# (or for theoretically possible optimization from the theoreticl compiler)
result :: data.map(@pure fn (x: int) int { x * x })

## leading literals

# if there's only one literal arg, the parens may be dropped
foo "bar"
print "lol"
sleep 1_000
log.group :process

# this can be used in conjunction with trailing functions
benchmark 1_000_000 { do_work() }
config :production { ... }
hook .startup { ... }

## structs

Point :: struct {
	x: int
	y: int
}

point :: Point.{
	x = 19
	y = 90
}

# one line
point :: Point.{ x = 19, y = 88 }

# zero values when unspecified
origin :: Point.{}

# support default field values
User :: struct {
	age: int
	name: string
	swag: int = 5
}

# references
# this is an explicit heap allocation
# `&User.{}` makes one shared value, copies of the reference are aliases
u := &User.{}
v := u # same user, not a copy
v.swag = 9
assert!(u.swag == 9)
# writing through a reference requires a `mut` binding

# required fields
Foo :: struct {
	n: int @required
}

# short struct literals
normal :: Point.{
	x = 2
	y = 1
}
short :: Point.{3, 2}

# a positional prefix can stop early, leaving the rest zeroed
partial :: Point.{3}

# the type name can be dropped when it's known from context (typed decl, call arg, return, field value)
p : Point = .{ x = 2, y = 1 }

# positional values go before named fields, like call args
q :: Point.{3, y = 1}

# tuple structs

# a struct can take a tuple body instead of a record body
Money :: struct (int)
Point :: struct (x: float, y: float)
m :: Money(500)
p :: Point(1.0, 2.0)
p :: Point(x = 1.0, y = 2.0) # named call args work too
assert!(p.0 == p.x)
print(m.0) # 500

# these types are distinct, unlike type aliases
pay :: fn(m: Money) {}
# pay(500) # error: expected Money, got int
pay(Money(500))

# can receive methods
Money :< {
	str :: fn(self) string {
		"${self.0}"
	}
	double :: fn(self) Self {
		self * 2
	}
}
print(Money(5)) # $5
print(Money(5).double()) # $10

# tuple structs are nominal step-bros of type aliases
# transparent types
Money :: float
UserId :: int | string
# distinct types
Money :: struct (float)
UserId :: struct (int | string)

# struct update

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

user := User.{
	name = "abc"
	age = 23
}
user = register(user)

# trailing records

Options :: struct {
	foo: int
	bar: bool
}
User :< {
	with_options :: fn(self, opt: Options) {
		print(opt)
	}
}
user :: User.{}
user.with_options(bar = true, foo = 4)
# same record literal as `Options.{ bar = true, foo = 4 }`, just braceless, coerced against the last param's struct type

# annotating with `@params` lets a trailing record be omitted
# otherwise you need to specify at least one field or the compiler will error
# (@params means the empty literal `.{}` is allowed too)
@params
Settings :: struct {
	idk: int
}
User :< {
	with_settings :: fn(self, settings: Settings) {
		print(settings)
	}
}
user.with_settings()

# visibility

User :: struct {
	# fields are private by default
	name: string
	# `pub` modifier can alter the visibility
	pub email: string
}

# anonymous structs

Food :: struct {
	name: string
	nutrition: struct {
		calories: int
	}
}

# the dot literal coerces to the field's type
apple := Food.{
	name = "apple"
	nutrition = .{
		calories = 4
	}
}

# short struct literals work too, named or positional
pear := Food.{
	name = "pear"
	nutrition = .{ 5 }
}

# with nothing expected, a fully named literal infers an anonymous struct
pos := .{ x = 1, y = 2 }

# anonymous structs are usable in any type signature
open :: fn(opts: struct { retries: int }) struct { ok: bool } {
	.{ ok = opts.retries > 0 }
}

# static struct methods
User :< {
	new :: fn() Self {
		Self {}
	}
}
user :: User.new()

# struct methods
User :< {
	can_register :: fn(self) bool {
		self.age > 16
	}
	# calling a `mut self` method requires the receiver to be a `mut` binding
	set_age :: fn(mut self, age: int) {
		self.age = age
	}
}

# embedded structs

Profile :: struct {
	Options
	name: string
}

profile :: Profile.{
	foo = 4
	name = "one cool dude"
}
assert! profile.foo == profile.Options.foo

# you can refer to and assign to embedded structs directly
profile := Profile.{
	Options = Options.{
		foo = 200
	}
}
print(profile.Options)
profile.Options = Options.{}

# operator overloading

# can override by claiming built-in traits, like `Add`, `Sub`, `Mul`, `Div`, `Mod`

Point :: struct {
	x: int
	y: int
}

Point :< {
	zero :: fn() Self { Point.{0, 0} }
}

Point : Add < {
	add :: fn(self, other: Self) Self {
		Self.{ self.x + other.x, self.y + other.y }
	}
}
assert!(Point.{1, 0} + Point.{2, 3} == Point.{3, 3})

# `==` and `!=` compare values
assert!((1, "a") == (1, "a"))

# claim `Eq` to define equality yourself
Frac :: struct { num: int, den: int }
Frac : Eq < {
	eq :: fn(self, other: Self) bool {
		self.num * other.den == other.num * self.den
	}
}
assert!(Frac.{1, 2} == Frac.{2, 4})

# unary `-` dispatches to a `Neg` claim
Point : Neg < {
	neg :: fn(self) Self { Self.{ -self.x, -self.y } }
}
assert!(-Point.{1, 2} == Point.{-1, -2})

# `Ord` orders a type with a single `lt`. `< > <= >=` all derive from it
Frac : Ord < {
	lt :: fn(self, other: Self) bool { self.num * other.den < other.num * self.den }
}
assert!(Frac.{1, 3} < Frac.{1, 2})

# enums can claim operator traits too
Dir :: enum { up, down }
Dir : Ord < {
	lt :: fn(self, other: Self) bool { other == .up }
}
assert!(Dir.down < Dir.up)

# arithmetic fills don't need to be of the same type
Scale :: struct { f: int }
Scale : Mul < {
	mul :: fn(self, other: int) Self { Self.{ self.f * other } }
}
assert!(Scale.{3} * 2 == 2 * Scale.{3}) # "+/* are commutative

## traits

# a trait is a set of behaviors and/or data
Animal :: trait {
	# field requirement
	kind: string

	# method requirement
	speak: fn(self) string

	# default methods build on the requirements
	# may be overridden
	shout :: fn(self) string {
		self.speak().upper()
	}
}

Dog :: struct { kind: string }
Cat :: struct { kind: string }
Person :: struct {
	kind := "Human"
}

# traits are satisfied by an explicit implementation
Dog : Animal < {
	speak :: fn(self) string { "woof" }
}
Cat : Animal < {
	speak :: fn(self) string { "meow" }
}
Person : Animal < {
	speak :: fn(self) string { "Lorem ipsum..." }
}

# an embedded struct can satisfy requirements too
Meta :: struct { kind: string, id: int }
Enemy :: struct {
	Meta
	hp: int
}
Enemy : Animal < { speak :: fn(self) string { "rawr" } }

demo_traits :: fn() {
	dog :: Dog.{"Collie"}
	cat :: Cat.{"Egyptian Mau"}
	animals :: Animal.[dog, cat]

	loop animal in animals {
		print "a {animal.kind} says: {animal.speak()}"
	}
}

# use `is` to check whether a type satisfies a trait
assert!(Person is Animal)

# traits can use `@implicit` to opt-in to structural / duck typing
# any type with the right shape satisfies it even without an implementation block
@implicit
Fruit :: trait {
	seeds: bool
	color: Color
}
Kiwi :: struct {
	seeds := true
	color: Color = :green
}
Apple :: struct {
	seeds := true
	color: Color = :red
}
Bike :: struct {
	color: Color = :purple
}
# a bare claim needs no body when the shape is already satisfied
Apple :< Fruit
assert!(Kiwi is Fruit)
assert!(Apple is Fruit)
assert!(Bike is not Fruit)

## static vs dynamic dispatch

# A trait used as a bound is static: monomorphized per concrete type.
# no vtable, no indirection, no allocation
greet[A: Animal] :: fn(a: A) { print(a.shout()) }

# A trait used directly as a type is dynamic: a trait object behind a vtable.
zoo := Animal.[Dog.{"collie"}, Cat.{"mau"}, Enemy.{ kind = "boss", hp = 9 }]
loop a in zoo { print "a {a.kind} says {a.speak()}" }

## associated types

# an associated type is just a slot whose type is `type`, filled per claim
Iterator :: trait {
	Item: type
	next: fn(mut self) ?Item
}
Range :: struct { start: int, end: ?int, step: int }
Range : Iterator < {
	Item :: int
	next :: fn(mut self) ?int {
		done := match self.end {
			some(e) => (self.step > 0 && self.start >= e) || (self.step < 0 && self.start <= e),
			none => false, # no end means unbounded
		}
		if done { return none }
		defer self.start += self.step
		self.start
	}
}

## supertraits

# require another trait alongside this one
# every Ord is also an Eq
Ord : Eq : trait {
	cmp: fn(self, other: Self) Ordering
}
max[T: Ord] :: fn(a: T, b: T) T {
	if a.cmp(b) == .greater { a } else { b }
}

# a type param may default, for when nothing at the callsite pins it
parse_int[T = int] :: fn(s: string) ?T { ... }
n :: parse_int("42")? # T = int
b :: parse_int[u8]("42")? # an explicit arg or an inferred one wins

## associated constants

Bounded :: trait {
	min: Self
	max: Self
}
i8 : Bounded < {
	min :: -128
	max :: 127
}

## conversions

# traits can have type params too
From[T] :: trait { from: fn(v: T) Self }
TryFrom[T] :: trait { try_from: fn(v: T) !Self }

Money :: struct { cents: int }
Money : From[int] < { from :: fn(n: int) Self { Money.{ n } } }
Money : From[string] < { from :: fn(s: string) Self { Money.{ int.try_from(s) or 0 } } }
assert!(Money.from(5).cents == Money.from("5").cents)

## blanket claims

# claim a trait for every type that already meets a bound
#{ TODO: this worked better before I made a big redesign pass. doesn't fit well anymore
ToString :: trait {
	to_string: fn(self) string
}
[T: Display] T : ToString < {
	to_string :: fn(self) string { self.display() }
}
}#

## marker traits

# traits don't have to have methods or fields or anything
Copy :: trait {}
Point :< Copy

## delegation

# `via` routes a claim through an embedded field that already satisfies it
Horn :: struct { kind: string }
Horn : Animal < {
	speak :: fn(self) string { "honk" }
}
Car :: struct {
	Horn
}
Car :< Animal via Horn

# a `via` claim may override individual methods, routing the rest through the stated field
Car : Animal via Horn < { speak :: fn(self) string { "HONK HONK" } }

## composite types

#{
	every type constructor composes with every other, to any depth

	| Oi Syntax | Meaning | Rust Equivalent |
	| --- | --- | --- |
	| `[]T` | Dynamic array | `Vec<T>` |
	| `[N]T` | Fixed array | `[T; N]` |
	| `[K]V` | Map | `HashMap<K, V>` |
	| `(A, B)` | Tuple | `(A, B)` |
	| `?T` | Optional | `Option<T>` |
	| `!T` | Result | `Result<T, _>` (error is any `Error`) |
	| `E!T` | Result | `Result<T, E>` (error pinned to `E`) |
	| `&T` | Shared reference | `Rc<T>` |
	| `fn (A) R` | Function | `fn(A) -> R` |
	| `..T` | Vararg, param position only, body sees `[]T` | - |
	| `Foo[T]` | Generic instance | `Foo<T>` |
	| `Trait` | Trait object | `&dyn Trait` |

	the prefix shorthands (`[]` `[N]` `[K]` `?` `!` `&`) read left-to-right
	everything else nests in brackets
}#

# nests to any depth
Parsed :: Result[[]?Token, ParseError]

# order matters. these are different types
# `?[]int` -> `Option[[]int]` -> the whole list may be absent
# `[]?int` -> `[]Option[int]` -> each slot may be absent

World :: struct {
	entities: []Entity
	sessions: [UserId][]Session
	cache: [string]?[]u8
	handlers: []fn (Request) !Response
	grid: [16][16]Tile
}

# ?T and !T are shorthands
# `?T` -> `Option[T]`
# `!T` -> `Result[T, Error]`
# `E!T` -> `Result[T, E]`
# the long form and `E!T` pin the error to a specific type
read :: fn(path: string) Result[[]u8, io.Error] { ... } # error pinned
slurp :: fn(path: string) ![]u8 { ... } # error left open
parse :: fn(src: string) ParseError!Ast { ... } # shorthand pin

# generics may nest
Grid[T] :: [][]T
Lookup[V] :: [string]?V

# in expression position a bracket is an index, unless a literal or call follows
grid[x][y] = 0
boxed :: Box[int].{ v = 7 }

# the sandwich middle also names the type when there's nothing to infer from
empty : Stack[int] = Stack.{}
meters : Tagged[Meters] = Tagged.{ value = 5.0 }

## main entrypoint

main :: fn() {
	## variables

	# assignment

	# declaration without assignment
	foo: int # 0
	bar: string # ""
	p: Point # Point.{}
	grid: [3]string # ["", "", ""]

	# declaration with assignment
	a: int = 2
	b: string = "hi"
	c: Car = Car.{}
	m: [int]string = []

	# inferred
	no_mute :: "immutable"
	mute := "mutable"
	mute = "trololololol"

	# muliple assignment
	(foo, bar) :: ("food", "bard")
	(lat long) :: get_coords()

	# swap
	(baz, qux) := ("bazd", "quxd")
	(baz, qux) = (qux, baz)

	# destructuring

	[first second] :: [1 2 3]
	Point.{ x, y } :: Point.{ 1, 2 }
	Point.{ y = b, x } := Point.{ 1, 2 }

	# `_` skips
	(one, _) :: (1, 2)

	## primatives

	bull :: true
	str :: "string"
	integer :: 1337
	flt :: 69.420

	# ranges
	# half-open by default, `..=` makes it inclusive
	between := 1..3                    # 1, 2
	inclusive := 1..=3                 # 1, 2, 3
	# prefix `..x` on an int is `0..x`, postfix `x..` is open-ended
	until := ..3
	after := 1..
	crossing_over_with_john_edward := -4..4

	# first, second, last: step and its sign are inferred
	evens := 0..2..=10
	countdown := 10..9..0

	# paths
	# TODO: path literal

	# numbers

	# number litarals are `int` (`i64`) and `float` (`f64`) unless otherwise indicated
	i := 55 # int AKA i64
	f := 55.55 # float AKA f64
	e_notation_float := 10e2 # 1000.0

	# can use a prefix to denote common notations
	# these are all 123
	a0 := 123
	a1 := 0x7B
	a2 := 0b01111011
	a3 := 0o173

	# can separate arbitrarily with `_`
	bil := 1_000_000_000
	wtf := 1_2_3_4_5
	floater := 10_000.22
	binary_mask := 0b1_1111_1111
	permissions := 0o7_5_5
	big_addr := 0xFF80_0000_0000_0000

	# cast with `T.(value)`, which works for any type
	big_int := i64.(50_000)
	small_unsigned_int := u8.(16)
	Point.(p); Money.(500); ?int.(42)

	# narrowing truncates
	assert!(u8.(300) == 44 && int.(2.9) == 2)

	# ints can be automatically promoted to f64 or larger-width ints
	assert!(2 + 1.0 == 3.0)

	# supports arbitrary bit-width integers, like Zig
	# use `i<width>` and `u<width>`, where width in [1, 65535]
	weird_one := i2.(1)
	wat := u7.(1000)

	# supported floating types are: f16 f32 f64 f80 f128

	# strings

	normal := "NORMAL mode"
	raw := r"there is no\nescape"
	regex := r"\d+\.\d+"
	multiline := "
		strings are multiline
		by default
	"

	# strings are immutable bytes behind a pointer+length handle
	# (the buffer keeps a trailing NUL outside `len`, so it passes to C as-is)
	assert!("hello".len == 5)

	# `.cstr()` converts to `cstr`, a NUL-terminated pointer for C boundaries
	# `.str()` copies back the other way
	assert!("hello".cstr().str() == "hello")

	# a string literal passes where `cstr` is expected, zero-cost (already NUL-terminated)

	# indexing is by byte and bounds-checked, like arrays
	assert!("abc"[1] == 98)
	assert!("abc".1 == 98)

	# range indexing gives a view into the same buffer, no copy
	assert!("hello"[1..3] == "el")
	assert!("hello"[2..] == "llo")

	# concatenation
	assert!("foo" + "bar", "foobar")

	# string interpolation
	who := "mom"
	print("hi {who}!")

	# any expression works inside braces
	user := User { name = "alice", age = 30 }
	print("{user.name} is {user.age}")
	print("sum: {2 + 2}")
	print("upper: {who.upper()}")

	# escape braces by doubling
	print("use {{braces}} like this")

	# works in multiline strings
	msg := "
		dear {who},
		your balance is {amount}.
	"
	# but no interpolation in raw strings
	path := r"C:\Users\{who}" # {who} is not interpolated

	# arrays

	# collection of 0-indexed elements of the same type
	names := ["john", "jacob", "jingleheimerschmidt"]
	print(names)
	# can be accessed with an index expression
	assert!(names[1] == "jacob")
	i := 1
	assert!(names[i] == names[1])
	# numbers literals may also be used with dot notation
	assert!(names.0 == "john")
	assert!(names.2 == "jingleheimerschmidt")

	# append with `<<`
	odd := [1, 3, 5]
	odd << 7
	assert!(odd.3 == 7)
	# entire arrays can be appended too
	odd << [9 11]
	assert!(odd.5 == 11)
	assert!(odd.len == 6)

	# arrays support dropping the commas when only literals are present
	even := [2 4 6]

	# `..` spreads an array into a literal
	all := [..odd, ..even]

	# `in` operator returns whether array contains element
	assert!(6 in even)

	# arrays have fields
	# `len` is the number of initialized elements in the array
	assert!(even.len == 3)

	# array init
	arr: []int
	arr << 3

	# fixed size arrays

	# dot literals are exact values, with static size, copy semantics, no growth, no allocation
	nums := i32.[1 2 3] # [3]i32
	# anonymous `.{}`, `.[]`, and `.()` resolve against the expected type
	stooges: [3]string = .["Larry" "Curly" "Moe"]
	stooges.2 = "Emil Sitka"

	# a fixed array coerces (and makes copy) to `[]T` where one is expected
	roster: []string = stooges

	# maps

	# `key = value` entries make a bracket literal a map
	# keys are literals or variables
	by_id := [1 = "one", 2 = "two"] # [int]string
	by_status := [:ok = 200, :not_found = 404]
	k :: "one"
	num_map := [k = 1, "two" = 2]
	print(num_map["one"])
	typed_map: [string]int
	typed_map["three"] = 4
	typed_map.delete["three"]

	# empty `[]` resolves against the expected collection type
	empty [string]int := []

	# array slices are subsets of another array
	# used in place a slice is a free view. stored, it is an independent COW value
	# proper array
	even := [0 2 4 6 8]
	# slices of it
	assert! even[1..3] == [2 4]
	assert! even[..3] == [0 2 4]
	assert! even[1..] == [2 4 6 8]

	# tuples

	# tuples are very important in Oi
	# under the hood many things are tuples, and some if it bleeds through in [hopefully] interesting ways
	# function input params are [planned to be] treated as tuples in the compiler

	# the `$` var you've seen in other places makes this really clear
	its_all_tuples_man :: fn(a: bool, b: int, c: string) (bool, int, string) {
		$
	}
	result := its_all_tuples_man(true, 2, "lol")
	print(result) # (true, 2, "lol")

	# tuples support dropping the commas when only literals are present
	only_nums := (2 3 4)
	other_literals := ("lisp, innit?" true [2 4 5])

	# named tuple fields

	## Naturally every tuple field has a positional index.
	## But they can also optionally be given names.
	## This should remind the reader of tables in Lua (and Revo <3).

	t := (a = 1, b = 2)
	print(t) # (a = 1, b = 2)
	assert!(t.a == t.0)
	assert!(t.b == t.1)

	#{
		These names are purely aliases / hints, and do _not_ affect identity or comparison.
		Think of it like somebody asks us if their rock is the same as our rock.
		We can tell that they are the same, we just happen to know a lot more details about our rock than theirs.
		I've never been great with analogies.
		Anyway don't abuse this. The field names are for convenience, not as a replacement for structs.
	}#
	assert!((x = 4, y = 2) == (4, 2))
	assert!((x = 4, y = 2) == (4, z = 2))

	# names do not need to be given to all indices
	t := (1, b = 2)
	print(t) # (1, b = 2)
	assert!(t.b == t.1)

	# can be used in function return signatures
	split :: fn(value: string) (left: string, right: string) {
		split_once(value, "|") # returns a 2-tuple (a twople? anyone?)
	}
	splat := split("hi|mom")
	(l, r) := split("hi|mom")
	assert!(splat.left == "hi")
	assert!(splat.right == "mom")
	assert!(splat == (l, r))

	# another example with a common divmod method
	divmod :: fn(a: int, b: int) (q: int, r: int) {
		(a / b, a % b)
	}
	result := divmod(10, 3)
	print(result) # (q = 3, r = 1)
	assert!(result == (3, 1))
	assert!(result.0 == 3)
	assert!(result.1 == 1)
	assert!(result.q == 3)
	assert!(result.r == 1)

	# a tuple spreads into fixed slots, arity statically checked
	show(..divmod(10, 3))
	forward :: fn(a: int, b: int) string { show(..$) }

	# this can be used alongside the named return feature, as they are different systems
	divmod :: fn(a: int, b: int) out (q: int, r: int) {
		out.q = a / b
		out.r = a % b
		return
	}

	http_get :: fn(url: string) (int, body string, []Header) {
		(200, "the body", [])
	}
	result := http_get("/health")
	print(result) # (200, body = "the body", [])
	assert!(result.body == result.1)

	## unit type

	# (), a 0ple, is the unit type
	# when you have a fn with no return type expressed, it returns `()`
	assert!(() == ())

	# these are all equivalent:
	nada :: fn() {}
	zilch :: fn() () {}
	nope :: fn() {
		()
	}
	no_way :: fn() {
		return ()
	}
	nuh_uh :: fn() {
		return
	}
	assert!(nada() == zilch())
	assert!(nada() == nope())
	assert!(nada() == no_way())
	assert!(nada() == nuh_uh())
	assert!(nada() == ())

	## never

	# `never` indicates that a fn should not return
	foo :: fn() never {
		loop {}
	}
	foo()
	unreachable!("the above fn should never have finished")

	## atoms

	# Oi has first-class atoms
	:foo
	assert!(:foo != :bar)
	food := :apple
	assert!(food == :apple)

	# atoms coerce to enum variants when the type is known from context
	# NOTE: atoms by definition cannot carry payloads
	Color :: enum { red blue }
	c := Color.red # fully qualified
	c = .red # type inferred from declaration
	c = :blue # type inferred from declaration and coerced
	assert!(c == Color.blue)
	assert!(Color.blue == :blue)

	Stat :: enum { health mana stamina }
	User :: struct {
		stat: Stat
	}
	user1 := User.{ stat = .mana }
	user2 := User.{ stat = :mana }
	assert!(user1.stat == user2.stat)

	# this might be useful for quick prototyping?
	# nothing at the callsites needs to change when you later add the definition
	# NOTE: TBH I might remove this feature or make it a compiler warning when a typed enum exists.

	# prototype code
	state := :loading
	state = :ready

	# on a later pass, despite nothing at the callsites changing, adding this enum definition would add strong typing and copiler checking
	# STYLE: if an enum exists, prefer `.foo`
	State :: enum { loading ready error }

	# atoms work in type position
	status :: fn() :ok { :ok }

	# `atom` is the open type
	tag :: fn() atom { :ok }
	label :: fn(a: atom) string { "got {a}" }
	tags : [atom]int : [:a = 1, :b = 2]

	## types

	# type aliases
	type Score = int
	type Speed = (Point, int)

	# function signatures can be aliased
	type Operation = fn (int) int
	op :: fn(n: int, f: Operation) int {
		return f(n)
	}
	double :: fn(n: int) int {
		return 2 * n
	}
	# explicit cast
	print(op(4, Operation.(double))) # 8
	# duck typing
	print(op(4, double)) # 8
	# anonymous function
	print(op(4, fn (n int) int {
		return 3 * n
	})) # 12
	# anonymous function shorthand (types inferred, input accessible via `$`)
	print(op(4, fn { $ * 4 })) # 16

	# all types have zeroed values
	u := User.{}
	assert!(u.age == 0)
	assert!(u.name == "")

	## control flow

	i := 2
	if i == 0 {
		print("zero")
	} else if i == 1 {
		print("one")
	} else {
		print("idk")
	}

	# `do` takes a single statement instead of a block, anywhere a block is expected
	if i == 2 do print("two")
	if i > 0 do print("positive") else do print("nope")
	loop i < 9 do i += 1
	loop n in 0..3 do print(n)
	triple :: fn(n: int) int do n * 3

	## matching

	# arms are `pattern => expr,` (last comma optional) or `pattern => { block }`

	# else for catch-all
	os := "linux"
	match os {
		"darwin" => print("I used to hate macOS but now I realize it's at least better than Windows."),
		"linux" => print("I use Artix Linux btw"),
		else => print(os),
	}

	# can be used as an if-else chain
	# evaluated in order, first match wins if multiple satisfy the condition

	# comma can be used to test multiple values
	is_red_or_blue :: fn(c: Color) bool {
		return match c {
			.red, .blue => true,
			.green => false,
		}
	}

	# TODO: not sure whether Oi should support `$` in match or use binding
	match user {
		u @ User { age = 0..18 } => "minor: {u.name}",
		User { age = 0..18 } => "minor: {$.name}",
		_ => "adult",
	}

	## loops

	# `loop {}`: infinite
	# `loop <cond> {}`: while
	# `loop <pattern> in <iter> {}`: for

	# forever
	loop {
		print("are we there yet?")
	}

	# while
	i := 0
	loop i <= 3 {
		print("are we there yet?")
		i += 1
	}

	# for
	loop i in 0..5 {
		print(i)
	}

	# foreach
	loop x in [2 4 6 8] {
		print(x)
	}
	loop (x, y) in [(0, 0) (1, 2)] {
		print((y, x))
	}
	loop Point.{ x } in [Point.{ 1, 2 }] {
		print(x)
	}
	loop [a b] in [[1 2] [3 4]] {
		print(a + b)
	}
	loop b in "hi" {
		print(b)
	}
	loop (k, v) in ["one" = 1, "two" = 2] {
		print("{k}={v}")
	}

	# TODO: custom iterators

	## [almost?] everything is an expression

	# ternary (`if` is an expression)
	foo := if true { "yes" } else { "no" }

	# if no else, uses default value from the if body
	# TODO: or should it be `none` and make the var `?T`?
	str := if false { "idk" }
	num := if false { 42 }
	assert!(str == "")
	assert!(num == 0)

	# built-in functions
	result := assert!(check()) |> next

	# match
	(i, foo, bar, u, me) := (0, true, true, 2, [0 2 4])
	n := match true {
		i < 3 => "love ya",
		foo == bar => "soul mates",
		u in me => "🥵",
		else => "no dice",
	}

	## `Option` and `Result` types

	# `?T` holds `some(T)` or `none`
	# `!T` holds `ok(T)` or an `error` (any type implementing the `Error` trait)
	# bare return values are auto-wrapped
	# there is no need for an explicit `ok()` or `some()` un/wrapper like there is in Rust

	Repo :: struct {
		users []User
		cached_name ?string # zero value is `none`
	}

	Repo :< {
		# !T returns a value or an error
		find_user :: fn(id: int) !User {
			loop user in self.users {
				if user.id == id { return user }
			}
			return error("User {id} not found")
		}

		# ?T returns a value or `none`
		find_user_if_exists :: fn(id: int) ?User {
			loop user in self.users {
				if user.id == id { return user }
			}
			return none
		}
	}

	# ?T and !T must be handled, and the or block is required to unwrap
	# $ is the Error value (!T) or none (?T)
	user := repo.find_user(7) or {
		print($.message()) # "User 7 not found"
		return
	}

	# or block can yield a fallback value of the same type
	user := repo.find_user(7) or { User.{ name = "guest" } }

	# check error type in the or block
	file := fs.open(path) or {
		if $ is fs.NotFoundError { return create_default() }
		panic!($.message())
	}

	# postfix `?` propagates up to the caller: error out of a !T fn, none out of a ?T fn
	# panics if used in main()
	load_config :: fn(path: string) !Config {
		raw := fs.read(path)?
		parse(raw)?
	}

	display_name :: fn(id: int) ?string {
		user := repo.find_user_if_exists(id)?
		user.name
	}

	# creating option/result values directly
	nope :: ?int.(none)
	maybe :: ?int.(42)
	ok :: !int.(7)
	broken :: !int.(error("oops"))

	# ?T / !T wrap the whole tuple in multi-return
	checked_divmod :: fn(a: int, b: int) !(int, int) {
		if b == 0 { return error("division by zero") }
		(a / b, a % b)
	}
	(q, r) := checked_divmod(10, 3)?

	# pinned errors

	# the error type can be specified with `E!T`
	# this is shorthand for `Result[T, E]`, which is equivalent
	NetError :: enum { timeout refused }
	fetch :: fn(url: string) NetError!Response {
		# ...
		return .timeout
	}

	# `$` in the `or` block is the return from the called fn, which by definition means it will always be the specified error type
	res := fetch(url) or {
		if $ == NetError.timeout { return retry(url) }
		panic!("refused")
	}

	# the "error" type can be anything
	lookup : fn() Result[int, string]

	# `error()` can be used to disambiguate when ok/err are both the same type
	foo :: fn() int!int {
		if something_went_wrong() { return error(-42) }
		return 42
	}

	# custom error types
	# embed Error for default impls, only override what you need

	ParseError :: struct {
		Error
		line: int
		col: int
	}
	ParseError :< {
		message :: fn(self) string { "parse error at {self.line}:{self.col}" }
	}

	parse :: fn(src: string) !Ast {
		...
		return ParseError.{ line = 4, col = 2 } # auto-cast to Error
	}

	parse(src) or { panic!($.message()) }

	# error chaining via cause()
	WrappedError :: struct {
		Error
		msg: string
		inner: Error
	}
	WrappedError :< {
		message :: fn(self) string { self.msg }
		cause :: fn(self) ?Error { self.inner }
	}


	## enums

	# plain
	Color :: enum {
		red
		green
		blue
	}
	# fully-qualified enum, for when inference can't help
	c := Color.green
	# shorthand enum when the type is known from context
	c = .red

	# variants with payloads
	Shape :: enum {
		circle { radius: f64 }
		rectangle { width: f64, height: f64 }
		triangle(f64, f64, f64)
		point
	}
	s := Shape.circle { radius = 5.0 }
	s := .circle { radius = 5.0 }
	s := Shape.triangle(3.0, 4.0, 5.0)
	s := Shape.point

	# pattern matching (exhaustive)
	area := match s {
		.circle { radius } => PI * radius * radius,
		.rectangle { width, height } => width * height,
		.triangle(a, b, c) => heron(a, b, c),
		.point => 0.0,
	}

	# specified values
	Status :: enum {
		ok = 200
		not_found = 404
		server_error = 500
	}

	# `ord` gives the discriminant, which defaults to declaration position
	assert!(ord(Color.blue) == 2)
	assert!(ord(Status.not_found) == 404)

	# enums are int-backed by default
	assert!(int.(Status.ok) == 200)

	# backed enums

	Code : u8 : enum { ok = 200, err = 250 }
	codes: [2]Code # 2 bytes
	assert!(u8.(Code.err) == 250)

	# a string backing swaps discriminants for raw values
	# raws default to the variant name and must be unique
	Suit : string : enum {
		hearts = "♥"
		spades = "♠"
		clubs
	}
	# the cast gives the raw, `.str()` gives the variant name
	assert!(string.(Suit.spades) == "♠")
	assert!(string.(Suit.clubs) == "clubs")
	assert!(Suit.spades.str() == "spades")
	# discriminants stay positional
	assert!(ord(Suit.spades) == 1)

	# a backed enum cannot have payload variants

	# ?T and !T are syntax suger for these:
	Option[T] :: enum {
		some(T)
		none
	}
	Result[T, E] :: enum {
		ok(T)
		err(E)
	}

	# first value is default
	c: Color # .red
	s: Shape # .circle { radius = 0.0 }

	# methods

	Color :: enum {
		red
		green
		blue
	}

	Color :< {
		hex :: fn(self) string {
			match self {
				.red => "#ff0000",
				.green => "#00ff00",
				.blue => "#0000ff",
			}
		}

		is_warm :: fn(self) bool {
			self == .red
		}

		# Associated function (no self)
		primary :: fn() Color {
			.red
		}
	}

	# Display is auto-derived for enums, but can be overridden
	Color : Display {
		display :: fn(self) string {
			match self {
				.red => "🔴",
				.green => "🟢",
				.blue => "🔵",
			}
		}
	}

	c := Color.red
	print(c.hex()) # "#ff0000"
	default := Color.primary()

	# enums can be created from string or integer value and converted into string

	Cycle :: enum {
		one
		two = 2
		three
	}

	// create enum from value
	print(Cycle.from(10) or { Cycle.three })
	print(Cycle.from("two")?)
	print(Cycle.from(:three) or .one)

	// convert an enum value to a string
	print(Cycle.one.str())

	# the newlines are optional
	Fruit :: enum { apple orange grape }

	## sum types

	Id :: int | string
	Json :: :null | bool | f64 | string | []Json | [string]Json

	# member values coerce when the type is known from context
	id: Id = 7
	id = "abc123"

	# sum types may be used in type signatures
	lookup :: fn(id: Id) User | :missing { :missing }

	# nested sum aliases splice in place
	Num :: int | f64
	Value :: Num | string # = int | f64 | string

	# matching is exhaustive
	describe :: fn(id: Id) string {
		match id {
			n @ int => "numeric: {n}",
			s @ string => "named: {s}",
		}
	}

	# members must be distinct after alias resolution
	# Bad :: int | int # error: duplicate member

	# the zero value is the first member's zero
	blank: Id # 0

	# sum types are interchangeable
	# NOTE: except for containers like []Id and []Handle
	Handle :: string | int
	h Handle := blank

	# member order is defined by each type, which determines zero values
	new: Id
	fresh: Handle
	assert!(new != fresh)
	assert!(new == 0)
	assert!(fresh == "")

	# atoms make great sum types
	Status :: :ok | :err

	# to make distinct types, wrap sum type in tuple structs
	UserId :: struct (int | string)

	# `any` is the open type
	x: any = 5
	x = "five"
	describe :: fn(v: any) string {
		match v {
			n @ int => "int {n}",
			s @ string => "string {s}",
			else => "something else", # required, `any` is open
		}
	}
	# the zero value holds nothing and only matches `else`
	nothing: any

	## errors

	# built-in Error trait
	trait Error {
		message :: fn(self) string
		cause :: fn(self) ?Error { none }
	}

	# `!T` means: `T` or some value implementing `Error`
	read_config :: fn() !Config { ... }

	# crash out
	if false {
		assert!(true, "optional message")
		panic!("uh oh...")
	}

	## blocks

	# blocks are groups of expressions
	# the final expression is the block's value
	three := {
		light_the_beacons()
		3
	}

	# `;` joins lines
	long_but_short := { do_thing(); 3 }

	# blocks are eager and run in place
	# they can fully read and mutate the enclosing scope
	{ x }

	## fn literals

	#{
		Oi has only one way to make a function, which is a fn literal.
		`name :: func` is a named fucntion, a fill in a type body is a method.
		The syntax scales from tiny closures to fully typed, explicitly captured functions.
		```
		fn [captures]? (params)? ret? { body }
		```
		- captures: optional capture spec
			- omitted: fn implicitly captures any enclosing locals it references as read-only borrows
			- []: non-capturing. borrows nothing, so it stores, returns, and escapes freely.
				can still call named functions and read module-level consts/types
			- [x]: captures `x` as a read-only borrow, and nothing else
			- [mut x, y]: captures `x` as an exclusive scoped borrow, `y` read-only, and nothing else
			- [move x]: moves `x` into the fn (using `x` after is an error), and nothing else
			- any bracket (empty or populated) turns implicit capture off, and all referenced locals must then be listed explicitly
			- a fn holding any borrowed capture cannot escape the enclosing scope (unless it `move`s)
		- params: optional param spec
		- ret: optional return spec
	}#

	# a `::` binding is visible in its own body
	fac :: fn(n: int) int { if n < 2 { 1 } else { n * fac(n - 1) } }

	# implicit capture
	n := 10
	scale := fn (x: int) int { x * n }

	# non-capturing
	# NOTE: This does not mean pure. See [pure functions](#pure-functions)).
	mul := fn [] (x: int, y: int) int { x * y }
	nums.map(fn [] { $ * 2 })

	# explicit read-only capture
	factor := 3
	triple := fn [factor] (x: int) int { x * factor }

	# explicit mutable capture
	counter := 0
	increment :: fn [mut counter] (x: int) int {
		counter += x
		counter
	}

	# move capture
	spawn(fn [move data] { process(data) })

	## trailing functions

	# if a function is the last argument of a call, it may be written after the parens
	retry(3) fn {
		fetch(url)?
	}

	# if no named params are needed, the `fn` may be omitted (`$` is still accessible)
	retry(3) {
		fetch(url)?
	}

	# if the trailing function is the only argument, the parens may be omitted too
	spawn {
		do_work()
	}
	mutex.with { do_work() }

	# composed with leading literals, function calls may be written like this:
	# retry(3, fn { ... })
	retry 3 {
		fetch(url)?
	}
	timeout 5.sec {
		slow_call()
	}

	# like with normal functions, `$` is the input passed to the anonymous function
	db.transaction {
		$.insert(user)
		$.insert(order)
	}

	# a fn header may be provided to name the params
	db.transaction fn(tx) {
		tx.insert(user)
		tx.insert(order)
	}

	## blocks where fns are expected

	# a bare block is a function literal against any expected function type
	double : fn(int) int : { $ * 2 }

	# a literal may omit whatever the expected type supplies: param types, return, or the entire header.
	# names in a fn type are purely optional documentation and never bind
	Handler :: fn(Event) Event
	onclick : Handler : fn(ev) { log(ev); ev }

	# all the same function, because of a couple different composable rules
	onclick :: fn(ev: Event) Event { log(ev); ev }
	onclick : fn(ev: Event) Event : { log($); $ }
	onclick : fn(Event) Event : fn(ev) { log(ev); ev }
	onclick : Handler : fn(ev) { log(ev); ev }

	# `= {}` makes a reassignable fn variable
	on_click : fn(Event) = { log($) }

	## misc.

	# defer

	# defer takes an expression
	f := os.create("out.log")?
	defer f.close()

	# blocks are expressions too
	defer {
		print("closing file")
		f.close()
	}

	# defer gets the return values if relevant
	do_stuff :: fn() bool {
		defer {
			if !$ {
				print("uh oh...")
			}
		}
		if os.env("DEBUG") { return false }
		return true
	}

	# defer/err only runs if an error was raised
	defer/err eprint()

	# defers in loops run at the end of each iteration
	loop {
		defer print("here we go again...")
		do_stuff()
	}

	## pipelines

	call_to_action := "let's do this" |> trim |> upper

	# if any step returns none, the whole chain is none
	"optional-aware" |> upper?
	nickname := find_user(id)
		|> get_profile?
		|> get_display_name?
		or "anonymous"

	# any error short circuits
	"result-aware" |> upper?
	result := input |> trim |> upper |> save?

	# Each step gets the piped value as `$`.
	# A bare fn (ex: `trim`) is ran with the input as the first param (`trim` == `trim($)`).
	# Any other expression (a call using `$`, an `if`, a block) # is evaluated in place with `$` bound.
	# This lets us do clojure-like threading.
	"threading"
		|> wrap("[", $, "]")
		or log_errors("foo", $)
	"hello" |> $ + " world"
	[2 4 6 8] |> if $.len > 0 { print(true) }

	# any errors in the pipeline flow directly to an `or`
	"error-only pipes"
		|> upper
		or handler

	# any expression can be used as a pipeline step, including blocks
	# for convenience, in blocks `$` is bound to the passed-in params as if they were a function
	result := "error-only pipes with block"
		|> {
			idk($)
		}
		|> {
			log.info("stuff and things: {$}")
			:block_done
		}
		|> fn {
			assert!($ == true)
			log.info("this is an _actual_ function")
			:fn_done
		}
		or {
			eprint($)
			return $
		}
	assert!(result == :fn_done)
	config := os.env("config_path")
		|> read_file?
		|> parse?
		or {
			log.warn("Config load failed: {$}. Using default.")
			default_config()
		}
	"gtfo" |> process or { panic!("uh oh...") }
	"err binding" |> raise_err |> or { log.error($) }

	# you can specify params
	# to a name when nesting to avoid ambiguity
	"foo" |> fn (outer) {
		outer |> fn (inner) {
			log.debug("inner: {inner}, outer: {outer}")
		}
		assert!(outer == $)
	}

	# or you can cache the `$`
	"foo" |> {
		outer := $
		outer |> {
			inner := $
			log.debug("inner: {inner}, outer: {outer}")
		}
		assert!(outer == $)
	}

	# all together now (all together now!)
	result := data
		|> validate
		|> transform(4, $.name)?
		|> filter($ > 0)
		|> send?
		|> wrap("[", $, "]")
		|> {
			log.info("saving {$}...")
			save($)?
		}
		or log

	formatted := name
		|> upper
		|> wrap("[", $, "]")
		|> log(level = :info, $)

	# pipeline functions

	# a pipeline is a composition of stages

	# a fn head composes the stages into a single fn
	slugify :: trim |> lower |> replace(" ", "-")
	assert!(slugify(" Hello World ") == "hello-world")

	# a non-fn head immediately applies the stages to the input
	" foo-bar " |> trim |> upper |> replace("-", "_") # "FOO_BAR"

	# fn literals chain by the same rule
	f :: fn(x: int) (int, int) { (x, x) } |> fn(x: int, y: int) Point { Point.{ x, y } }
	assert!(f(2) == Point.{ 2, 2 })

	# compositions are expressions
	nums.map(double |> negate)

	# annotations may be provided when desired, as with any other binding
	slugify : fn(string) string : trim |> lower |> replace(" ", "-")

	# when a stage needs a parameter by name, write the fn out and pipe from the param
	# TODO: revisit, because I can see ways to make this more ergonomic like it was originally
	count_letters :: fn(s: string) int {
		s |> lower |> uniq |> replace("[^A-Za-z]", "") |> len |> {
			log.info("called count_letters with {s}, and it has {$} unique letters")
			$
		}
	}
	assert!(count_letters("hi, mom!") == 4)

	## metaprogramming

	# compile-time eval with comp

	# takes any expression
	PI :: comp 22.0 / 7.0
	VERSION :: comp git.current_sha()

	# including if and match expressions
	PLATFORM_DEFAULT :: comp if BUILD_OS == :windows { "\\r\\n" } else { "\\n" }

	# embedded resources
	image :: comp fs.read_bytes("assets/cats.png")?
	shader :: comp fs.read("shaders/urmom.glsl")?

	# or block expressions
	VERSION_INFO :: comp {
		sha := git.head_sha()
		branch := git.current_branch()
		"{branch}@{sha[0..7]}"
	}
	CONFIG :: comp {
		raw := fs.read("build.toml")?
		toml.parse(raw)?
	}
	comp {
		# comptime assertions
		assert!(max_connections > 0 && max_connections <= 65535)
	}

	# function calls can have comptime args
	open_typed :: fn(comp T: type, path: string) !T {
		raw := open(path)?
		deserialize(T, raw)
	}

	# generics are sugar for comp type params
	first[T] :: fn(xs []T) ?T {
		if xs.len == 0 { none } else { Some(xs[0]) }
	}
	# generics can have trait guards
	max[T Ord] :: fn(a: T, b: T) T {
		if a > b { a } else { b }
	}
	max :: fn(comp T: type, a: T, b: T) T where T is Ord { ... }

	## annotations

	# an annotation is a comptime known value attached with @
	# they don't do anything on their own but may be read back through reflection

	# atoms make great tags
	@:awesome
	kickflip :: fn() {}
	Foo :: struct { bar: int @:awesome }

	# structs may be used to associate payloads with annotations
	deprecated :: struct { reason: string }
	@deprecated.{"use speak()"}
	pub yell :: fn() string { ... }

	# a struct type in annotation position denotes its zero value (`@foo` -> `@foo.{}`)
	@deprecated
	pub bellow :: fn() string { ... }

	# comptime values may be used too
	strict :: deprecated.{"use speak()"}
	@strict
	pub shout :: fn() string { ... }

	# blessed builtins are just consts in core
	# pub required :: ()
	Player :: struct { name: string @required }

	# annotations attach to definitions and struct fields
	GetProc :: @c fn(name: cstr) ptr

	# `test! "name" { ... }` is the usual way to write a test
	# it expands to an `@test` fn
	test! "trims edges" { assert! trim(" hi ") == "hi" }

	# `@test` is the underlying annotation, needed for fields `test!` doesn't expose (like `skip`)
	# they get stripped from normal builds, and are run by `oi test`
	# pub test :: struct { name: string = "", skip: bool = false }
	@test trims_edges :: fn() { assert! trim(" hi ") == "hi" }
	@test.{"trims nothing", skip = true}
	trims_empty :: fn() { assert! trim("") == "" }

	# TODO: the comp and reflection stuff not fleshed out yet
	comp for note in typeinfo(yell).annotations { ... }

	## macros

	# a macro is a comptime fn over `Ast`, defined with `name! :: fn`
	# macros end in a !, which suspends normal parsing and collects the body
	# args arrive as parsed `Ast` by default

	# quasi-quote evals to `Ast`
	# paired backticks around valid Oi
	q := `2 + 2`

	# %name unquotes a comptime value, %{expr} unquotes an expression
	n := 2
	doubled := `%n + %{compute_rhs()}`

	# %{..xs} spreads an []Ast across a sequence position (call args, array elems, statements, match arms)
	args := [`1`, `2`, `3`]
	ast := `sum(%{..args})`

	# quotes are bidirectional
	match expr {
		`foo(%x, %y)` => swap_args(x, y),
		_ => expr,
	}

	derive! :: fn(input: Ast, traits: Ast) Ast {
		# input is the parsed struct, traits is the list passed to @derive!()
		name := input.type_name()
		fields := input.struct_fields()
		impls := traits.items().map(fn (t) {
			match t.name() {
				"Hash" => {
					hashes := fields.map(fn (f) { `self.%f.hash()` })
					`%name : Hash {
						hash :: fn(self) int { %{hashes.reduce(fn (a, b) { `%a * 31 + %b` })} }
					}`
				}
				"Debug" => `%name : Debug { ... }`,
			}
		})
		`%{..impls}`
	}

	# `@name!` runs a macro on the following expression
	@derive!(Hash, Debug)
	Point :: struct { x: int, y: int }

	# inline calls
	dbg!(count_letters("hi, mom!"))

	# paren-less statement form. the argument runs to end of statement, parenthesize to compose
	assert! foo.bar() == 5

	# a Tokens param opts a macro into the raw stream, for embedded DSLs whose bodies aren't valid Oi
	sql! :: fn(body: Tokens) Ast { ... }
	sql! { SELECT * FROM users WHERE id = {id} }

	# reflection in `comp`
	debug_print[T] :: fn(value: T) {
		comp for field in type_info(T).fields {
			println("{field.name} = {value.(field.name)}")
		}
	}

	# conditional compilation
	log :: fn(msg: string) {
		comp if BUILD_MODE == :debug {
			eprintln(msg)
		}
	}
}

## std

print[T: Display] :: fn(value: T)

# these are plain fns, interpolation happens in the lexer
print(value) # stdout, with newline
write(value) # stdout, no newline
eprint(value) # stderr, with newline
ewrite(value) # stderr, no newline

# these need source text or the AST, so they are macros
dbg! :: fn(expr)
assert! :: fn(expr, msg: string?)
panic! :: fn(msg: string?)
todo! :: fn()
unreachable! :: fn(msg: string?)

## stdlib
```
