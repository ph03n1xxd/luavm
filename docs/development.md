# Development and releases

## Workspace

LuaVM is a Rust workspace with separated responsibilities:

| Path | Responsibility |
| --- | --- |
| `crates/util` | Error and identifier foundations |
| `crates/core` | Versions, project configuration, profiles, and selection |
| `crates/io` | Filesystem, archive, download, checksum, and build adapters |
| `crates/engine` | Install, selection, state, and workflow orchestration |
| `apps/cli` | The `luavm` command-line application |
| `xtask` | Repository checks and automation |

## Local requirements

- Rust 1.85 or newer;
- the Rust `rustfmt` and `clippy` components;
- `cargo-nextest` for the test suite;
- `cargo-deny` when dependency manifests change;
- LuaVM host build prerequisites when running source-build smoke tests.

## Verification commands

Run these from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo run -p xtask -- layers
```

For a release binary:

```bash
cargo build --locked --release -p luavm
```

The binary is written to `target/release/luavm` or `target/release/luavm.exe`.

## Source-build smoke test

With the host compiler and make tool available:

```bash
luavm doctor
luavm install 5.4.8 --no-prerequisites
luavm exec 5.4.8 -- lua -v
```

Use a temporary `LUAVM_DATA_ROOT` for repeatable testing. Do not use a real user data root for destructive test cases.

## Documentation changes

Keep the root README focused on orientation and common commands. Put detailed workflows, platform notes, and troubleshooting procedures in `docs/`. Examples must match the current CLI help and should state when a command changes only LuaVM state versus the current shell.

## Release process

The release workflow is triggered by a tag matching `v*`. It builds Linux x86_64, macOS x86_64, and Windows x86_64 archives, then publishes checksums and release assets.

Before a release:

1. run the verification commands above;
2. build and inspect the release binary locally;
3. review the complete diff and documentation;
4. merge through the normal pull-request process;
5. create and push a version tag, for example:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

Do not push directly to `main`. Do not include local agent instructions, notebooks, secrets, or personal data in a release commit.
