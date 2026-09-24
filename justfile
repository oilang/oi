mod gen

set positional-arguments

# main oi compiler
[group("oi")]
[default]
@oi *args:
	cargo run --quiet -- "$@"

# various useful checks
[parallel]
check: build && fmt test lint

# used by the CI pipeline
ci: build fmt-check lint test

# build
[group("cargo")]
@build:
	cargo build

# run formatter
[group("cargo")]
@fmt:
	cargo fmt

# run formatter in check-only mode
[group("cargo")]
@fmt-check:
	cargo fmt --check

# run lints
[group("cargo")]
@lint:
	cargo clippy --all-targets --no-deps -- -D warnings

# run tests
[group("cargo")]
@test *args:
	cargo test -q "$@"

# install
[group("cargo")]
@install *args:
	cargo install --path . "$@"

# build rustdocs
[group("cargo")]
[group("docs")]
@docs: gen::examples
	cargo doc --no-deps --verbose

# generate and serve static website
[group("docs")]
@serve *args: gen::examples
	zola --root www serve --interface 0.0.0.0 --base-url / --port 8080 "$@"

# fix fixable things
[group("cargo")]
@fix:
	cargo fix --allow-dirty
	cargo clippy --no-deps --fix --allow-dirty
