# luavm

A lightweight, cross-platform Lua version manager written in Rust.

LuaVM manages Lua releases in isolated user-local directories. LuaJIT, Lua forks, and package managers are outside the scope of this project.

## Current status

LuaVM compiles official Lua sources with the host toolchain. Run `luavm doctor` before the first install; missing prerequisites can be installed with an explicit, confirmed `luavm doctor --fix`. LuaVM never changes system PATH without that confirmation.

```text
luavm list --available
luavm doctor
luavm doctor --fix
luavm install 5.4.8
luavm global 5.4.8
luavm shell-init powershell
luavm detect --json
luavm exec --project -- lua -v
```

Project-local configuration is stored in `.luavm.toml`:

```toml
lua = "5.4.8"

[build]
cflags = ["-O2"]
ldflags = []
make_args = []
```

Installations live under a LuaVM-owned data root and custom build flags produce distinct isolated profiles.
Official release checksums are obtained from the Lua archive catalog automatically; `--allow-unverified` is required only when an official checksum is unavailable.
