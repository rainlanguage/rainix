#!/usr/bin/env bats

@test "/usr/bin should be in PATH" {
  run bash -c 'echo "$PATH" | grep -qE "(^|:)/usr/bin(:|$)"'
  [ "$status" -eq 0 ]
}

@test "nixpkgs apple_sdk xcrun should NOT be in PATH" {
  run bash -c 'echo "$PATH" | grep -q "xcrun"'
  [ "$status" -ne 0 ]
}

@test "should have access to native macos xcrun" {
  run xcrun --version
  [ "$status" -eq 0 ]

  run which xcrun
  [ "$output" == "/usr/bin/xcrun" ]
  [ "$status" -eq 0 ]
}

@test "should have access to native macos SetFile bin through native macos xcrun" {
  run xcrun --find SetFile
  [ "$output" == "/Library/Developer/CommandLineTools/usr/bin/SetFile" ]
  [ "$status" -eq 0 ]
}

@test "should have access to native macos SetFile bin through /usr/bin in PATH" {
  run which SetFile
  [ "$output" == "/usr/bin/SetFile" ]
  [ "$status" -eq 0 ]
}

@test "DEVELOPER_DIR should be unset" {
  run test -z "${DEVELOPER_DIR+x}"
  [ "$status" -eq 0 ]
}
