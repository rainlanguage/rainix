# rainix-subgraph-deploy.yaml is `workflow_call` only and deploys with a live
# key, so nothing here runs it; what is asserted is its wiring.

setup() {
  repo_root="$BATS_TEST_DIRNAME/../../.."
  workflow="$repo_root/.github/workflows/rainix-subgraph-deploy.yaml"
  sha="$(yq -r '.env.RAINIX_SHA' "$workflow")"
  deploy_run="$(yq -r '.jobs.deploy.steps[] | select(.run) | select(.run | test("-c subgraph-deploy$")) | .run' "$workflow")"
  # The env vars the task refuses to run without, read from the task itself.
  required="$(grep -oE '^\s*subgraph_require_env [A-Z_ ]+' "$repo_root/lib/subgraph-deploy.sh" | sed 's/^ *subgraph_require_env //')"
}

@test "the workflow can only be called, never triggered" {
  [ "$(yq -r '.on | keys | join(",")' "$workflow")" = "workflow_call" ]
}

@test "subgraph-deploy runs in the subgraph shell at the pinned sha" {
  [[ "$sha" =~ ^[0-9a-f]{40}$ ]]
  # shellcheck disable=SC2016
  [ "$deploy_run" = 'nix develop github:rainlanguage/rainix/${{ env.RAINIX_SHA }}#subgraph-shell -c subgraph-deploy' ]
}

@test "every rainix ref in a run step resolves through the pinned sha" {
  local unpinned
  unpinned="$(yq -r '.jobs.deploy.steps[] | select(.run) | .run' "$workflow" | grep 'github:rainlanguage/rainix' | grep -v 'env.RAINIX_SHA' || true)"
  [ -z "$unpinned" ]
}

@test "every env var subgraph-deploy requires is wired into its step" {
  [ -n "$required" ]
  local var value
  for var in $required; do
    value="$(yq -r ".jobs.deploy.steps[] | select(.run) | select(.run | test(\"-c subgraph-deploy$\")) | .env.$var" "$workflow")"
    if [ -z "$value" ] || [ "$value" = "null" ]; then
      echo "FAIL: $var is not set on the subgraph-deploy step" >&2
      return 1
    fi
  done
}

@test "each host's credential is a secret scoped to the deploy step alone" {
  local secret holders
  for secret in ORMI_DEPLOY_KEY GOLDSKY_TOKEN; do
    # Optional: a caller supplies the credential of each host it deploys to.
    [ "$(yq -r ".on.workflow_call.secrets.$secret.required" "$workflow")" = "false" ]
    # shellcheck disable=SC2016
    [ "$(yq -r ".jobs.deploy.steps[] | select(.run) | select(.run | test(\"-c subgraph-deploy\$\")) | .env.$secret" "$workflow")" = "\${{ secrets.$secret }}" ]
    holders="$(grep -c "secrets.$secret" "$workflow")"
    [ "$holders" -eq 1 ]
    [ "$(yq -r ".env.$secret" "$workflow")" = "null" ]
    [ "$(yq -r ".jobs.deploy.env.$secret" "$workflow")" = "null" ]
  done
}

@test "the deploy name and query base come from the caller's inputs" {
  # shellcheck disable=SC2016
  [ "$(yq -r '.jobs.deploy.steps[] | select(.run) | select(.run | test("-c subgraph-deploy$")) | .env.SUBGRAPH_NAME' "$workflow")" = '${{ inputs.subgraph-name }}' ]
  # shellcheck disable=SC2016
  [ "$(yq -r '.jobs.deploy.steps[] | select(.run) | select(.run | test("-c subgraph-deploy$")) | .env.ORMI_QUERY_BASE' "$workflow")" = '${{ inputs.ormi-query-base }}' ]
  [ "$(yq -r '.on.workflow_call.inputs.subgraph-name.required' "$workflow")" = "true" ]
  # shellcheck disable=SC2016
  [ "$(yq -r '.jobs.deploy.steps[] | select(.run) | select(.run | test("-c subgraph-deploy$")) | .env.GOLDSKY_QUERY_BASE' "$workflow")" = '${{ inputs.goldsky-query-base }}' ]
  [ "$(yq -r '.on.workflow_call.inputs.ormi-query-base.default' "$workflow")" = "" ]
  [ "$(yq -r '.on.workflow_call.inputs.goldsky-query-base.default' "$workflow")" = "" ]
}

@test "the job can only read the repo" {
  [ "$(yq -r '.jobs.deploy.permissions | to_entries | map(.key + "=" + .value) | join(",")' "$workflow")" = "contents=read" ]
}
