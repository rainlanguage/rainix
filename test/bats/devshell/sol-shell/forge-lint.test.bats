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

# The third load-bearing property, and the one rainix-sol-static's caching
# decision rests on: `forge lint` compiles for ASTs only. It writes an artifact
# per contract holding `abi` and `id` and NO `bytecode`, while recording that
# artifact in `cache/solidity-files-cache.json` as compiled — foundry's dirty
# check asks whether the artifact file exists, not whether it has bytecode. A
# later `forge build` handed that pair reports "No files changed, compilation
# skipped" and every `vm.getCode` against those contracts reverts.
@test "forge lint writes bytecode-less artifacts and calls them compiled" {
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

  [ -f out/Subject.sol/Subject.json ]
  run jq -r '.bytecode.object // "NO-BYTECODE"' out/Subject.sol/Subject.json
  [ "$status" -eq 0 ]
  [ "$output" = "NO-BYTECODE" ]

  run jq -r '.files["src/Subject.sol"].artifacts.Subject | length' cache/solidity-files-cache.json
  [ "$status" -eq 0 ]
  [ "$output" -ge 1 ]
}
