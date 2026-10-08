use crate::common::{Project, Run, err, oi, ok};

#[test]
fn imports_work() {
	let p = Project::new()
		.main(["use foo", "print(foo.total())"])
		.lib(
			"foo",
			[
				"use bar",
				"P :: struct { x: int, y: int }",
				"sum :: fn(p: P) int { p.x + p.y }",
				"pub total :: fn() int { bar.twice(sum(P.{x = 2, y = 5})) }",
			],
		)
		.lib("bar", ["pub twice :: fn(n: int) int { n * 2 }"]);
	p.check("14");
}

#[test]
fn private_fn_rejected() {
	let p = Project::new()
		.main(["use foo", "print(foo.secret())"])
		.lib("foo", ["secret :: fn() int { 1 }"]);
	p.fail_with("private to module `foo`");
}

#[test]
fn module_headers_must_agree() {
	let p = Project::new()
		.main(["use foo", "print(foo.hi())"])
		.file("foo/a.oi", ["module foo", "pub hi :: fn() int { 1 }"])
		.file("foo/b.oi", ["module bar", "pub yo :: fn() int { 2 }"]);
	let out = err(p.run());
	assert!(out.contains("foo/b.oi"), "{out}");
	assert!(out.contains("disagree"), "{out}");
}

#[test]
fn headerless_module_and_entry_run() {
	let p = Project::new()
		.file("main.oi", ["module app", "use foo", "print(foo.hi())"])
		.file("foo/lib.oi", ["module bar", "pub hi :: fn() int { 1 }"]);
	p.check("1");
	let p = Project::new()
		.file("main.oi", ["use foo", "print(foo.hi())"])
		.file("foo/lib.oi", ["pub hi :: fn() int { 1 }"]);
	p.check("1");
}

#[test]
fn import_cycle_rejected() {
	let p = Project::new()
		.main(["use a", "print(a.v())"])
		.file("a/m.oi", ["module a", "use b", "pub v :: fn() int { b.v() }"])
		.file("b/m.oi", ["module b", "use a", "pub v :: fn() int { a.v() }"]);
	p.fail_with("import cycle");
}

#[test]
fn duplicate_name_across_files_rejected() {
	let p = Project::new()
		.main(["use foo", "print(foo.hi())"])
		.file("foo/a.oi", ["module foo", "pub hi :: fn() int { 1 }"])
		.file("foo/b.oi", ["module foo", "hi :: fn() int { 2 }"]);
	p.fail_with("defined twice");
}

#[test]
fn imports_are_per_file() {
	let p = Project::new()
		.main(["use foo", "print(foo.hi())"])
		.file(
			"foo/a.oi",
			["module foo", "use bar.two", "pub hi :: fn() int { two() + one() }"],
		)
		.file("foo/b.oi", ["module foo", "one :: fn() int { two() - 1 }"])
		.lib("bar", ["pub two :: fn() int { 2 }"]);
	p.fail_with("undefined function `two`");
}

#[test]
fn import_alias() {
	let p = Project::new()
		.main(["f :: use foo", "print(f.hi())"])
		.lib("foo", ["pub hi :: fn() int { 7 }"]);
	p.check("7");

	let p = Project::new()
		.main(["f :: use foo", "print(foo.hi())"])
		.lib("foo", ["pub hi :: fn() int { 7 }"]);
	p.fail_with("foo");
}

#[test]
fn selective_import() {
	for main in [
		"use foo\nuse foo.{ hi }\nprint(hi() + foo.hi())",
		"use foo.hi\nprint(hi() + hi())",
		"use foo.{ h :: hi }\nprint(h() + h())",
	] {
		let p = Project::new().main([main]).lib("foo", ["pub hi :: fn() int { 7 }"]);
		p.check("14");
	}
}

#[test]
fn selective_import_fails() {
	let p = Project::new()
		.main(["use foo.{ nope }", "print(nope())"])
		.lib("foo", ["pub hi :: fn() int { 7 }"]);
	p.fail_with("has no `nope`");

	for lib in [
		"P :: fn() int { 1 }",
		"P :: struct { x: int }",
		"P :: trait {}",
		"P :: 7",
	] {
		let p = Project::new().main(["use foo.{ P }"]).lib("foo", [lib]);
		p.fail_with("private to module `foo`");
	}
}

#[test]
fn type_import() {
	for (main, lib) in [
		(
			"use foo.{ P }\np := P.{ x = 3, y = 4 }\nprint(p.x + p.y)",
			"pub P :: struct { pub x: int, pub y: int }",
		),
		(
			"Q :: use foo.P\nsum :: fn(q: Q) int { q.x + q.y }\nprint(sum(Q.{ x = 3, y = 4 }))",
			"pub P :: struct { pub x: int, pub y: int }",
		),
		(
			"use foo.{ E }\ne := E.a\nprint(match e { E.a => 7, E.b => 0 })",
			"pub E :: enum { a, b }",
		),
		("use foo.{ Id }\nn: Id = 7\nprint(n)", "pub Id :: int"),
		("use foo.{ id }\nn: id = 7\nprint(n)", "pub id :: int"),
	] {
		let p = Project::new().main([main]).lib("foo", [lib]);
		p.check("7");
	}
}

