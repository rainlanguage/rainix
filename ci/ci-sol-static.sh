#!/usr/bin/env bash
# Slither first to avoid any potential conflicts with other checks.
slither --ignore-compile --skip-clean .
forge fmt --check