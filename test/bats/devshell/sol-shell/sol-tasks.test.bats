# sol-shell tasks remaining after the trivial-wrapper cleanup. Static, test,
# and legal are now invoked directly as `slither .` / `forge test -vvv` /
# `reuse lint` — `cargo test`-style — from the rainix reusable workflows. This
# bats only checks `rainix-sol-artifacts` since it wraps a non-trivial deploy
# script with retries.

@test "rainix-sol-artifacts should be available on PATH" {
  run command -v rainix-sol-artifacts
  [ "$status" -eq 0 ]
}

@test "check-published-deploy-constants should be available on PATH" {
  run command -v check-published-deploy-constants
  [ "$status" -eq 0 ]
}
