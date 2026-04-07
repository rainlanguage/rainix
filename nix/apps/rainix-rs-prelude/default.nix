{ lib
, stdenv
, writeShellApplication
, rust-toolchain
, cargo-release
, gmp
, openssl
, libusb1
, pkg-config
, wasm-bindgen-cli
, gettext
, cargo-flamegraph
, libiconv
, darwin
}:
writeShellApplication {
  name = "rainix-rs-prelude";
  meta.description = "Rainix Rust prelude";
  runtimeInputs = [
    rust-toolchain
    cargo-release
    gmp
    openssl
    libusb1
    pkg-config
    wasm-bindgen-cli
    gettext
    cargo-flamegraph
  ] ++ lib.optionals stdenv.isDarwin [
    libiconv
    darwin.DarwinTools
  ];
  text = ''
    # Intentionally empty — exists so downstream consumers can call
    # rainix-rs-prelude unconditionally alongside rainix-sol-prelude.
  '';
}
