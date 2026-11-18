#![forbid(unsafe_code)]
#![doc = "Command-line entry point for LuaVM."]

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use luavm_core::{BuildProfile, LuaVersion, ProfileId};
use luavm_engine::{InstallRequest, InstallStage, Manager, SelectionReport, help_text};
use luavm_io::{DataRoot, Installer, read_global_state};
use luavm_util::{Error, Result};

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("luavm: {}", error);
            ExitCode::from(1)
        },
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let command = args.first().map(String::as_str).unwrap_or("help");
    match command {
        "help" | "--help" | "-h" => {
            print!("{}", help_text());
            return Ok(());
        },
        "version" | "--version" | "-V" => {
            println!("luavm 0.1.0");
            return Ok(());
        },
        "doctor" => return doctor(&args),
        "shell-init" => return shell_init(&args),
        "completions" => return completions(&args),
        _ => {},
    }
    let root = DataRoot::from_environment()?;
    let manager = Manager::open(root);
    let cwd = env::current_dir().map_err(|error| Error::Io(error.to_string()))?;
    match command {
        "detect" => detect(&manager, &cwd, args.iter().any(|value| value == "--json")),
        "current" => current(&manager, &cwd, args.iter().any(|value| value == "--json")),
        "global" => global(&manager, &args),
        "local" => local(&manager, &cwd, &args),
        "path" => path(&manager, &args),
        "use" => use_version(&manager, &args),
        "env" => environment(&manager, &cwd, &args),
        "exec" => execute(&manager, &cwd, &args),
        "list" => list(
            &manager,
            args.iter().any(|value| value == "--json"),
            args.iter().any(|value| value == "--available"),
        ),
        "uninstall" => uninstall(&manager, &args),
        "install" => install(&manager, &args),
        _ => Err(Error::InvalidInput(format!(
            "unknown command `{command}`; run `luavm help`"
        ))),
    }
}

fn parse_lua_version(value: &str) -> Result<LuaVersion> {
    if matches!(value.to_ascii_lowercase().as_str(), "luajit" | "luarocks") {
        return Err(Error::Unsupported(
            "LuaVM supports official Lua releases only".to_owned(),
        ));
    }
    LuaVersion::parse(value)
}

fn parse_version(args: &[String]) -> Result<LuaVersion> {
    let value = args
        .get(1)
        .ok_or_else(|| Error::InvalidInput("a Lua version is required".to_owned()))?;
    parse_lua_version(value)
}

fn parse_profile(args: &[String]) -> Result<Option<ProfileId>> {
    let Some(position) = args.iter().position(|value| value == "--profile") else {
        return Ok(None);
    };
    let value = args
        .get(position + 1)
        .ok_or_else(|| Error::InvalidInput("--profile requires a profile id".to_owned()))?;
    if value.starts_with('-') {
        return Err(Error::InvalidInput(
            "--profile requires a non-flag profile id".to_owned(),
        ));
    }
    Ok(Some(ProfileId::new(value.clone())?))
}

fn repeated_values(args: &[String], name: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    let mut position = 0_usize;
    while position < args.len() {
        if args.get(position).map(String::as_str) == Some(name) {
            let value = args
                .get(position.saturating_add(1))
                .ok_or_else(|| Error::InvalidInput(format!("{name} requires a value")))?;
            if value.starts_with('-') {
                return Err(Error::InvalidInput(format!("{name} requires a non-flag value")));
            }
            values.push(value.clone());
            position = position.saturating_add(2);
        } else {
            position = position.saturating_add(1);
        }
    }
    Ok(values)
}

fn detect(manager: &Manager, cwd: &Path, json: bool) -> Result<()> {
    let report = manager.resolve(None, true, cwd)?;
    print_report(&report, json)
}

