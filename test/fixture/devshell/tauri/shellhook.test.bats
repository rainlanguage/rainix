#!/usr/bin/env bats

@test "/usr/bin should be in PATH" {
  echo "$PATH" | grep -qE "(^|:)/usr/bin(:|$)"
}

@test "xcrun should NOT be in PATH" {
  ! echo "$PATH" | grep -q "xcrun"
}

@test "DEVELOPER_DIR should be unset" {
  test -z "${DEVELOPER_DIR+x}"
}
