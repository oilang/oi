# Oi

A language written by human([s?](https://github.com/oilang/oi/fork)), for humans.
More specifically, for humans who love programming.

Oi is a general purpose system language with a high emphasis on ergonomics.

It was designed such that the code you want to write is usually the code you actually write.
Where other languages optimize for things like safety, perf, or simplicity, Oi optimizes for [flow](https://en.wikipedia.org/wiki/Flow_(psychology)).

Early and unstable and thar be dragons afoot.
You have been warned.

See `examples/` for working examples, and [`ref/syntax.md`](ref/syntax.md) for more sci-fi theoretical future stuff.

> NOTE: this readme is more about building the compiler itself.
> Probably look at the website or docs or something to see something more universally helpful.

## Contributing

Requires nightly Rust (just cranelift things).

```shell
# build compiler
just build

# run tests
just test

# generate rustdocs
just docs

# build, fmt, test, and lint together, the combo I run most
just check

# preview the website (req: `zola`)
just serve
```

## Usage

```shell
# run a main.oi file in the current dir
oi run

# run a file
oi run examples/main.oi

# execute a script
oi exec '2 + 3'

# interactive REPL
oi repl
```

## Docs

- [oilang.github.io/oi/](https://oilang.github.io/oi/): simple website
- [`ref/syntax.md`](ref/syntax.md): canonical language design spec
- [`ref/Oi.md`](ref/Oi.md): loose plan and direction, open questions
