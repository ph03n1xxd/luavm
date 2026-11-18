# Configuration and storage

## Project configuration

LuaVM reads the nearest `.luavm.toml` when a command enables project selection, such as `detect`, `current`, `env --project`, or `exec --project`.

Minimal configuration:

```toml
lua = "5.4.8"
```

Build configuration:

```toml
lua = "5.4.8"

[build]
cflags = ["-O2", "-DPROJECT_BUILD"]
ldflags = []
make_args = []
```

The supported keys are:

- `lua`: a Lua version such as `5.4.8`;
- `profile`: an existing profile ID to select explicitly;
- `build.cflags`: compiler flags;
- `build.ldflags`: linker flags;
- `build.make_args`: make arguments.

Configuration values are parsed as data and are not passed through a shell. Invalid configuration stops selection with an error instead of silently falling back to another version.

## Selection precedence

When a command resolves a Lua version, the sources are considered in this order:

1. an explicit version in the command;
2. the nearest `.luavm.toml` when project selection is enabled;
3. the global selection;
4. no selection.

Inspect the source and installation state with:

```text
luavm detect --json
luavm current --json
```

## Build profiles

Every installation has a profile ID derived from the Lua version, target, compiler identity, compiler flags, linker flags, and make arguments. Different build settings therefore produce separate profiles instead of overwriting an existing installation.

An installation has this shape:

```text
<data-root>/installations/lua/<version>/profiles/<profile-id>/
  bin/
  include/
  lib/
  metadata.json
```

The install command prints the profile ID and final path. Use that ID with commands that accept `--profile`.

## Data roots

LuaVM keeps downloads, state, staging data, and installations below one root:

| Platform | Default |
| --- | --- |
| Linux | `$XDG_DATA_HOME/luavm`, otherwise `$HOME/.local/share/luavm` |
| macOS | `$HOME/Library/Application Support/luavm` |
| Windows | `%LOCALAPPDATA%\\luavm` |

For tests and automation, set `LUAVM_DATA_ROOT` before invoking LuaVM:

```powershell
$env:LUAVM_DATA_ROOT = Join-Path $env:TEMP "luavm-test"
luavm list
```

The root contains:

```text
state.json
downloads/
installations/lua/
staging/
logs/
```

LuaVM never installs into system directories and never permanently edits the host `PATH`.

## Checksums and archives

Official release metadata is fetched from the Lua archive catalog. When a checksum is available, LuaVM verifies the download before extraction. If a release has no catalog checksum, installation requires explicit `--allow-unverified` confirmation.

Archives are bounded, checked for traversal, and rejected when they contain symbolic or hard links. Builds occur in a per-operation staging directory and are published only after binary verification.