fn doctor(args: &[String]) -> Result<()> {
    let json = args.iter().any(|value| value == "--json");
    let fix = args.iter().any(|value| value == "--fix");
    if json && fix {
        return Err(Error::InvalidInput("--json and --fix cannot be combined".to_owned()));
    }
    let statuses = Installer.prerequisites();
    let healthy = statuses.iter().all(|status| status.available);
    print_doctor_status(&statuses, json);
    if healthy {
        return Ok(());
    }
    if !fix {
        return Err(Error::NotFound(
            "source builds require the missing host prerequisites shown above; rerun with --fix to opt in to installation".to_owned(),
        ));
    }
    let steps = prerequisite_install_steps();
    if steps.is_empty() {
        return Err(Error::NotFound(
            "no supported package manager was found; install the missing prerequisites manually".to_owned(),
        ));
    }
    println!("LuaVM will run these prerequisite installation commands:");
    for (program, arguments) in &steps {
        println!("  {} {}", program, arguments.join(" "));
    }
    print!("Continue? [y/N] ");
    io::stdout().flush().map_err(|error| Error::Io(error.to_string()))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| Error::Io(error.to_string()))?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Err(Error::Conflict("prerequisite installation was cancelled".to_owned()));
    }
    for (program, arguments) in steps {
        let status = Command::new(&program)
            .args(&arguments)
            .status()
            .map_err(|error| Error::Process(format!("unable to run {program}: {error}")))?;
        if !status.success() {
            return Err(Error::Process(format!("{program} exited with {status}")));
        }
    }
    let statuses = Installer.prerequisites();
    let healthy = statuses.iter().all(|status| status.available);
    print_doctor_status(&statuses, false);
    if healthy {
        Ok(())
    } else {
        Err(Error::NotFound(
            "prerequisite installation completed; restart the shell if PATH changed, then rerun `luavm doctor`"
                .to_owned(),
        ))
    }
}

