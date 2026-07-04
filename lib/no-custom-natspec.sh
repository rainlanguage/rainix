#!/usr/bin/env bash

# Custom NatSpec tags (@custom:) are banned in first-party Solidity: they are
# an unreviewed side-channel that fragments docs (e.g. @custom:error blocks
# duplicating declaration-site error docs). Behavior belongs in standard
# NatSpec (@notice/@dev/@param/@return). The single allowed form is ERC-7201's
# normative, tooling-read storage annotation, matched exactly:
#   @custom:storage-location erc7201:
# Vendor dirs are excluded by ROOT-ANCHORED paths (./dependencies/, ./lib/) so
# first-party src/lib/ or test/lib/ files are still checked.

# Print every offending @custom: line in first-party .sol under $1 (default
# current dir); return 1 if any exist, 0 when clean.
check_no_custom_natspec() {
  local dir="${1:-.}"
  local hits
  hits=$(cd "$dir" && grep -rn --include='*.sol' -E '@custom:' . \
    --exclude-dir=node_modules --exclude-dir=.git \
    | grep -vE '^\./(dependencies|lib)/' \
    | grep -vE '@custom:storage-location erc7201:' || true)
  if [ -n "$hits" ]; then
    echo "Custom NatSpec tags (@custom:) found — use standard NatSpec instead"
    echo "(only ERC-7201's exact '@custom:storage-location erc7201:' form is allowed):"
    echo "$hits"
    return 1
  fi
  echo "no-custom-natspec: clean"
}
