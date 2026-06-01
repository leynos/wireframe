# Verus tool metadata

`VERSION` pins the Verus release token used by `prover-tools verus install`.
`SHA256SUMS` records the expected archive digest for the Linux release artefact
that Wireframe currently supports for local installation.

The current checksum was derived from the official GitHub release artefact:

```sh
curl -L --fail --show-error \
  -o /tmp/verus-0.2026.05.24.ecee80a-x86-linux.zip \
  https://github.com/verus-lang/verus/releases/download/\
release/0.2026.05.24.ecee80a/verus-0.2026.05.24.ecee80a-x86-linux.zip
sha256sum /tmp/verus-0.2026.05.24.ecee80a-x86-linux.zip
```

Expected output:

```text
323a44c0d787ce9a788665e1c6922360c44a72d1b9696359ec4f7bf5fbbc63e6  /tmp/verus-0.2026.05.24.ecee80a-x86-linux.zip
```
