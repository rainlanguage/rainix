{ writeShellApplication
, rust-toolchain
}:
writeShellApplication {
  name = "rainix-rs-test";
  meta.description = "Rainix Rust tests";
  runtimeInputs = [
    rust-toolchain
  ];
  text = ''
    cargo test
  '';
}
