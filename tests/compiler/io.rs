use crate::helpers::*;

#[test]
fn prints() {
	check(
		indoc! {r#"
			print("hello")
			print(42, true)
			print("a", "b", "c")
			print()
			write()
			write("hi")
			42
		"#},
		["hello", "42 true", "a b c", "", "hi42"],
	);
}

#[test]
fn stderr() {
	let (stdout, stderr) = run_streams(indoc! {r#"
		eprint("err")
		ewrite("x")
		ewrite()
		eprint()
		42
	"#});
	assert_eq!(stdout, "42");
	assert_eq!(stderr, "err\nx");
}

#[test]
fn connect_unix() {
	let path = std::env::temp_dir().join(format!("oi_unix_{}.sock", std::process::id()));
	let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
	run(&format!(
		"use net\nnet.connect_unix(\"{}\")?.write(\"hi\")",
		path.display()
	));
	let mut text = String::new();
	std::io::Read::read_to_string(&mut listener.accept().unwrap().0, &mut text).unwrap();
	std::fs::remove_file(&path).ok();
	assert_eq!(text, "hi");
}
