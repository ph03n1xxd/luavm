# Command reference

Run `luavm help` for the compact built-in reference. The examples below use an explicitly selected version where that makes the behavior clear.

## Live Windows example

The following session was run with LuaVM 0.1.0 on Windows using an isolated temporary `LUAVM_DATA_ROOT`. Paths are shortened to `<temp-root>` below.

### Version, help, and prerequisites

```powershell
PS> luavm version
luavm 0.1.0

PS> luavm help
LuaVM is a lightweight Lua version manager.

Usage: luavm <command> [options]

PS> luavm doctor
[ok] mingw32-make: available
[ok] C compiler (gcc or clang): available

PS> luavm doctor --json
{"schema":1,"ready":true,"prerequisites":[{"name":"mingw32-make","available":true,"hint":"Install MinGW-w64 and add mingw32-make to PATH"},{"name":"C compiler (gcc or clang)","available":true,"hint":"Install MinGW-w64 or LLVM and add it to PATH"}]}

PS> luavm doctor --fix
[ok] mingw32-make: ready
[ok] C compiler (gcc or clang): ready
```

Because the prerequisites were already healthy, `doctor --fix` only reported their status and did not install host packages. On an incomplete host, it prints proposed package-manager commands and asks for confirmation.

### Remote catalog and installation

```powershell
PS> luavm list --available
1.0.0
1.1.0
2.1.0
2.2.0
2.4.0
2.5.0
3.0.0
3.1.0
...

PS> luavm list --available --json
{"schema":1,"available":["1.0.0","1.1.0","2.1.0", ...]}

PS> luavm install 5.4.7 --no-prerequisites
Preparing to install Lua 5.4.7...
Checking the Lua release catalog...
Downloading Lua source...
Extracting and validating Lua source...
Building Lua (this may take a few minutes)...
Finalizing the installation...
Lua 5.4.7 installed
profile: pa03c61af686d6ce8
path: <temp-root>\installations\lua\5.4.7\profiles\pa03c61af686d6ce8
sha256: 9fbf5e28ef86c69858f6d3d34eccc32e911c1a28b4120ff3e84aaa70cfbf1e30
```

### Installed versions and global selection

```powershell
PS> luavm list
5.4.7

PS> luavm list --json
{"schema":1,"installed":["5.4.7"]}

PS> luavm path 5.4.7
<temp-root>\installations\lua\5.4.7\profiles\pa03c61af686d6ce8

PS> luavm global 5.4.7
global Lua version set to 5.4.7

PS> luavm current
version: 5.4.7
source: global
profile: pa03c61af686d6ce8
installed: true

PS> luavm current --json
{"schema":1,"version":"5.4.7","source":"global","config_path":null,"profile":"pa03c61af686d6ce8","installed":true}

PS> luavm use 5.4.7
Lua 5.4.7 selected for this shell
run `eval "$(luavm env 5.4.7)"` in POSIX shells
installation: <temp-root>\installations\lua\5.4.7\profiles\pa03c61af686d6ce8
```

### Environment and execution

```powershell
PS> luavm env 5.4.7 --shell powershell
$env:PATH = '<temp-root>\installations\lua\5.4.7\profiles\pa03c61af686d6ce8\bin' + [IO.Path]::PathSeparator + $env:PATH
$env:LUAVM_ACTIVE_VERSION = '5.4.7'

PS> luavm exec 5.4.7 -- lua -v
Lua 5.4.7  Copyright (C) 1994-2024 Lua.org, PUC-Rio
```

### Project-local selection

The live run created `.luavm.toml` in a temporary project, entered a nested directory, and ran:

```powershell
PS> luavm local 5.4.7
project Lua version set to 5.4.7 in <temp-root>\project\.luavm.toml

PS> luavm detect
version: 5.4.7
source: project
config: <temp-root>\project\.luavm.toml
profile: pa03c61af686d6ce8
installed: true

PS> luavm detect --json
{"schema":1,"version":"5.4.7","source":"project","profile":"pa03c61af686d6ce8","installed":true}

PS> luavm env --project --shell powershell
$env:PATH = '<temp-root>\installations\lua\5.4.7\profiles\pa03c61af686d6ce8\bin' + [IO.Path]::PathSeparator + $env:PATH
$env:LUAVM_ACTIVE_VERSION = '5.4.7'

PS> luavm exec --project -- lua -v
Lua 5.4.7  Copyright (C) 1994-2024 Lua.org, PUC-Rio

PS> luavm local --unset
project Lua version cleared
```

