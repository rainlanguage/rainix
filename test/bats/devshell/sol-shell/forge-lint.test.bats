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
