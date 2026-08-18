setup() {
  work="$(mktemp -d)"
  scratch="$(mktemp -d)"
  # An absolute path pins the compiler to the one in this shell, so the check is
  # not resolving a version over the network.
  solc="$(command -v solc-0.8.25)"

  mkdir -p "$work/src/lib" "$work/script" "$work/test/concrete"

  cat > "$work/foundry.toml" <<EOF
[package]
name = "rain-test-package"
version = "0.1.0"

[profile.default]
src = 'src'
out = 'out'
libs = ['dependencies']
solc = "$solc"
EOF

  # The shape every soldeer library in the org ships: the build config and its
  # remappings are the consumer's, and the test tree does not publish.
  cat > "$work/.soldeerignore" <<'EOF'
/foundry.toml
/remappings.txt
/test
EOF

  cat > "$work/src/lib/LibThing.sol" <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity ^0.8.25;

library LibThing {
    function one() internal pure returns (uint256) {
        return 1;
    }
}
EOF

  cat > "$work/test/concrete/Helper.sol" <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity =0.8.25;

contract Helper {}
EOF

  # The worked example a consumer copies. It publishes; the tree it imports from
  # does not.
  cat > "$work/script/Build.sol" <<'EOF'
// SPDX-License-Identifier: LicenseRef-DCL-1.0
pragma solidity =0.8.25;

import {LibThing} from "../src/lib/LibThing.sol";
import {Helper} from "../test/concrete/Helper.sol";

contract Build {
    function run() external returns (uint256) {
        new Helper();
        return LibThing.one();
    }
}
EOF
}

teardown() {
  rm -rf "$work" "$scratch"
}

@test "a published file importing an excluded path fails the build" {
  run rainix-static soldeer-package-build --root "$work" --scratch "$scratch"

  [ "$status" -eq 1 ]
  [[ "$output" == *"rain-test-package~0.1.0 does not build as published"* ]]
  # The repo tree is complete, so this import is only unresolvable in the
  # package — which is the whole reason a repo-side build cannot see it.
  [[ "$output" == *"test/concrete/Helper.sol"* ]]
  [ -f "$work/src/lib/LibThing.sol" ]
}

@test "the same package builds once the imported file publishes too" {
  mkdir -p "$work/src/concrete"
  mv "$work/test/concrete/Helper.sol" "$work/src/concrete/Helper.sol"
  sed -i 's#"../test/concrete/Helper.sol"#"../src/concrete/Helper.sol"#' "$work/script/Build.sol"

  run rainix-static soldeer-package-build --root "$work" --scratch "$scratch"

  [ "$status" -eq 0 ]
  [[ "$output" == *"clean — rain-test-package~0.1.0 builds as published"* ]]
  [[ "$output" == *"3 Solidity files"* ]]
}

@test "the scratch tree carries the build config the package does not ship" {
  mkdir -p "$work/src/concrete"
  mv "$work/test/concrete/Helper.sol" "$work/src/concrete/Helper.sol"
  sed -i 's#"../test/concrete/Helper.sol"#"../src/concrete/Helper.sol"#' "$work/script/Build.sol"
  printf 'some-remapping/=dependencies/some-remapping/\n' > "$work/remappings.txt"

  run rainix-static soldeer-package-build --root "$work" --scratch "$scratch"

  [ "$status" -eq 0 ]
  # A clean run removes the scratch tree, so re-run it against a build that
  # cannot succeed to inspect what the tree was given.
  printf 'import {Nope} from "./Nope.sol";\n' >> "$work/src/lib/LibThing.sol"
  run rainix-static soldeer-package-build --root "$work" --scratch "$scratch"
  [ "$status" -eq 1 ]
  [ -f "$scratch/src/lib/LibThing.sol" ]
  [ ! -d "$scratch/test" ]
  run cat "$scratch/foundry.toml"
  [[ "$output" == *"rain-test-package"* ]]
  run cat "$scratch/remappings.txt"
  [[ "$output" == *"some-remapping/=dependencies/some-remapping/"* ]]
}

@test "a repo that publishes no package is skipped rather than built" {
  cat > "$work/foundry.toml" <<'EOF'
[profile.default]
src = 'src'
EOF

  run rainix-static soldeer-package-build --root "$work" --scratch "$scratch"

  [ "$status" -eq 0 ]
  [[ "$output" == *"declares no [package] name and version"* ]]
  [[ "$output" == *"skipping"* ]]
}

@test "a half-declared package is skipped rather than built" {
  # Both fields are required to name what publishes. The fixture's tree fails to
  # build as published, so either half alone reaching the build is exit 1 here.
  sed -i '/^version = /d' "$work/foundry.toml"

  run rainix-static soldeer-package-build --root "$work" --scratch "$scratch"

  [ "$status" -eq 0 ]
  [[ "$output" == *"declares no [package] name and version"* ]]
  [[ "$output" == *"skipping"* ]]

  sed -i 's/^name = .*/version = "0.1.0"/' "$work/foundry.toml"

  run rainix-static soldeer-package-build --root "$work" --scratch "$scratch"

  [ "$status" -eq 0 ]
  [[ "$output" == *"declares no [package] name and version"* ]]
  [[ "$output" == *"skipping"* ]]
}