fn print_doctor_status(statuses: &[luavm_io::installer::PrerequisiteStatus], json: bool) {
    if json {
        let values = statuses
            .iter()
            .map(|status| {
                format!(
                    "{{\"name\":{},\"available\":{},\"hint\":{}}}",
                    json_string(Some(status.name.as_str())),
                    status.available,
                    json_string(Some(status.hint.as_str()))
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let ready = statuses.iter().all(|status| status.available);
        println!("{{\"schema\":1,\"ready\":{},\"prerequisites\":[{values}]}}", ready);
    } else {
        for status in statuses {
            let marker = if status.available { "ok" } else { "missing" };
            println!("[{marker}] {} — {}", status.name, status.hint);
        }
    }
}

fn prerequisite_install_steps() -> Vec<(String, Vec<String>)> {
    if cfg!(windows) && command_exists("winget") {
        return vec![(
            "winget".to_owned(),
            vec![
                "install".to_owned(),
                "--exact".to_owned(),
                "--id".to_owned(),
                "BrechtSanders.WinLibs.POSIX.UCRT".to_owned(),
                "--accept-source-agreements".to_owned(),
                "--accept-package-agreements".to_owned(),
            ],
        )];
    }
    if cfg!(target_os = "macos") {
        return vec![("/usr/bin/xcode-select".to_owned(), vec!["--install".to_owned()])];
    }
    if cfg!(unix) {
        if command_exists("apt-get") {
            return vec![
                ("sudo".to_owned(), vec!["apt-get".to_owned(), "update".to_owned()]),
                (
                    "sudo".to_owned(),
                    vec![
                        "apt-get".to_owned(),
                        "install".to_owned(),
                        "-y".to_owned(),
                        "build-essential".to_owned(),
                    ],
                ),
            ];
        }
        if command_exists("dnf") {
            return vec![(
                "sudo".to_owned(),
                vec![
                    "dnf".to_owned(),
                    "install".to_owned(),
                    "-y".to_owned(),
                    "make".to_owned(),
                    "gcc".to_owned(),
                ],
            )];
        }
        if command_exists("pacman") {
            return vec![(
                "sudo".to_owned(),
                vec![
                    "pacman".to_owned(),
                    "-S".to_owned(),
                    "--needed".to_owned(),
                    "--noconfirm".to_owned(),
                    "make".to_owned(),
                    "gcc".to_owned(),
                ],
            )];
        }
    }
    Vec::new()
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn current(manager: &Manager, cwd: &Path, json: bool) -> Result<()> {
    let active = env::var("LUAVM_ACTIVE_VERSION")
        .ok()
        .map(|value| parse_lua_version(&value))
        .transpose()?;
    let report = manager.resolve(active, true, cwd)?;
    if report.selection.version.is_none() && !json {
        println!("no Lua version selected");
        return Ok(());
    }
    print_report(&report, json)
}

fn global(manager: &Manager, args: &[String]) -> Result<()> {
    if args.iter().any(|value| value == "--unset") {
        manager.unset_global()?;
        println!("global Lua version cleared");
        return Ok(());
    }
    let version = parse_version(args)?;
    let profile = parse_profile(args)?;
    manager.set_global(version, profile)?;
    println!("global Lua version set to {version}");
    Ok(())
}

fn local(manager: &Manager, cwd: &Path, args: &[String]) -> Result<()> {
    if args.iter().any(|value| value == "--unset") {
        manager.unset_local(cwd)?;
        println!("project Lua version cleared");
        return Ok(());
    }
    let version = parse_version(args)?;
    let profile = parse_profile(args)?;
    let path = manager.set_local(cwd, version, profile)?;
    println!("project Lua version set to {version} in {}", path.display());
    Ok(())
}

fn path(manager: &Manager, args: &[String]) -> Result<()> {
    let version = parse_version(args)?;
    let profile = parse_profile(args)?;
    println!("{}", manager.installed_path(version, profile)?.display());
    Ok(())
}

fn install(manager: &Manager, args: &[String]) -> Result<()> {
    let version = parse_version(args)?;
    println!("Preparing to install Lua {version}...");
    ensure_prerequisites(args)?;
    let cflags = repeated_values(args, "--cflag")?;
    let ldflags = repeated_values(args, "--ldflag")?;
    let make_args = repeated_values(args, "--make-arg")?;
    let explicit_sha256 = option_value(args, "--sha256")?;
    let allow_unverified = args.iter().any(|value| value == "--allow-unverified");
    println!("Checking the Lua release catalog...");
    let release = Installer.release(version)?;
    if let (Some(expected), Some(actual)) = (explicit_sha256.as_deref(), release.sha256.as_deref())
        && !expected.eq_ignore_ascii_case(actual)
    {
        return Err(Error::Conflict(
            "the supplied checksum does not match the official release catalog".to_owned(),
        ));
    }
    let official_sha256 = explicit_sha256.or(release.sha256);
    if official_sha256.is_none() && !allow_unverified {
        return Err(Error::InvalidInput(
            "this release has no official checksum; pass --allow-unverified explicitly".to_owned(),
        ));
    }
    let request = InstallRequest {
        version,
        archive_url: release.archive_url,
        official_sha256,
        profile: BuildProfile {
            cflags,
            ldflags,
            make_args,
            compiler: String::new(),
            target: String::new(),
        },
        allow_unverified,
    };
    let result = manager.install_with_progress(request, |stage| {
        println!("{}", install_stage_message(stage));
    })?;
    println!("Lua {} installed", result.version);
    println!("profile: {}", result.profile);
    println!("path: {}", result.path.display());
    println!("sha256: {}", result.sha256);
    Ok(())
}

fn install_stage_message(stage: InstallStage) -> &'static str {
    match stage {
        InstallStage::Downloading => "Downloading Lua source...",
        InstallStage::Extracting => "Extracting and validating Lua source...",
        InstallStage::Building => "Building Lua (this may take a few minutes)...",
        InstallStage::Finalizing => "Finalizing the installation...",
    }
}

fn ensure_prerequisites(args: &[String]) -> Result<()> {
    let statuses = Installer.prerequisites();
    if statuses.iter().all(|status| status.available) {
        return Ok(());
    }
    if args.iter().any(|value| value == "--no-prerequisites") {
        return Err(Error::NotFound(
            "source build prerequisites are missing; run `luavm doctor --fix`".to_owned(),
        ));
    }
    let fix_args = vec!["doctor".to_owned(), "--fix".to_owned()];
    doctor(&fix_args)
}

fn shell_init(args: &[String]) -> Result<()> {
    let shell = args.get(1).map(String::as_str).unwrap_or("bash");
    match shell {
        "bash" | "zsh" => println!(
            "luavm() {{\n  if [ \"$1\" = \"use\" ]; then\n    shift\n    eval \"$(command luavm env \"$@\")\"\n  else\n    command luavm \"$@\"\n  fi\n}}"
        ),
        "fish" => println!(
            "function luavm\n  if test \"$argv[1]\" = use\n    set -e argv[1]\n    command luavm env $argv | source\n  else\n    command luavm $argv\n  end\nend"
        ),
        "powershell" => println!(
            "function luavm {{\n  if ($args.Count -gt 0 -and $args[0] -eq 'use') {{\n    $envText = & luavm.exe env $args[1..($args.Count - 1)]\n    Invoke-Expression ($envText -join [Environment]::NewLine)\n  }} else {{\n    & luavm.exe @args\n  }}\n}}"
        ),
        "cmd" => println!("rem Use: for /f \"delims=\" %i in ('luavm env --shell cmd') do @%i"),
        _ => return Err(Error::InvalidInput(format!("unsupported shell `{shell}`"))),
    }
    Ok(())
}

fn completions(args: &[String]) -> Result<()> {
    let shell = args.get(1).map(String::as_str).unwrap_or("bash");
    match shell {
        "bash" => println!(
            "_luavm() {{ COMPREPLY=( $(compgen -W 'help version doctor install uninstall list use global local detect current env exec path shell-init completions' -- \"${{COMP_WORDS[1]}}\") ); }}\ncomplete -F _luavm luavm"
        ),
        "zsh" => println!(
            "#compdef luavm\n_arguments '1:command:(help version doctor install uninstall list use global local detect current env exec path shell-init completions)'"
        ),
        "fish" => println!(
            "complete -c luavm -f -a 'help version doctor install uninstall list use global local detect current env exec path shell-init completions'"
        ),
        "powershell" => println!(
            "Register-ArgumentCompleter -CommandName luavm -ScriptBlock {{ param($wordToComplete) 'help','version','doctor','install','uninstall','list','use','global','local','detect','current','env','exec','path','shell-init','completions' | Where-Object {{ $_ -like \"$wordToComplete*\" }} }}"
        ),
        _ => return Err(Error::InvalidInput(format!("unsupported shell `{shell}`"))),
    }
    Ok(())
}

fn use_version(manager: &Manager, args: &[String]) -> Result<()> {
    let version = parse_version(args)?;
    let profile = parse_profile(args)?;
    let path = manager.installed_path(version, profile)?;
    println!("Lua {version} selected for this shell");
    println!("run `eval \"$(luavm env {version})\"` in POSIX shells");
    println!("installation: {}", path.display());
    Ok(())
}

fn environment(manager: &Manager, cwd: &Path, args: &[String]) -> Result<()> {
    let explicit = args
        .get(1)
        .filter(|value| !value.starts_with('-'))
        .map(|value| parse_lua_version(value))
        .transpose()?;
    let report = manager.resolve(explicit, args.iter().any(|value| value == "--project"), cwd)?;
    let path = report
        .installation_path
        .ok_or_else(|| Error::NotFound("no Lua version is selected".to_owned()))?;
    if !report.installed {
        return Err(Error::NotFound(format!(
            "selected Lua installation is missing: {}",
            path.display()
        )));
    }
    let shell = option_value(args, "--shell")?.unwrap_or_else(|| "bash".to_owned());
    let bin = path.join("bin");
    let current_path = env::var("PATH").unwrap_or_default();
    print_shell_environment(&shell, &bin, &current_path, report.selection.version)
}

fn execute(manager: &Manager, cwd: &Path, args: &[String]) -> Result<()> {
    let separator = args
        .iter()
        .position(|value| value == "--")
        .ok_or_else(|| Error::InvalidInput("exec requires `--` before the child command".to_owned()))?;
    let command = args
        .get(separator + 1)
        .ok_or_else(|| Error::InvalidInput("exec requires a child command".to_owned()))?;
    let explicit = args
        .get(1)
        .filter(|value| !value.starts_with('-'))
        .map(|value| parse_lua_version(value))
        .transpose()?;
    let report = manager.resolve(explicit, args.iter().any(|value| value == "--project"), cwd)?;
    let path = report
        .installation_path
        .ok_or_else(|| Error::NotFound("no Lua version is selected".to_owned()))?;
    if !report.installed {
        return Err(Error::NotFound(format!(
            "selected Lua installation is missing: {}",
            path.display()
        )));
    }
    let bin = path.join("bin");
    let current_path = env::var("PATH").unwrap_or_default();
    let new_path = prepend_path(&bin, &current_path);
    let status = Command::new(command)
        .args(args.get(separator.saturating_add(2)..).unwrap_or_default())
        .env("PATH", new_path)
        .status()
        .map_err(|error| Error::Process(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Process(format!("child command exited with {status}")))
    }
}

fn list(manager: &Manager, json: bool, available: bool) -> Result<()> {
    if available {
        let versions = Installer.available_versions()?;
        if json {
            let values = versions
                .iter()
                .map(|version| format!("\"{version}\""))
                .collect::<Vec<_>>()
                .join(",");
            println!("{{\"schema\":1,\"available\":[{values}]}}", values = values);
        } else {
            for version in versions {
                println!("{version}");
            }
        }
        return Ok(());
    }
    let root = manager.root().join("installations/lua");
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if json {
                println!("{{\"schema\":1,\"installed\":[]}}");
            }
            return Ok(());
        },
        Err(error) => return Err(Error::Io(error.to_string())),
    };
    let mut versions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error::Io(error.to_string()))?;
        if !entry
            .file_type()
            .map_err(|error| Error::Io(error.to_string()))?
            .is_dir()
        {
            continue;
        }
        if let Ok(version) = LuaVersion::parse(&entry.file_name().to_string_lossy()) {
            versions.push(version);
        }
    }
    versions.sort();
    if json {
        let values = versions
            .iter()
            .map(|version| format!("\"{version}\""))
            .collect::<Vec<_>>()
            .join(",");
        println!("{{\"schema\":1,\"installed\":[{values}]}}", values = values);
    } else {
        for version in versions {
            println!("{version}");
        }
    }
    Ok(())
}

