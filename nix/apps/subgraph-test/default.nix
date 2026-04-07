{ writeShellApplication }:
writeShellApplication {
  name = "subgraph-test";
  meta.description = "Brings up a subgraph docker image, failing if the container exits.";
  text = ''
    (cd ./subgraph && docker compose up --abort-on-container-exit)
  '';
}
