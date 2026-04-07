{ stdenv
, system ? stdenv.targetPlatform.system
}:
stdenv.mkDerivation (finalAttrs: {
  pname = "goldsky";
  version = "8.6.6";
  phases = [
    "installPhase"
  ];
  src =
    let
      system-mapping = {
        x86_64-linux = "linux";
        x86_64-darwin = "macos";
        aarch64-darwin = "macos";
      };
      system-sha = {
        x86_64-linux = "sha256:1cqbinax63w07qxvmgni52qw4cd83ywkhjikw3rd4wgd2fh36027";
        x86_64-darwin = "sha256:0yznf81yxc3a9vnfjdmmzdb59mh9bwrpxw87lrlhlchfr0jmnjk4";
        aarch64-darwin = "sha256:0yznf81yxc3a9vnfjdmmzdb59mh9bwrpxw87lrlhlchfr0jmnjk4";
      };
    in
    builtins.fetchurl {
      url = "https://cli.goldsky.com/${finalAttrs.version}/${system-mapping.${system}}/goldsky";
      sha256 = system-sha.${system};
    };
  installPhase = ''
    mkdir -p $out/bin
    cp $src $out/bin/goldsky
    chmod +x $out/bin/goldsky
  '';
})
