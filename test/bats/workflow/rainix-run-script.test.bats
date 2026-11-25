# rainix-run-script.yaml is `workflow_call` only; what is asserted is its
# wiring. `rainix-static safe-queue` itself is covered by its unit tests.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-run-script.yaml"
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
  queue_step="$(yq -r '.jobs.run.steps[] | select(.name == "Fetch the Safe queue") | .run' "$workflow")"
  run_env_queue="$(yq -r '.jobs.run.steps[] | select(.name == "Run script") | .env.SAFE_QUEUE' "$workflow")"
}

@test "the workflow can only be called, never triggered" {
  [ "$(yq -r '.on | keys | join(",")' "$workflow")" = "workflow_call" ]
}

@test "every rainix ref in a run step resolves through the pinned sha" {
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]]
  local unpinned
  unpinned="$(yq -r '.jobs.run.steps[] | select(.run) | .run' "$workflow" | grep 'github:rainlanguage/rainix' | grep -v 'env.RAINIX_SHA' || true)"
  [ -z "$unpinned" ]
}

@test "the queue step writes the file the run step names in SAFE_QUEUE" {
  [ -n "$run_env_queue" ] && [ "$run_env_queue" != "null" ]
  [[ "$queue_step" == *"rainix-static safe-queue"*"--out $run_env_queue"* ]]
}

@test "the queue step fails the job when safe-queue fails" {
  [[ "$queue_step" == "set -euo pipefail"* ]]
}

@test "the queue is fetched before the script runs" {
  local queue run
  queue="$(yq -r '.jobs.run.steps | to_entries[] | select(.value.name == "Fetch the Safe queue") | .key' "$workflow")"
  run="$(yq -r '.jobs.run.steps | to_entries[] | select(.value.name == "Run script") | .key' "$workflow")"
  [ "$queue" -lt "$run" ]
}

@test "the script is built before safe-queue reads its ABI" {
  [[ "$queue_step" == *"forge build"*"rainix-static safe-queue"* ]]
}
