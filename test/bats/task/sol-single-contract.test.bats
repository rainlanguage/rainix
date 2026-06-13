setup() {
  # shellcheck disable=SC1091
  source lib/sol-single-contract.sh
  TESTDIR="$(mktemp -d)"
}

teardown() {
  rm -rf "$TESTDIR"
}

@test "sol_count_contracts counts a single contract" {
  cat > "$TESTDIR/Counter.sol" <<'EOF'
pragma solidity ^0.8.25;

contract Counter {
    uint256 public count;
}
EOF
  run sol_count_contracts "$TESTDIR/Counter.sol"
  [ "$status" -eq 0 ]
  [ "$output" -eq 1 ]
}

@test "sol_count_contracts counts abstract contract" {
  cat > "$TESTDIR/Base.sol" <<'EOF'
pragma solidity ^0.8.25;

abstract contract Base {
    function foo() public virtual;
}
EOF
  run sol_count_contracts "$TESTDIR/Base.sol"
  [ "$status" -eq 0 ]
  [ "$output" -eq 1 ]
}

@test "sol_count_contracts counts two contracts" {
  cat > "$TESTDIR/Two.sol" <<'EOF'
pragma solidity ^0.8.25;

contract A {}

contract B {}
EOF
  run sol_count_contracts "$TESTDIR/Two.sol"
  [ "$status" -eq 0 ]
  [ "$output" -eq 2 ]
}

@test "sol_count_contracts does not count library or interface" {
  cat > "$TESTDIR/Lib.sol" <<'EOF'
pragma solidity ^0.8.25;

library Math {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}

interface IThing {
    function go() external;
}
EOF
  run sol_count_contracts "$TESTDIR/Lib.sol"
  [ "$status" -eq 0 ]
  [ "$output" -eq 0 ]
}

@test "sol_count_contracts ignores the keyword in line comments" {
  cat > "$TESTDIR/Commented.sol" <<'EOF'
pragma solidity ^0.8.25;

// contract Decoy is not real
contract Real {}
EOF
  run sol_count_contracts "$TESTDIR/Commented.sol"
  [ "$status" -eq 0 ]
  [ "$output" -eq 1 ]
}

@test "sol_count_contracts ignores the keyword in block comments" {
  cat > "$TESTDIR/BlockComment.sol" <<'EOF'
pragma solidity ^0.8.25;

/*
contract Decoy {}
contract AlsoDecoy {}
*/
contract Real {}
EOF
  run sol_count_contracts "$TESTDIR/BlockComment.sol"
  [ "$status" -eq 0 ]
  [ "$output" -eq 1 ]
}

@test "sol_count_contracts allows file-scope error/struct/enum alongside one contract" {
  cat > "$TESTDIR/WithFileScope.sol" <<'EOF'
pragma solidity ^0.8.25;

error Unauthorized();

struct Point {
    uint256 x;
    uint256 y;
}

enum State {
    Open,
    Closed
}

uint256 constant MAX = 100;

contract Real {}
EOF
  run sol_count_contracts "$TESTDIR/WithFileScope.sol"
  [ "$status" -eq 0 ]
  [ "$output" -eq 1 ]
}

@test "sol_single_contract_check passes for one contract per file" {
  cat > "$TESTDIR/A.sol" <<'EOF'
contract A {}
EOF
  cat > "$TESTDIR/B.sol" <<'EOF'
abstract contract B {}
EOF
  run sol_single_contract_check "$TESTDIR/A.sol" "$TESTDIR/B.sol"
  [ "$status" -eq 0 ]
}

@test "sol_single_contract_check fails when a file declares two contracts" {
  cat > "$TESTDIR/Ok.sol" <<'EOF'
contract Ok {}
EOF
  cat > "$TESTDIR/Bad.sol" <<'EOF'
contract Bad1 {}
contract Bad2 {}
EOF
  run sol_single_contract_check "$TESTDIR/Ok.sol" "$TESTDIR/Bad.sol"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Bad.sol"* ]]
  [[ "$output" == *"declares 2 contracts"* ]]
}

@test "sol_single_contract_check fails on a contract plus an inline mock" {
  cat > "$TESTDIR/Mock.t.sol" <<'EOF'
pragma solidity ^0.8.25;

contract MockAsset {}

contract ManagerTest {}
EOF
  run sol_single_contract_check "$TESTDIR/Mock.t.sol"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Mock.t.sol"* ]]
}

@test "sol_single_contract_check_tracked passes on a clean tracked repo" {
  (
    cd "$TESTDIR" || exit 1
    git init -q
    git config user.email test@example.com
    git config user.name test
    printf 'contract A {}\n' > A.sol
    printf 'contract B {}\n' > B.sol
    git add A.sol B.sol
  )
  run bash -c "cd '$TESTDIR' && source '$BATS_TEST_DIRNAME/../../../lib/sol-single-contract.sh' && sol_single_contract_check_tracked"
  [ "$status" -eq 0 ]
}

@test "sol_single_contract_check_tracked fails on a tracked multi-contract file" {
  (
    cd "$TESTDIR" || exit 1
    git init -q
    git config user.email test@example.com
    git config user.name test
    printf 'contract A {}\ncontract B {}\n' > Multi.sol
    git add Multi.sol
  )
  run bash -c "cd '$TESTDIR' && source '$BATS_TEST_DIRNAME/../../../lib/sol-single-contract.sh' && sol_single_contract_check_tracked"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Multi.sol"* ]]
}
