+++
title = "Oi!"
description = "Your friendly neighborhood programming language."
template = "homepage.html"
+++

A language written by human([s?](https://github.com/oilang/oi/fork)), for humans.
More specifically, for humans who love programming.

Oi is a general purpose system language with a high emphasis on ergonomics.

It was designed such that the code you want to write is usually the code you actually write.
Where other languages optimize for things like safety, perf, or simplicity, Oi optimizes for [flow](https://en.wikipedia.org/wiki/Flow_(psychology)).

Its features [try to] encourage uninterrupted thought:
- expression oriented
- implicit returns
- leading literals
- trailing functions
- trailing records
- pipelines
- named results
- zero values
- destructuring
- error handling

# examples

```odin,linenos,name=examples/shapes.oi
Shape :: enum {
	point
	circle { radius: f64 }
	rect { w: float, h: float }
	triangle(f64, f64, f64)
}

area :: fn(s: Shape) f64 {
	match s {
		.circle.(r) => 3.14159 * r ** 2,
		.rect.(w, h) => w * h,
		else => 0.0,
	}
}

Shape.rect.{ 3.0, 4.0 }
	|> area
	|> assert!($ == 12.0)

shape := Shape.triangle.(3.0, 4.0, 5.0)

match shape {
	.point => {
		print("origin: {()}")
	}
	.circle.(r) => {
		print("circle: {(r,)}")
	}
	.rect.(w, h) => {
		print("rect: {(w h)}")
	}
	t @ .triangle.(a, b, c) => {
		print(t)
	}
}
```
