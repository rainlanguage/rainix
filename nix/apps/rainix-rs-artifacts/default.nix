{ writeShellApplication
, rust-toolchain
}:
writeShellApplication {
  name = "rainix-rs-artifacts";
  meta.description = "Rainix Rust build artifacts";
  runtimeInputs = [
    rust-toolchain
  ];
  text = ''
    cargo build --release
  '';
}