### Shell setup and cleanup

```powershell
PS> luavm shell-init powershell
function luavm {
  if ($args.Count -gt 0 -and $args[0] -eq 'use') {
    $envText = & luavm.exe env $args[1..($args.Count - 1)]
    Invoke-Expression ($envText -join [Environment]::NewLine)
  } else {
    & luavm.exe @args
  }
}

PS> luavm completions powershell
Register-ArgumentCompleter -CommandName luavm -ScriptBlock { ... }

PS> luavm uninstall 5.4.7 --yes
luavm: conflict: the version is active in this shell; pass --force from a new shell to remove it

PS> luavm uninstall 5.4.7 --yes
Lua 5.4.7 uninstalled

PS> luavm list

PS> luavm global --unset
global Lua version cleared
```

## Discovery and installation

### `list`

```text
luavm list
luavm list --json
luavm list --available
luavm list --available --json
```

The first form lists installed versions. `--available` fetches the remote official Lua catalog. JSON output includes a schema number and either `installed` or `available`.

### `doctor`

```text
luavm doctor
luavm doctor --json
luavm doctor --fix
```

`doctor` checks the build tool and C compiler. `--fix` prints proposed prerequisite commands, asks for confirmation, and runs them through the supported package manager when one is available.

### `install`

```text
luavm install <version>
luavm install <version> --no-prerequisites
luavm install <version> --sha256 <digest>
luavm install <version> --allow-unverified
```

LuaVM resolves the official archive and checksum, downloads it over HTTPS, validates the archive, builds it in staging, verifies the resulting binary, and atomically publishes the profile.

`--no-prerequisites` disables the automatic prerequisite check. `--allow-unverified` is required when the official catalog has no checksum for a release. Supplying `--sha256` adds an explicit digest check.

Build settings can be stored in `.luavm.toml`; see [Configuration and storage](configuration.md#project-configuration).

### `path`

```text
luavm path 5.4.8
luavm path 5.4.8 --profile <profile-id>
```

Prints the installation directory for a version and profile.

## Version selection

### `global`

```text
luavm global 5.4.8
luavm global 5.4.8 --profile <profile-id>
luavm global --unset
```

Stores the default selection in LuaVM state. It does not modify the current shell's `PATH`.

### `local`

```text
luavm local 5.4.8
luavm local 5.4.8 --profile <profile-id>
luavm local --unset
```

Writes `.luavm.toml` in the current directory. The file is discovered from child directories up to the filesystem root.

### `detect` and `current`

```text
luavm detect
luavm detect --json
luavm current
luavm current --json
```

`detect` reports project-aware selection. `current` also considers `LUAVM_ACTIVE_VERSION`, which is set by shell environment output.

Selection precedence is explicit command version, nearest project configuration, global state, then no selection.

### `use`

```text
luavm use 5.4.8
```

`use` validates the installation and prints shell-selection information. A child process cannot change its parent shell, so use a generated shell wrapper, evaluate `env` output, or use `exec` for reliable activation.

## Running commands

### `env`

```text
luavm env 5.4.8 --shell bash
luavm env 5.4.8 --shell zsh
luavm env 5.4.8 --shell fish
luavm env 5.4.8 --shell powershell
luavm env 5.4.8 --shell cmd
luavm env --project --shell powershell
```

`env` prints shell-specific environment changes. It preserves unrelated `PATH` entries and does not permanently edit the host environment.

### `exec`

```text
luavm exec 5.4.8 -- lua script.lua
luavm exec --project -- lua -e "print(_VERSION)"
```

Everything after `--` is the child command and its arguments. LuaVM starts the child directly with the selected Lua `bin` directory prepended to `PATH`; it does not invoke a shell.

## Shell integration

Generate a wrapper or completion definition:

```text
luavm shell-init bash
luavm shell-init zsh
luavm shell-init fish
luavm shell-init powershell
luavm shell-init cmd

luavm completions bash
luavm completions zsh
luavm completions fish
luavm completions powershell
```

PowerShell example:

```powershell
Invoke-Expression (luavm shell-init powershell | Out-String)
luavm use 5.4.8
lua -v
```

## Removal

```text
luavm uninstall 5.4.8 --yes
luavm uninstall 5.4.8 --profile <profile-id> --yes
```

Uninstall is deliberately non-interactive only with `--yes`. A version active in the current shell is protected; clear the shell environment or use `--force` from a controlled context when removal is intentional.
