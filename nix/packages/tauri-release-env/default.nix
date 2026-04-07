{ lib
, stdenv
, buildEnv
, darwin
, rust-toolchain
, cargo-tauri
, cargo-release
, gmp
, openssl
, libusb1
, pkg-config
, wasm-bindgen-cli
, gettext
, libiconv
, cargo-flamegraph
, nodejs
, jq
}:
buildEnv {
  name = "tauri-release-env";
  paths = [
    rust-toolchain
    cargo-tauri
    cargo-release
    gmp
    openssl
    libusb1
    pkg-config
    wasm-bindgen-cli
    gettext
    cargo-flamegraph
    nodejs
    jq
  ] ++ lib.optionals stdenv.isDarwin [
    libiconv
    darwin.DarwinTools
  ];
}
