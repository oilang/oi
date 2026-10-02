# generate the examples page from ./examples/
[group("docs")]
examples:
	#!/usr/bin/env bash
	{
		printf '+++\ntitle = "examples"\n+++\n'
		printf '> NOTE: These are pulled from the `examples/` folder in the repo.\n'
		for f in examples/*.oi; do
			printf '\n# %s\n\n```odin,linenos,name=%s\n' "$(basename "$f" .oi)" "$f"
			cat "$f"
			printf '```\n'
		done
	} > www/content/examples.md

# write the oi version for the site
[group("docs")]
@version:
	cargo run --quiet -- --version | cut -d' ' -f2 > www/version.txt
