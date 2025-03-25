#!/bin/bash

# Bail if /usr/bin is not in the PATH
if echo "$PATH" | grep -qE '(^|:)/usr/bin(:|$)'; then
  echo "/usr/bin is in the PATH, ...OK"
else
  echo "did NOT find any /usr/bin in the PATH, aborting..."
  exit 1
fi

# Bail if xcrun is in the PATH
if echo "$PATH" | grep -qE '(^|:)xcrun(:|$)'; then
  echo "found xcrun in the PATH, aborting..."
  exit 1
else
  echo "no xcrun detected in the PATH, ...OK"
fi

# Bail if DEVELOPER_DIR is set
if [ -z "${DEVELOPER_DIR+x}" ]; then
  echo "The environment variable DEVELOPER_DIR is unset, ...OK"
else
  echo "The environment variable DEVELOPER_DIR is not unset, aborting..."
  exit 1
fi
