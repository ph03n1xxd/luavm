# luavm
A lightweight, cross-platform Lua version manager written in Rust.

LuaVM manages official Lua releases in isolated user-local directories. LuaJIT, Lua forks, and package managers are outside the scope of this project.

## Current status

The first CLI vertical slice is implemented. Source compilation requires `curl`, `tar`, and a platform C compiler/build tool. LuaVM never installs those prerequisites or modifies system PATH.

```text
luavm list --available
luavm install 5.4.8 --sha256 <official-sha256>
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
