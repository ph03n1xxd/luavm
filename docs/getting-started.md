# Getting started

LuaVM builds official Lua source releases locally and installs each version into a private LuaVM data root. It does not replace the operating system's Lua executable or permanently edit `PATH`.

## Install LuaVM

Download the archive for your platform from the [latest GitHub release](https://github.com/ph03n1xxd/luavm/releases/latest). Verify the archive against `SHA256SUMS`, extract the executable, and place it on `PATH`.

The published artifacts currently target:

| Platform | Archive |
| --- | --- |
| Linux x86_64 | `luavm-linux-x86_64.tar.gz` |
| macOS x86_64 | `luavm-macos-x86_64.tar.gz` |
| Windows x86_64 | `luavm-windows-x86_64.zip` |

Check the installation:

```text
luavm version
```

## Prepare the host

LuaVM needs a compiler, a make implementation, `curl`, and `tar` to build an official Lua source archive.

Check the compiler and make tool:

```text
luavm doctor
```

On Windows, add `mingw32-make` and either `gcc` or `clang` to `PATH`. MinGW-w64 or LLVM are suitable toolchains. On macOS, install Xcode Command Line Tools. On Linux, install the distribution's standard C build package and make implementation.

If the supported package manager is available, LuaVM can propose prerequisite commands:

```text
luavm doctor --fix
```

Review the commands and confirm them interactively. Restart the shell if the package manager changed `PATH`.

## Install a Lua release

Discover releases published by the official Lua archive:

```text
luavm list --available
```

Install one release:

```text
luavm install 5.4.8
```

The command reports the active phase. A successful install prints the profile ID, final path, and archive digest.

Verify the result without changing the current shell:

```text
luavm exec 5.4.8 -- lua -v
```

Installations are side by side, so a second release can be installed without replacing the first:

```text
luavm install 5.4.7
luavm exec 5.4.7 -- lua -v
luavm exec 5.4.8 -- lua -v
```

## Use Lua in PowerShell

For a single command, prefer `exec`:

```powershell
luavm exec 5.4.8 -- lua script.lua
```

To put a version on `PATH` in the current PowerShell process:

```powershell
Invoke-Expression (luavm env 5.4.8 --shell powershell | Out-String)
lua -v
```

For the `luavm use 5.4.8` experience, install the shell wrapper in the current session:

```powershell
Invoke-Expression (luavm shell-init powershell | Out-String)
luavm use 5.4.8
lua -v
```

To load the wrapper in every new PowerShell session, put the same `Invoke-Expression` line in `$PROFILE`.

## Use Lua in other shells

LuaVM can emit environment changes for Bash, Zsh, Fish, PowerShell, and Command Prompt:

```text
luavm env 5.4.8 --shell bash
luavm env 5.4.8 --shell zsh
luavm env 5.4.8 --shell fish
luavm env 5.4.8 --shell powershell
luavm env 5.4.8 --shell cmd
```

The output is intended to be evaluated by the calling shell. See [Command reference](commands.md#shell-integration) for shell-specific examples.