#[test]
fn member_visibility() {
	for (main, lib) in [
		// a private field read
		("use foo.{ make }\nprint(make().x)", "pub P :: struct { x: int }"),
		// a private field in a literal
		(
			"use foo.{ P }\nprint(P.{ x = 1 }.y)",
			"pub P :: struct { x: int, pub y: int }",
		),
		// a private field in a positional literal
		(
			"use foo.{ P }\nprint(P.{ 1, 2 }.y)",
			"pub P :: struct { x: int, pub y: int }",
		),
		// a private field in a match pattern
		(
			"use foo.{ P, make }\nmatch make() { P.{ x } => print(x), }",
			"pub P :: struct { x: int }",
		),
		// a private `:<` method call
		(
			"use foo.{ make }\nprint(make().hidden())",
			"pub P :: struct { x: int }\nP :< { hidden :: fn(self) int { 1 } }",
		),
	] {
		let p = Project::new()
			.main([main])
			.lib("foo", [lib, "pub make :: fn() P { P.{ x = 1 } }"]);
		p.fail_with("private to module `foo`");
	}

	let p = Project::new().main(["use foo.{ P }", "print(P.{ x = 3 }.shown())"]).lib(
		"foo",
		[
			"pub P :: struct { pub x: int }",
			"P :< { pub shown :: fn(self) int { self.x * 2 } }",
		],
	);
	p.check("6");

	let p = Project::new()
		.main(["use foo.{ P, make }", "match make() { P.{ x } => print(x), }"])
		.lib(
			"foo",
			["pub P :: struct { pub x: int }", "pub make :: fn() P { P.{ x = 9 } }"],
		);
	p.check("9");
}

#[test]
fn package_visibility() {
	let home = Project::new()
		.file(
			"lib/greet/lib.oi",
			["module greet", "use util", "pub hi :: fn() int { util.inner() }"],
		)
		.file(
			"lib/util/lib.oi",
			["module util", "pub(package) inner :: fn() int { 42 }"],
		);
	let entry = Project::new();
	let exec = |key: &str, dir: &std::path::Path| {
		oi(&["exec", "use greet\nprint(greet.hi())"])
			.current_dir(&entry)
			.env(key, dir)
			.run(None)
	};
	// one package on OI_PATH, two once installed
	assert_eq!(ok(exec("OI_PATH", &home.as_ref().join("lib"))), "42");
	assert!(err(exec("OI_HOME", home.as_ref())).contains("private to module `util`"));
}

#[test]
fn traits_are_module_scoped() {
	let p = Project::new()
		.main([
			"use foo.{ FooP :: P, FooT :: T }",
			"use bar.{ BarP :: P, BarT :: T }",
			"print(FooP is FooT, BarP is BarT)",
		])
		.lib("foo", ["pub T :: trait {}", "pub P :: struct { x: int }", "P : T < {}"])
		.lib("bar", ["pub T :: trait {}", "pub P :: struct { y: int }", "P : T < {}"]);
	p.check("true true");
}

#[test]
fn std_trait_claimed_without_import_in_module() {
	let p = Project::new()
		.main(["use foo.{ P }", "print((P.{ x = 2 } + P.{ x = 3 }).x)"])
		.lib(
			"foo",
			[
				"pub P :: struct { pub x: int }",
				"P : Add < { add :: fn(self, other: P) P { P.{ x = self.x + other.x } } }",
			],
		);
	p.check("5");
}

#[test]
fn generic_fn_uses_local_type() {
	let p = Project::new().main(["use foo", "print(foo.pack(7))"]).lib(
		"foo",
		[
			"P :: struct { x: int }",
			"pub pack[T] :: fn(v: T) int { P.{ x = 3 }.x + v }",
		],
	);
	p.check("10");
}

#[test]
fn generic_struct_in_module() {
	let p = Project::new().main(["use foo", "print(foo.mk().v)"]).lib(
		"foo",
		[
			"pub Box[T] :: struct { pub v: T }",
			"pub mk :: fn() Box[int] { Box.{ v = 7 } }",
		],
	);
	p.check("7");
}

#[test]
fn type_and_const_reexport() {
	let p = Project::new()
		.main(["use mid.{ P, name }", "print(P.{ x = name }.x)"])
		.lib("mid", ["pub use base.P", "pub use base.name"])
		.lib("base", ["pub P :: struct { pub x: int }", "pub name :: 7"]);
	p.check("7");
}

#[test]
fn narrowed_import() {
	for main in [
		"io :: use foo.{ hi }\nprint(io.hi())",
		"io :: use foo.{ h :: hi }\nprint(io.h())",
	] {
		let p = Project::new().main([main]).lib("foo", ["pub hi :: fn() int { 7 }"]);
		p.check("7");
	}
}

