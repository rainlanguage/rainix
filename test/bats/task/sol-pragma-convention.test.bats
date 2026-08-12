setup() {
  # shellcheck disable=SC1091
  source lib/sol-pragma-convention.sh
  TESTDIR="$(mktemp -d)"
}

teardown() {
  rm -rf "$TESTDIR"
}

# --- sol_expected_pragma_operator ---

@test "sol_expected_pragma_operator returns = for concrete contract" {
  cat > "$TESTDIR/Vault.sol" <<'EOF'
pragma solidity =0.8.25;

contract Vault {}
EOF
  run sol_expected_pragma_operator "$TESTDIR/Vault.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '=' ]
}

@test "sol_expected_pragma_operator returns ^ for abstract contract" {
  cat > "$TESTDIR/Base.sol" <<'EOF'
pragma solidity ^0.8.25;

abstract contract Base {
    function foo() public virtual;
}
EOF
  run sol_expected_pragma_operator "$TESTDIR/Base.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '^' ]
}

@test "sol_expected_pragma_operator returns ^ for library" {
  cat > "$TESTDIR/Lib.sol" <<'EOF'
pragma solidity ^0.8.25;

library Lib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
EOF
  run sol_expected_pragma_operator "$TESTDIR/Lib.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '^' ]
}

@test "sol_expected_pragma_operator returns ^ for interface" {
  cat > "$TESTDIR/IVault.sol" <<'EOF'
pragma solidity ^0.8.25;

interface IVault {
    function deposit(uint256 assets) external;
}
EOF
  run sol_expected_pragma_operator "$TESTDIR/IVault.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '^' ]
}

@test "sol_expected_pragma_operator returns ^ for file with only errors and constants" {
  cat > "$TESTDIR/Errors.sol" <<'EOF'
pragma solidity ^0.8.25;

error Unauthorized();
error NotFound(uint256 id);

uint256 constant MAX = 100;
EOF
  run sol_expected_pragma_operator "$TESTDIR/Errors.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '^' ]
}

@test "sol_expected_pragma_operator returns = for concrete contract alongside errors" {
  cat > "$TESTDIR/ErrContract.sol" <<'EOF'
pragma solidity =0.8.25;

error Unauthorized();

contract Guard {}
EOF
  run sol_expected_pragma_operator "$TESTDIR/ErrContract.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '=' ]
}

@test "sol_expected_pragma_operator skips file with no pragma" {
  cat > "$TESTDIR/NoPragma.sol" <<'EOF'
contract Foo {}
EOF
  run sol_expected_pragma_operator "$TESTDIR/NoPragma.sol"
  [ "$status" -ne 0 ]
  [ "$output" = '' ]
}

@test "sol_expected_pragma_operator ignores contract keyword in block comment" {
  cat > "$TESTDIR/BlockCommented.sol" <<'EOF'
pragma solidity ^0.8.25;

/*
contract Decoy {}
*/
library Util {}
EOF
  run sol_expected_pragma_operator "$TESTDIR/BlockCommented.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '^' ]
}

@test "sol_expected_pragma_operator ignores contract keyword in line comment" {
  cat > "$TESTDIR/LineCommented.sol" <<'EOF'
pragma solidity ^0.8.25;

// contract Decoy {}
library Util {}
EOF
  run sol_expected_pragma_operator "$TESTDIR/LineCommented.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '^' ]
}

# --- sol_actual_pragma_operator ---

@test "sol_actual_pragma_operator extracts ^ operator" {
  cat > "$TESTDIR/Caret.sol" <<'EOF'
pragma solidity ^0.8.25;
library L {}
EOF
  run sol_actual_pragma_operator "$TESTDIR/Caret.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '^' ]
}

@test "sol_actual_pragma_operator extracts = operator" {
  cat > "$TESTDIR/Exact.sol" <<'EOF'
pragma solidity =0.8.25;
contract C {}
EOF
  run sol_actual_pragma_operator "$TESTDIR/Exact.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '=' ]
}

@test "sol_actual_pragma_operator extracts >= operator" {
  cat > "$TESTDIR/Range.sol" <<'EOF'
pragma solidity >=0.8.0;
contract C {}
EOF
  run sol_actual_pragma_operator "$TESTDIR/Range.sol"
  [ "$status" -eq 0 ]
  [ "$output" = '>=' ]
}

@test "sol_actual_pragma_operator returns non-zero for missing pragma" {
  cat > "$TESTDIR/NoPragma.sol" <<'EOF'
contract C {}
EOF
  run sol_actual_pragma_operator "$TESTDIR/NoPragma.sol"
  [ "$status" -ne 0 ]
}

# --- sol_pragma_convention_check ---

@test "sol_pragma_convention_check passes concrete contract with =" {
  cat > "$TESTDIR/Concrete.sol" <<'EOF'
pragma solidity =0.8.25;
contract Concrete {}
EOF
  run sol_pragma_convention_check "$TESTDIR/Concrete.sol"
  [ "$status" -eq 0 ]
}

@test "sol_pragma_convention_check passes library with ^" {
  cat > "$TESTDIR/MyLib.sol" <<'EOF'
pragma solidity ^0.8.25;
library MyLib {}
EOF
  run sol_pragma_convention_check "$TESTDIR/MyLib.sol"
  [ "$status" -eq 0 ]
}

@test "sol_pragma_convention_check passes abstract contract with ^" {
  cat > "$TESTDIR/Abstract.sol" <<'EOF'
pragma solidity ^0.8.25;
abstract contract Abstract {}
EOF
  run sol_pragma_convention_check "$TESTDIR/Abstract.sol"
  [ "$status" -eq 0 ]
}

