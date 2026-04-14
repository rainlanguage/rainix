{ stdenv
, system ? stdenv.targetPlatform.system
}:
stdenv.mkDerivation (finalAttrs: {
  pname = "the-graph";
  version = "0.69.2";
  phases = [
    "unpackPhase"
    "installPhase"
  ];
  src =
    let
      release-name = "%40graphprotocol%2Fgraph-cli%40${finalAttrs.version}";
      system-mapping = {
        x86_64-linux = "linux-x64";
        x86_64-darwin = "darwin-x64";
        aarch64-darwin = "darwin-arm64";
      };
      system-sha = {
        x86_64-linux = "sha256:07grrdrx8w3m8sqwdmf9z9zymwnnzxckgnnjzfndk03a8r2d826m";
        x86_64-darwin = "sha256:0j4p2bkx6pflkif6xkvfy4vj1v183mkg59p2kf3rk48wqfclids8";
        aarch64-darwin = "sha256:0pq0g0fq1myp0s58lswhcab6ccszpi5sx6l3y9a18ai0c6yzxim0";
      };
    in
    fetchTarball {
      url = "https://github.com/graphprotocol/graph-tooling/releases/download/${release-name}/graph-${system-mapping.${system}}.tar.gz";
      sha256 = system-sha.${system};
    };
  installPhase = ''
    mkdir -p $out
    cp -r $src/* $out
  '';
})