#[test]
fn narrowed_import_fails() {
	for (main, expected) in [
		("io :: use foo.{ hi }\nprint(io.yo())", "not part of"),
		("io :: use foo.{ nope }\nprint(io.nope())", "has no `nope`"),
	] {
		let p = Project::new()
			.main([main])
			.lib("foo", ["pub hi :: fn() int { 7 }", "pub yo :: fn() int { 8 }"]);
		p.fail_with(expected);
	}
}

#[test]
fn reexport() {
	for (mid, call) in [
		("pub use base.hi", "mid.hi()"),
		("pub use base.{ hi }", "mid.hi()"),
		("pub use base.{ h :: hi }", "mid.h()"),
	] {
		let p = Project::new()
			.main(["use mid", &format!("print({call})")])
			.lib("mid", [mid])
			.lib("base", ["pub hi :: fn() int { 7 }"]);
		p.check("7");
	}
}

#[test]
fn reexport_chain() {
	let p = Project::new()
		.main(["use top.{ hi }", "print(hi())"])
		.lib("top", ["pub use mid.hi"])
		.lib("mid", ["pub use base.hi"])
		.lib("base", ["pub hi :: fn() int { 7 }"]);
	p.check("7");
}

#[test]
fn reexport_module() {
	for (top, call) in [
		("pub use base", "top.base.hi()"),
		("pub b :: use base.{ hi }", "top.b.hi()"),
		("pub use mid", "top.mid.base.hi()"),
	] {
		let p = Project::new()
			.main(["use top", &format!("print({call})")])
			.lib("top", [top])
			.lib("mid", ["pub use base"])
			.lib("base", ["pub hi :: fn() int { 7 }", "pub bye :: fn() int { 8 }"]);
		p.check("7");
	}
	let p = Project::new()
		.main(["use top", "print(top.b.bye())"])
		.lib("top", ["pub b :: use base.{ hi }"])
		.lib("base", ["pub hi :: fn() int { 7 }", "pub bye :: fn() int { 8 }"]);
	p.fail_with("is not part of");
}

#[test]
fn reexport_fails() {
	for (mid, expected) in [
		("use base.hi", "has no function `hi`"),
		("pub(package) use base", "only `pub` imports"),
	] {
		let p = Project::new()
			.main(["use mid", "print(mid.hi())"])
			.lib("mid", [mid])
			.lib("base", ["pub hi :: fn() int { 7 }"]);
		p.fail_with(expected);
	}
}

#[test]
fn const_import() {
	for main in [
		"use foo\nprint(foo.name)",
		"use foo.{ name }\nprint(name)",
		"use foo.name\nprint(name)",
	] {
		let p = Project::new().main([main]).lib("foo", ["pub name :: 7"]);
		p.check("7");
	}
}

#[test]
fn module_cannot_call_main_private_fn() {
	let p = Project::new()
		.main(["secret :: fn() int { 1 }", "use foo", "print(foo.hi())"])
		.lib("foo", ["pub hi :: fn() int { secret() }"]);
	p.fail_with("undefined function");
}

#[test]
fn module_fn_uses_builtins_and_prints() {
	let p = Project::new()
		.main(["use foo", "foo.go()"])
		.lib("foo", ["pub go :: fn() { n: int = 3\nprint(n + 1) }"]);
	p.check("4");
}

#[test]
fn dir_wins_over_single_file_module() {
	let p = Project::new()
		.main(["use foo", "print(foo.hi())"])
		.file("foo.oi", ["module foo", "pub hi :: fn() int { 1 }"])
		.lib("foo", ["pub hi :: fn() int { 2 }"]);
	p.check("2");
}

#[test]
fn core_root_beside_local_module() {
	let p = Project::new()
		.main([
			"use math",
			"cm :: use core.math",
			"print(math.hi() + cm.abs(-3) + core.math.max(1, 2))",
		])
		.lib("math", ["pub hi :: fn() int { 7 }"]);
	p.check("12");

	let p = Project::new()
		.main(["use math", "print(math.abs(-3))"])
		.lib("math", ["pub hi :: fn() int { 7 }"]);
	p.fail_with("local `math` shadows `core.math`");
}

#[test]
fn subdir_is_a_submodule() {
	for main in [
		"use foo\nprint(foo.api.gen() + foo.hi())",
		"use foo.api.gen\nprint(gen() + 1)",
	] {
		let p = Project::new()
			.main([main])
			.lib("foo", ["pub hi :: fn() int { 1 }"])
			.file("foo/api/gen.oi", ["module api", "pub gen :: fn() int { 7 }"]);
		p.check("8");
	}
}

#[test]
fn single_file_module_chains_import() {
	let p = Project::new()
		.main(["use foo", "print(foo.hi())"])
		.file("foo.oi", ["module foo", "use bar", "pub hi :: fn() int { bar.hi() }"])
		.file("bar.oi", ["module bar", "pub hi :: fn() int { 7 }"]);
	p.check("7");
}

#[test]
fn exec_resolves_imports_against_cwd() {
	let p = Project::new().lib("foo", ["pub hi :: fn() int { 99 }"]);
	let out = p.ok(&["exec", "use foo\nprint(foo.hi())"]);
	assert_eq!(out, "99");
}
