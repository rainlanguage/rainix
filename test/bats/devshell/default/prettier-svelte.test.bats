@test "prettier hook preserves TypeScript enum bodies in Svelte files when consumer pins prettier" {
	tmpdir="$(mktemp -d)"

	cat > "$tmpdir/package.json" <<'PJSON'
{ "name": "fixture", "version": "0.0.0", "private": true }
PJSON

	# Mirrors the cyclo.site .prettierrc: the bug requires BOTH the svelte
	# plugin and the tailwindcss plugin to be loaded — the tailwindcss
	# plugin's wrapping of the formatter, combined with nixpkgs prettier
	# 3.6.2, is what corrupts the embedded TS in `<script lang="ts">`.
	cat > "$tmpdir/.prettierrc" <<'PCFG'
{
	"plugins": ["prettier-plugin-svelte", "prettier-plugin-tailwindcss"],
	"overrides": [{ "files": "*.svelte", "options": { "parser": "svelte" } }]
}
PCFG

	cat > "$tmpdir/Lock.svelte" <<'SVELTE'
<script lang="ts">
	enum ButtonStatus {
		READY = 'LOCK'
	}
</script>
SVELTE

	cd "$tmpdir"
	npm install --no-save --silent --no-audit --no-fund \
		prettier@3.1.1 \
		prettier-plugin-svelte@3.1.2 \
		prettier-plugin-tailwindcss@0.5.14 \
		svelte@4.2.7

	# .pre-commit-config.yaml has leading comment lines added by git-hooks.nix;
	# strip them so jq can parse the JSON body.
	prettier_entry="$(grep -v '^#' "${BATS_TEST_DIRNAME}/../../../../.pre-commit-config.yaml" | jq -r '.repos[0].hooks[] | select(.name == "prettier") | .entry')"

	bash -c "$prettier_entry Lock.svelte"

	actual="$(cat "$tmpdir/Lock.svelte")"

	rm -rf "$tmpdir"

	# The bug strips the enum body, leaving the syntactically invalid
	# `enum ButtonStatus` (no `{ ... }`). Prettier may reformat the
	# enum body's whitespace/quoting, so assert the body opening brace
	# survives rather than byte-equality.
	echo "$actual" | grep -q 'enum ButtonStatus {'
}
