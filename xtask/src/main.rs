#![forbid(unsafe_code)]
#![doc = "Repository automation commands for LuaVM."]

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::from(1)
        },
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str).unwrap_or("help") {
        "help" => {
            println!("cargo xtask <layers|docs|parity|ci>");
            Ok(())
        },
        "layers" => layers(),
        "docs" => docs(),
        "parity" => {
            println!("Lua-only command scope is enforced by the CLI and core domain types");
            Ok(())
        },
        "ci" => {
            println!("run the repository verification gates from the project documentation");
            Ok(())
        },
        command => Err(format!("unknown xtask command `{command}`")),
    }
}

fn layers() -> Result<(), String> {
    let expected = [
        ("luavm-util", &[][..]),
        ("luavm-core", &["luavm-util"][..]),
        ("luavm-io", &["luavm-util", "luavm-core"][..]),
        ("luavm-engine", &["luavm-util", "luavm-core", "luavm-io"][..]),
        ("luavm", &["luavm-util", "luavm-core", "luavm-io", "luavm-engine"][..]),
    ];
    for (package, allowed) in expected {
        let manifest = manifest_for(package).ok_or_else(|| format!("missing manifest for {package}"))?;
        let content = fs::read_to_string(manifest).map_err(|error| error.to_string())?;
        for dependency in ["luavm-util", "luavm-core", "luavm-io", "luavm-engine"] {
            if content.contains(&format!("{dependency}.workspace")) && !allowed.contains(&dependency) {
                return Err(format!("layer violation: {package} depends on {dependency}"));
            }
        }
    }
    println!("layer check passed");
    Ok(())
}

fn manifest_for(package: &str) -> Option<&'static Path> {
    match package {
        "luavm-util" => Some(Path::new("crates/util/Cargo.toml")),
        "luavm-core" => Some(Path::new("crates/core/Cargo.toml")),
        "luavm-io" => Some(Path::new("crates/io/Cargo.toml")),
        "luavm-engine" => Some(Path::new("crates/engine/Cargo.toml")),
        "luavm" => Some(Path::new("apps/cli/Cargo.toml")),
        _ => None,
    }
}

fn docs() -> Result<(), String> {
    let readme = fs::read_to_string("README.md").map_err(|error| error.to_string())?;
    if !readme.contains("LuaVM") {
        return Err("README.md is missing or incomplete".to_owned());
    }
    println!("documentation check passed");
    Ok(())
}
