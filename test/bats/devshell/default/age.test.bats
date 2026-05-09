@test "age should be available on PATH" {
  run age --version
  [ "$status" -eq 0 ]
}

@test "age-keygen should be available on PATH" {
  run age-keygen --help
  [ "$status" -eq 0 ]
}

@test "age round-trips a payload through a generated identity" {
  tmpdir=$(mktemp -d)
  age-keygen -o "$tmpdir/key.txt" 2>/dev/null
  recipient=$(grep "public key:" "$tmpdir/key.txt" | sed 's/.*public key: //')
  printf 'hello world' | age --recipient "$recipient" --output "$tmpdir/out.age"
  run age --decrypt --identity "$tmpdir/key.txt" "$tmpdir/out.age"
  [ "$status" -eq 0 ]
  [ "$output" = "hello world" ]
  rm -rf "$tmpdir"
}
