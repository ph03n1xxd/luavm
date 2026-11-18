# Troubleshooting

## `doctor` reports a missing tool

Run:

```text
luavm doctor --json
```

Install the missing compiler or make tool, restart the shell if `PATH` changed, and run `luavm doctor` again. On Windows, check that `mingw32-make.exe` and `gcc.exe` or `clang.exe` can be found from the same PowerShell session that runs LuaVM.

LuaVM also invokes `curl` and `tar` for source downloads and extraction. If those commands are not available, install them through the platform toolchain or add them to `PATH`.

## Installation appears idle

Recent LuaVM builds print these phases:

```text
Preparing to install Lua 5.4.8...
Checking the Lua release catalog...
Downloading Lua source...
Extracting and validating Lua source...
Building Lua (this may take a few minutes)...
Finalizing the installation...
```

The C compilation phase can take several minutes on a first install. Keep the terminal open and check the final error if the command exits unsuccessfully.

## `lua` is not found after installation

An install does not modify the parent shell. Run the command through LuaVM:

```text
luavm exec 5.4.8 -- lua -v
```

Or activate the version in PowerShell:

```powershell
Invoke-Expression (luavm env 5.4.8 --shell powershell | Out-String)
lua -v
```

For a persistent `luavm use` wrapper, evaluate `luavm shell-init powershell` from your PowerShell profile.

## No Lua version is selected

Choose an explicit version, set a global version, or add a project file:

```text
luavm exec 5.4.8 -- lua -v
luavm global 5.4.8
luavm detect
```

If using a project file, run `luavm detect` from the project directory or a child directory and inspect any configuration error it reports.

## The version already exists

An installation with the same version and profile is already present. Inspect it with:

```text
luavm list
luavm path 5.4.8
```

Use a different project build profile when you intentionally need different compiler or linker settings.

## Checksum or archive errors

Do not bypass a checksum mismatch. Retry the download and confirm that the requested version and network response are correct. `--allow-unverified` is only for releases for which the official catalog does not publish a checksum.

LuaVM rejects archive traversal, links, oversized archives, and malformed official release data by design.

## Uninstall is refused because the version is active

The active shell is protected from accidental removal:

```powershell
Remove-Item Env:LUAVM_ACTIVE_VERSION -ErrorAction SilentlyContinue
luavm uninstall 5.4.8 --yes
```

If a version directory remains empty after an older uninstall, current LuaVM versions do not report it as installed because `list` requires an actual profile directory.

## Windows makefile errors

Older builds could report duplicate `AIX`/`FreeBSD` make targets or `The syntax of the command is incorrect`. Current Windows builds normalize those case-colliding Lua make targets and install MinGW outputs without invoking Lua's Unix-only install recipe. Rebuild LuaVM from the current source or replace the executable on `PATH` with the latest release binary.