@test "sol_pragma_convention_check fails concrete contract with ^" {
  cat > "$TESTDIR/Bad.sol" <<'EOF'
pragma solidity ^0.8.25;
contract Bad {}
EOF
  run sol_pragma_convention_check "$TESTDIR/Bad.sol"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Bad.sol"* ]]
  [[ "$output" == *'requires "="'* ]]
}

@test "sol_pragma_convention_check fails library with =" {
  cat > "$TESTDIR/BadLib.sol" <<'EOF'
pragma solidity =0.8.25;
library BadLib {}
EOF
  run sol_pragma_convention_check "$TESTDIR/BadLib.sol"
  [ "$status" -ne 0 ]
  [[ "$output" == *"BadLib.sol"* ]]
  [[ "$output" == *'requires "^"'* ]]
}

@test "sol_pragma_convention_check fails on unsupported >= operator" {
  cat > "$TESTDIR/Range.sol" <<'EOF'
pragma solidity >=0.8.25;
contract C {}
EOF
  run sol_pragma_convention_check "$TESTDIR/Range.sol"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Range.sol"* ]]
  [[ "$output" == *'not ^ or ='* ]]
}

@test "sol_pragma_convention_check skips files with no pragma" {
  cat > "$TESTDIR/NoPragma.sol" <<'EOF'
contract C {}
EOF
  run sol_pragma_convention_check "$TESTDIR/NoPragma.sol"
  [ "$status" -eq 0 ]
}

@test "sol_pragma_convention_check passes multiple files" {
  cat > "$TESTDIR/Lib.sol" <<'EOF'
pragma solidity ^0.8.25;
library Lib {}
EOF
  cat > "$TESTDIR/Conc.sol" <<'EOF'
pragma solidity =0.8.25;
contract Conc {}
EOF
  run sol_pragma_convention_check "$TESTDIR/Lib.sol" "$TESTDIR/Conc.sol"
  [ "$status" -eq 0 ]
}

@test "sol_pragma_convention_check catches the offending file among many" {
  cat > "$TESTDIR/Good.sol" <<'EOF'
pragma solidity ^0.8.25;
library Good {}
EOF
  cat > "$TESTDIR/Bad.sol" <<'EOF'
pragma solidity ^0.8.25;
contract Bad {}
EOF
  run sol_pragma_convention_check "$TESTDIR/Good.sol" "$TESTDIR/Bad.sol"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Bad.sol"* ]]
  [[ "$output" != *"Good.sol"* ]]
}

# --- sol_pragma_convention_check_tracked ---

@test "sol_pragma_convention_check_tracked passes a clean tracked repo" {
  (
    cd "$TESTDIR" || exit 1
    git init -q
    git config user.email test@example.com
    git config user.name test
    printf 'pragma solidity ^0.8.25;\nlibrary L {}\n' > L.sol
    printf 'pragma solidity =0.8.25;\ncontract C {}\n' > C.sol
    git add L.sol C.sol
  )
  run bash -c "cd '$TESTDIR' && source '$BATS_TEST_DIRNAME/../../../lib/sol-pragma-convention.sh' && sol_pragma_convention_check_tracked"
  [ "$status" -eq 0 ]
}

@test "sol_pragma_convention_check_tracked fails on a convention-violating file" {
  (
    cd "$TESTDIR" || exit 1
    git init -q
    git config user.email test@example.com
    git config user.name test
    printf 'pragma solidity ^0.8.25;\ncontract Bad {}\n' > Bad.sol
    git add Bad.sol
  )
  run bash -c "cd '$TESTDIR' && source '$BATS_TEST_DIRNAME/../../../lib/sol-pragma-convention.sh' && sol_pragma_convention_check_tracked"
  [ "$status" -ne 0 ]
  [[ "$output" == *"Bad.sol"* ]]
}

@test "sol_pragma_convention_check_tracked skips src/generated/ files" {
  (
    cd "$TESTDIR" || exit 1
    git init -q
    git config user.email test@example.com
    git config user.name test
    mkdir -p src/generated
    # Deliberate violation in generated dir — should be skipped
    printf 'pragma solidity ^0.8.25;\ncontract Generated {}\n' > src/generated/Pointers.sol
    git add src/generated/Pointers.sol
  )
  run bash -c "cd '$TESTDIR' && source '$BATS_TEST_DIRNAME/../../../lib/sol-pragma-convention.sh' && sol_pragma_convention_check_tracked"
  [ "$status" -eq 0 ]
}

@test "sol_pragma_convention_check_tracked skips dependencies/ files" {
  (
    cd "$TESTDIR" || exit 1
    git init -q
    git config user.email test@example.com
    git config user.name test
    mkdir -p dependencies/some-lib
    # Deliberate violation in vendored dir — should be skipped
    printf 'pragma solidity ^0.8.25;\ncontract Vendored {}\n' > dependencies/some-lib/Foo.sol
    git add dependencies/some-lib/Foo.sol
  )
  run bash -c "cd '$TESTDIR' && source '$BATS_TEST_DIRNAME/../../../lib/sol-pragma-convention.sh' && sol_pragma_convention_check_tracked"
  [ "$status" -eq 0 ]
}

@test "sol_pragma_convention_check_tracked passes empty repo" {
  (
    cd "$TESTDIR" || exit 1
    git init -q
    git config user.email test@example.com
    git config user.name test
  )
  run bash -c "cd '$TESTDIR' && source '$BATS_TEST_DIRNAME/../../../lib/sol-pragma-convention.sh' && sol_pragma_convention_check_tracked"
  [ "$status" -eq 0 ]
}