fn uninstall(manager: &Manager, args: &[String]) -> Result<()> {
    let version = parse_version(args)?;
    let profile = parse_profile(args)?;
    if !args.iter().any(|value| value == "--yes") {
        return Err(Error::InvalidInput(
            "uninstall requires --yes in this non-interactive CLI".to_owned(),
        ));
    }
    let path = manager.installed_path(version, profile.clone())?;
    let active = env::var("LUAVM_ACTIVE_VERSION").ok();
    let version_text = version.to_string();
    if active.as_deref() == Some(version_text.as_str()) && !args.iter().any(|value| value == "--force") {
        return Err(Error::Conflict(
            "the version is active in this shell; pass --force from a new shell to remove it".to_owned(),
        ));
    }
    let root = DataRoot::new(manager.root().to_path_buf())?;
    let global = read_global_state(&root.paths())?;
    let clears_global = global.version == Some(version)
        && match (&profile, &global.profile) {
            (Some(selected), Some(global_profile)) => selected == global_profile,
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(_), None) => false,
        };
    let root = manager
        .root()
        .canonicalize()
        .map_err(|error| Error::Io(error.to_string()))?;
    let target = path.canonicalize().map_err(|error| Error::Io(error.to_string()))?;
    if !target.starts_with(&root) {
        return Err(Error::InvalidInput(
            "refusing to remove a path outside the LuaVM data root".to_owned(),
        ));
    }
    fs::remove_dir_all(target).map_err(|error| Error::Io(error.to_string()))?;
    if clears_global {
        manager.unset_global()?;
    }
    println!("Lua {version} uninstalled");
    Ok(())
}

