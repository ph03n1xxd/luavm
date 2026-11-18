# LuaVM

LuaVM is a lightweight, cross-platform Lua version manager written in Rust. It builds official Lua source releases with the host toolchain and keeps each version and build profile isolated in a user-local data directory.

[![CI](https://github.com/ph03n1xxd/luavm/actions/workflows/ci.yml/badge.svg)](https://github.com/ph03n1xxd/luavm/actions/workflows/ci.yml)

## Contents

- [About](#about)
- [Getting started](#getting-started)
- [Common usage](#common-usage)
- [Documentation](#documentation)
- [Build from source](#build-from-source)
- [Project status](#project-status)
- [Contributing](#contributing)
- [License](#license)

## About

LuaVM is designed for developers who need more than one Lua release without replacing the system Lua installation.

It provides:

- side-by-side Lua installations;
- checksum-verified downloads from the official Lua archive catalog;
- isolated build profiles for custom flags;
- global and project-local version selection;
- shell environment output and child-process execution;
- bounded archive extraction and atomic publication;
- Linux, macOS, and Windows support through the native host toolchain.

LuaJIT, Lua forks, LuaRocks, package management, and permanent system `PATH` changes are outside the project scope.

## Getting started

### Install LuaVM

Download the archive for your operating system from the [latest release](https://github.com/ph03n1xxd/luavm/releases/latest), verify its `SHA256SUMS` entry, extract the `luavm` executable, and place it on your `PATH`.

For a source checkout, see [Building and contributing](docs/development.md).

### Check prerequisites

LuaVM compiles Lua locally, so a C compiler, `make`, `curl`, and `tar` are required. Run:

```powershell
luavm doctor
```

On Windows, install MinGW-w64 or LLVM and make sure `mingw32-make` plus `gcc` or `clang` are on `PATH`. On macOS, install Xcode Command Line Tools. On Linux, install the distribution's C build tools.

### Install and run Lua

```text
luavm list --available
luavm install 5.4.8
luavm exec 5.4.8 -- lua -v
```

Installation reports its current phase while downloading, extracting, building, and publishing the profile.

### Select a version

Set a default for future commands:

```text
luavm global 5.4.8
luavm exec -- lua -v
```

Use a project-local version by creating `.luavm.toml`:

```toml
lua = "5.4.8"

[build]
cflags = ["-O2"]
ldflags = []
make_args = []
```

Then run from that project or a child directory:

```text
luavm detect
luavm exec --project -- lua -v
```

For shell setup, see [Shell integration](docs/commands.md#shell-integration).

## Common usage

```text
# Show installed versions
luavm list

# Show versions available from the remote Lua catalog
luavm list --available

# Install a release
luavm install 5.4.8

# Run one command with a selected Lua version
luavm exec 5.4.8 -- lua script.lua

# Inspect the selected version and its source
luavm current
luavm detect --json

# Remove an installation after explicit confirmation
luavm uninstall 5.4.8 --yes
```

LuaVM does not modify the parent shell automatically. Use `luavm env`, `luavm shell-init`, or `luavm exec` when a command needs the selected Lua on `PATH`.

## Documentation

- [Getting started](docs/getting-started.md): installation, prerequisites, and first run.
- [Command reference](docs/commands.md): commands, options, output, and shell integration.
- [Configuration and storage](docs/configuration.md): `.luavm.toml`, selection precedence, profiles, and data roots.
- [Development and releases](docs/development.md): workspace checks, tests, and release packaging.
- [Troubleshooting](docs/troubleshooting.md): common setup, build, selection, and uninstall problems.

## Build from source

Requirements: Rust 1.85 or newer and the host build prerequisites described above.

```bash
cargo build --locked --release -p luavm
```

The executable is written to `target/release/luavm` or `target/release/luavm.exe` on Windows.

## Project status

LuaVM 0.1.0 is an active release candidate. The core install, isolation, selection, shell, and Windows source-build workflows are implemented. Cross-platform acceptance testing, documentation refinement, and release hardening are still ongoing.

macOS is supported by LuaVM, but macOS GitHub Actions builds are currently disabled because the hosted Intel runner has been unreliable to provision. Linux and Windows remain covered by continuous integration.

## Contributing

Keep changes narrow, add regression coverage, and run the checks in [Development and releases](docs/development.md). Changes to `main` are made through pull requests.

## License

LuaVM is distributed under the [MIT License](LICENSE).

## Acknowledgments

The README organization was informed by the [Best-README-Template](https://github.com/othneildrew/Best-README-Template). The project documentation and examples here are written specifically for LuaVM.
