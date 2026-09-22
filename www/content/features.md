+++
title = "features"
+++

# everything is an expression

It is a goal for absolutely everything to be an expression.
Right now most things are.
```odin
# if expressions
status := if score > 100 {
	"absolute legend"
} else {
	"noob"
}

# match expressions
value := match token {
	.number.(n) => n,
	.ident.(_) => 0,
}
```

# bindings

```odin
# immutable
never_change :: "hi there"

# mutable
sound := :loud
sound = :quiet

# fills
Point :: struct { x: float, y: float }
Point :< {
	locate :: fn(self) {
		print("I'm at ({self.x}, {self.y})")
	}
}
```

# pipelines

```odin
# basic
call_to_action :: " let's do this " |> trim |> $ + "!!!" |> upper

# option/result aware
# short-circuits on none/error
nickname := find_user(id)
	|> get_profile?
	|> get_display_name?
	or "anonymous"
```

# error handling

Errors are values, and recovery is explicit.

```odin
user := load_user(id) or {
	return error("unknown user")
}
```

# leading literals

If there's only one literal argument, function parens may be omitted.

```odin
print "lol"
foo "bar"
sleep 1_000
log.group :process
```

# trailing functions

```odin
# if a function is the last argument of a call, it may be written after the parens.
retry(3) fn {
	fetch(url)?
}

# if no named params are needed, the `fn` may be omitted
retry(3) {
	fetch(url)?
}

# if the trailing function is the only argument, the parens may be omitted too
spawn {
	do_work()
}
```

# implicit input

> TODO: rename to something cooler?

`$` is what gets passed to a function.

```odin
print_coord :: fn(x: int, y: int) {
	print($.0, $.1)
}
```

This is especially useful inside pipelines.

```odin
# clojure-like threading
"hello" |> $ + " world"
"goodbye" |> wrap("[", $, "]")
```

# named results

Bindings may be provided to return signatures, creating a mutable zeroed value.

```odin
divmod :: fn(a: int, b: int) out: (int, int) {
	out.0 = a / b
	out.1 = a % b
	return
}
```

# deferred cleanup

Cleanup that runs when a scope exits, in reverse order.

```odin
f := os.create("out.log")!
defer f.close()

defer or {
	log.error($)
}
```

# first-class tests

Functions marked as tests are treated specially.
The are not compiled unless ran with `oi test`.

```odin
test! "division" {
	assert! 8 / 2 == 4
}
```

# comfy tooling

CLI commands you're used to.

```bash
oi run .
oi exec "2 + 5"
oi test
oi build
oi repl
oi install
oi init

# not yet implemented:
oi fmt
oi doc
oi watch
oi lsp
```
