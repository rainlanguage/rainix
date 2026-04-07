{ writeShellApplication
, rust-toolchain
}:
writeShellApplication {
  name = "rainix-rs-static";
  meta.description = "Rainix Rust static analysis";
  runtimeInputs = [
    rust-toolchain
  ];
  text = ''
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D clippy::all
  '';
}
