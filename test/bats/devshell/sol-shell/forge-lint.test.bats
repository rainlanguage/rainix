# The `forge lint -D warnings` gate that rainix-sol-static runs over every sol
# consumer. Two properties of that command are load-bearing and neither is
# visible in it, so both are pinned here:
#
#   - `forge lint` on its own ALWAYS exits 0. It prints its findings and
#     succeeds. The subcommand is not the gate; `-D warnings` is.
#   - `-D` is a compiler flag, not a lint flag. It denies solc diagnostics as
#     well as forge-lint findings, so a repo with zero forge-lint findings
#     still fails the gate on a plain solc warning.
#
# Each case is a throwaway foundry project outside this repo, pinned to the
# sol-shell `solc` so no case reaches the network for a compiler, and invisible
# to the fixture project's own lint/fmt/reuse runs.

setup() {
  TESTDIR="$(mktemp -d)"
  mkdir -p "$TESTDIR/src"
  cat > "$TESTDIR/foundry.toml" <<EOF
[profile.default]
src = "src"
out = "out"
solc = "$(command -v solc)"
EOF
}

teardown() {
  rm -rf "$TESTDIR"
}

# Reads the subject contract from stdin so each case carries its own source.
subject() {
  cat > "$TESTDIR/src/Subject.sol"
}

@test "forge lint -D warnings passes on source with no finding and no solc warning" {
  subject <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity ^0.8.25;

contract Subject {
    uint256 internal counter;

    function increment() external {
        counter += 1;
    }
}
EOF
  cd "$TESTDIR"
  run forge lint -D warnings
  [ "$status" -eq 0 ]
}

@test "forge lint -D warnings fails on a forge-lint finding" {
  subject <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity ^0.8.25;

contract Subject {
    function narrow(uint256 value) external pure returns (uint128) {
        return uint128(value);
    }
}
EOF
  cd "$TESTDIR"
  run forge lint -D warnings
  [ "$status" -ne 0 ]
  echo "$output" | grep -q 'warning\[unsafe-typecast\]'
}

# The reason the gate is `-D warnings` and not `forge lint`: the same source
# that fails above exits 0 without the flag, so a bare `forge lint` step would
# be a no-op gate that prints and passes.
@test "forge lint without -D warnings exits 0 on that same finding" {
  subject <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity ^0.8.25;

contract Subject {
    function narrow(uint256 value) external pure returns (uint128) {
        return uint128(value);
    }
}
EOF
  cd "$TESTDIR"
  run forge lint
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'warning\[unsafe-typecast\]'
}

# The escape hatch a consumer needs when the flagged construct is the thing
# under test rather than a mistake.
@test "a scoped forge-lint suppression clears the gate" {
  subject <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity ^0.8.25;

contract Subject {
    function narrow(uint256 value) external pure returns (uint128) {
        // forge-lint: disable-next-line(unsafe-typecast)
        return uint128(value);
    }
}
EOF
  cd "$TESTDIR"
  run forge lint -D warnings
  [ "$status" -eq 0 ]
}

# A forge-lint finding is re-derived from source on every run, so the gate
# catches it whether or not the compile is skipped. This is what makes the step
# worth having after `slither .`, which has already compiled the tree.
@test "forge lint -D warnings still fails on a finding when the compile is skipped" {
  subject <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity ^0.8.25;

contract Subject {
    function narrow(uint256 value) external pure returns (uint128) {
        return uint128(value);
    }
}
EOF
  cd "$TESTDIR"
  run forge lint
  [ "$status" -eq 0 ]
  run forge lint -D warnings
  echo "$output" | grep -q 'compilation skipped'
  [ "$status" -ne 0 ]
  echo "$output" | grep -q 'warning\[unsafe-typecast\]'
}

# `-D warnings` is a compiler flag, so on a cold tree the gate is wider than
# forge-lint: this source has no forge-lint finding at all and still fails on
# solc's own diagnostic. Consumers that are forge-lint clean can be red here.
@test "forge lint -D warnings fails on a solc warning with no forge-lint finding" {
  subject <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity ^0.8.25;

contract Subject {
    function constantOne() external view returns (uint256) {
        return 1;
    }
}
EOF
  cd "$TESTDIR"
  run forge lint -D warnings
  [ "$status" -ne 0 ]
  echo "$output" | grep -q 'Function state mutability can be restricted to pure'

  run forge lint
  [ "$status" -eq 0 ]
  if echo "$output" | grep -q '^warning\['; then
    echo "FAIL: expected no forge-lint finding on solc-warning-only source" >&2
    echo "$output" >&2
    return 1
  fi
}

# The asymmetry between the two halves of `-D warnings`, pinned because it
# decides what the CI step can be relied on for. solc re-emits a diagnostic
# only when it actually compiles, so a warm `cache`/`out` — which the job
# restores, and which `slither .` fills in anyway — silences the solc half
# while leaving the forge-lint half intact. `--force` recompiles and brings it
# back, at the price of a full rebuild.
@test "a skipped compile silences the solc half of the gate, --force restores it" {
  subject <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity ^0.8.25;

contract Subject {
    function constantOne() external view returns (uint256) {
        return 1;
    }
}
EOF
  cd "$TESTDIR"
  run forge lint
  [ "$status" -eq 0 ]

  run forge lint -D warnings
  [ "$status" -eq 0 ]

  run forge lint --force -D warnings
  [ "$status" -ne 0 ]
  echo "$output" | grep -q 'Function state mutability can be restricted to pure'
}
