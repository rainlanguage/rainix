#!/usr/bin/env bats

@test "/usr/bin should be in PATH" {
  run bash -c 'echo "$PATH" \| grep -qE "(^|:)/usr/bin(:|$)"'
  [ "$status" -eq 0 ]
}

@test "xcrun should NOT be in PATH" {
  run bash -c 'echo "$PATH" | grep -q "xcrun"'
  [ "$status" -ne 0 ]
}

@test "DEVELOPER_DIR should be unset" {
  run test -z "${DEVELOPER_DIR+x}"
  [ "$status" -eq 0 ]
}
