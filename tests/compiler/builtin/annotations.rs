use crate::common::Project;
use crate::helpers::*;

#[test]
fn nozero_structs() {
	check(
		indoc! {r#"
			@nozero
			Handle :: struct { fd: int }
			h := Handle.{ fd = 3 }
			print(h.fd)
		"#},
		"3",
	);
	check(
		indoc! {r#"
			@nozero
			Handle :: struct { fd: int }
			Conn :: struct { handle: Handle = Handle.{ fd = 7 } }
			c := Conn.{}
			print(c.handle.fd)
		"#},
		"7",
	);
}

#[test]
fn nozero_failures() {
	fail(
		indoc! {r#"
			@nozero
			Handle :: struct { fd: int }
			h: Handle
		"#},
		"`Handle` has no zero value",
	);
	fail(
		indoc! {r#"
			@nozero
			Handle :: struct { fd: int }
			Conn :: struct { handle: Handle }
			c: Conn
		"#},
		"`Handle` has no zero value",
	);
}

#[test]
fn noinit() {
	let net = indoc! {r#"
		module net
		@noinit
		pub Handle :: struct { pub fd: int }
		pub new :: fn(fd: int) Handle { Handle.{ fd = fd } }
	"#};
	Project::new()
		.file("main.oi", ["use net", "print(net.new(3).fd)"])
		.file("net.oi", net)
		.check("3");
	Project::new()
		.file("main.oi", ["use net.{ Handle }", "print(Handle.{ fd = 3 }.fd)"])
		.file("net.oi", net)
		.fail_with("can't build `Handle` outside module `net`");
}
