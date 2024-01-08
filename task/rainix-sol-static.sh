#!/usr/bin/env bash
set -euxo pipefail

slither .
forge fmt --check