fn print_report(report: &SelectionReport, json: bool) -> Result<()> {
    let version = report.selection.version.map(|value| value.to_string());
    if json {
        println!(
            "{{\"schema\":1,\"version\":{},\"source\":\"{}\",\"config_path\":{},\"profile\":{},\"installed\":{}}}",
            json_string(version.as_deref()),
            report.selection.source,
            json_string(report.config_path.as_deref().and_then(Path::to_str)),
            json_string(report.profile.as_ref().map(ProfileId::as_str)),
            report.installed
        );
    } else {
        println!("version: {}", version.unwrap_or_else(|| "none".to_owned()));
        println!("source: {}", report.selection.source);
        if let Some(path) = &report.config_path {
            println!("config: {}", path.display());
        }
        if let Some(profile) = &report.profile {
            println!("profile: {profile}");
        }
        println!("installed: {}", report.installed);
    }
    Ok(())
}

fn option_value(args: &[String], name: &str) -> Result<Option<String>> {
    let Some(position) = args.iter().position(|value| value == name) else {
        return Ok(None);
    };
    let value = args
        .get(position + 1)
        .ok_or_else(|| Error::InvalidInput(format!("{name} requires a value")))?;
    if value.starts_with('-') {
        return Err(Error::InvalidInput(format!("{name} requires a non-flag value")));
    }
    Ok(Some(value.clone()))
}

