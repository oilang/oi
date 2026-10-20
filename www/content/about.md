+++
title = "about"
description = "idk what"
+++

# goals & non-goals

Oi is not trying to be:
- the smallest
- the fastest
- the most academically perfect
- the most minimal
- another X replacement

The real goal is a language that is enjoyable to write.

# philosophy


## write what you mean

If a construct is obvious to both the compiler and the reader, Oi tries to let you write it directly.

- implicit returns
- leading literals
- trailing functions
- pattern matching
- named tuples
- minimal punctuation

The language tries to get the gtfo out of your way.

## everything composes

Pipelines, expressions, blocks, traits, generics.
Rather than introducing a one-off syntax for every feature, Oi tries to make small concepts combine nicely together.

```odin
# type composability
Absurd :: ([int]?[]string, []?string, i69)

# ranges + loops
range :: 2..4
loop i in range {
	print(i)
}

# leading literals + trailing functions
test "charlotte's web" {
	assert(charlotte.write() == "some pig")
}
retry 3 {
	fetch(url)?
}
timeout 5 {
	slow_call()?
}

# $ + trailing functions
db.transaction {
	$.insert(user)?
	$.insert(order)?
}

# $ + pipelines
"clojure-like threading" |> wrap("[", $, "]")

# pipelines + trailing functions
result := "spying on pipes"
	|> {
		log.info("stuff and things: {$}")
		$
	}
assert(result == "spying on pipes")
```

## sane defaults

- everything has zero values
- private default
- error handling is required

## practical

Oi borrows great ideas where they fit.

You'll see things shamelessly lifted from some languages you know well, and some you don't.
Hopefully this doesn't come off as just a grab bag build-a-lang workshop.
I think (read: hope) that Oi has its own cohesive narrative, working as more than the sum of its parts.
Some of the more specific plagiarized features ~~(*\***cough**\* `impl` \***cough**\**)~~ will likely get revised/renamed/reworked in the near future, but at least for now it doesn't hide its intent.

## okay, but why?
Over two decades of learning and using some 50+ programming languages, I developed a taste for ergonomics.
I've found that 
> TODO: finish or scrap or something

# influences

- Odin
- V
- Rust
- Jai
- Nushell
- awesome new languages that pop up on [/r/programminglanguages](https://www.reddit.com/r/ProgrammingLanguages/), like [revo](https://github.com/if-not-nil/revo) and [Capy](https://github.com/capy-language/capy)
- to a lesser extent: Nim, Zig, Clojure, Janet, Lua, Julia
