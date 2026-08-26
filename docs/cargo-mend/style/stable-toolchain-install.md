## Stable toolchain install

cargo-mend links against `rustc_driver`. Always install with stable + `RUSTC_BOOTSTRAP=1` so the binary can read `.rmeta` files from stable-toolchain projects. Installing with nightly produces a binary that fails with `E0514` on stable projects.

### Local development

```bash
RUSTC_BOOTSTRAP=1 cargo +stable install --path .
```

### Published release

```bash
RUSTC_BOOTSTRAP=1 cargo +stable install cargo-mend --version <VERSION>
```

Never use `cargo install --path .` without these flags — it will default to nightly (if that's your active toolchain) and produce an incompatible binary.