fn prepend_path(bin: &Path, current: &str) -> String {
    let separator = if cfg!(windows) { ';' } else { ':' };
    let bin = bin.to_string_lossy().into_owned();
    let entries = current
        .split(separator)
        .filter(|entry| !entry.is_empty() && !same_path(entry, &bin))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        bin
    } else {
        format!("{bin}{separator}{}", entries.join(&separator.to_string()))
    }
}

fn same_path(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn print_shell_environment(shell: &str, bin: &Path, current: &str, version: Option<LuaVersion>) -> Result<()> {
    let bin = bin.to_string_lossy();
    let version = version.ok_or_else(|| Error::NotFound("no Lua version is selected".to_owned()))?;
    match shell {
        "bash" | "zsh" => {
            println!("export PATH={}:\"$PATH\"", posix_quote(&bin));
            println!("export LUAVM_ACTIVE_VERSION={}", posix_quote(&version.to_string()));
        },
        "fish" => {
            println!("set -gx PATH {} $PATH", fish_quote(&bin));
            println!("set -gx LUAVM_ACTIVE_VERSION {}", fish_quote(&version.to_string()));
        },
        "powershell" => {
            println!(
                "$env:PATH = {} + [IO.Path]::PathSeparator + $env:PATH",
                powershell_quote(&bin)
            );
            println!("$env:LUAVM_ACTIVE_VERSION = {}", powershell_quote(&version.to_string()));
        },
        "cmd" => {
            println!("set \"PATH={};%PATH%\"", cmd_quote(&bin));
            println!("set \"LUAVM_ACTIVE_VERSION={version}\"");
        },
        _ => return Err(Error::InvalidInput(format!("unsupported shell `{shell}`"))),
    }
    let _ = current;
    Ok(())
}

fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn cmd_quote(value: &str) -> String {
    value.replace('%', "%%").replace('"', "^\"")
}

fn json_string(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
        None => "null".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_stage_messages_are_not_empty() {
        for stage in [
            InstallStage::Downloading,
            InstallStage::Extracting,
            InstallStage::Building,
            InstallStage::Finalizing,
        ] {
            assert!(!install_stage_message(stage).is_empty());
        }
    }
}
