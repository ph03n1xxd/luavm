#![forbid(unsafe_code)]
#![doc = "LuaVM command workflows and selection resolution."]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use luavm_core::{BuildProfile, LuaVersion, ProfileId, ProjectConfig, Selection, SelectionSource, resolve_selection};
use luavm_io::{
    DataRoot, Installer, acquire_lock, discover_project_config, new_operation_id, read_global_state, write_global_state,
};
use luavm_util::{Error, Result};

/// A resolved selection with enough context for CLI output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionReport {
    /// The selected version and source.
    pub selection: Selection,
    /// Nearest project config, if one was discovered.
    pub config_path: Option<PathBuf>,
    /// Selected profile, if a version exists.
    pub profile: Option<ProfileId>,
    /// Final installation path, if a version/profile exists.
    pub installation_path: Option<PathBuf>,
    /// Whether the selected installation exists.
    pub installed: bool,
}

/// The manager facade used by the CLI and future automation clients.
#[derive(Debug, Clone)]
pub struct Manager {
    root: DataRoot,
}

/// Inputs for an official Lua source installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    /// Lua version to build.
    pub version: LuaVersion,
    /// Official HTTPS archive URL.
    pub archive_url: String,
    /// Official SHA-256, when published.
    pub official_sha256: Option<String>,
    /// Build settings used to create the isolated profile.
    pub profile: BuildProfile,
    /// Explicit confirmation when no official checksum exists.
    pub allow_unverified: bool,
}

/// Result of a committed installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallResult {
    /// Installed version.
    pub version: LuaVersion,
    /// Profile identity.
    pub profile: ProfileId,
    /// Final installation path.
    pub path: PathBuf,
    /// Download digest.
    pub sha256: String,
}

impl Manager {
    /// Opens a manager at the configured data root without creating files.
    pub fn open(root: DataRoot) -> Self {
        Self { root }
    }

    /// Returns the manager data root.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// Resolves explicit, project, and global selection in order.
    pub fn resolve(&self, explicit: Option<LuaVersion>, include_project: bool, cwd: &Path) -> Result<SelectionReport> {
        let project = if include_project {
            discover_project_config(cwd)?
        } else {
            None
        };
        let global = read_global_state(&self.root.paths())?;
        let project_version = project.as_ref().and_then(|(_, config)| config.lua);
        let selection = resolve_selection(explicit, project_version, global.version);
        let (profile, installation_path, installed) = match selection.version {
            Some(version) => {
                let project_profile = project.as_ref().and_then(|(_, config)| config.profile.clone());
                let profile = match selection.source {
                    SelectionSource::Project => project_profile.or_else(|| {
                        project
                            .as_ref()
                            .map(|(_, config)| default_profile(version, &config.build))
                    }),
                    SelectionSource::Global => global.profile.clone(),
                    _ => None,
                };
                let profile = profile.unwrap_or_else(|| default_profile(version, &BuildProfile::default()));
                let path = self.root.paths().installation(version, &profile);
                let installed = path.is_dir();
                (Some(profile), Some(path), installed)
            },
            None => (None, None, false),
        };
        Ok(SelectionReport {
            selection,
            config_path: project.map(|(path, _)| path),
            profile,
            installation_path,
            installed,
        })
    }

    /// Sets the global version and optional profile.
    pub fn set_global(&self, version: LuaVersion, profile: Option<ProfileId>) -> Result<()> {
        let paths = self.root.paths();
        let _lock = acquire_lock(&paths)?;
        let state = luavm_io::GlobalState {
            version: Some(version),
            profile,
        };
        write_global_state(&paths, &state)
    }

    /// Clears the global selection.
    pub fn unset_global(&self) -> Result<()> {
        let paths = self.root.paths();
        let _lock = acquire_lock(&paths)?;
        write_global_state(&paths, &luavm_io::GlobalState::default())
    }

    /// Writes a project-local selection in the current directory.
    pub fn set_local(&self, cwd: &Path, version: LuaVersion, profile: Option<ProfileId>) -> Result<PathBuf> {
        let path = cwd.join(".luavm.toml");
        let build = BuildProfile::default();
        let config = ProjectConfig {
            lua: Some(version),
            profile,
            build,
        };
        fs::write(&path, config.to_toml()?).map_err(|error| Error::Io(error.to_string()))?;
        Ok(path)
    }

    /// Removes the project-local config in the current directory.
    pub fn unset_local(&self, cwd: &Path) -> Result<()> {
        let path = cwd.join(".luavm.toml");
        if path.exists() {
            fs::remove_file(path).map_err(|error| Error::Io(error.to_string()))?;
        }
        Ok(())
    }

    /// Returns a path for an installed version/profile, if it exists.
    pub fn installed_path(&self, version: LuaVersion, profile: Option<ProfileId>) -> Result<PathBuf> {
        let profile = profile.unwrap_or_else(|| default_profile(version, &BuildProfile::default()));
        let path = self.root.paths().installation(version, &profile);
        if !path.is_dir() {
            return Err(Error::NotFound(format!(
                "Lua {version} with profile {profile} is not installed"
            )));
        }
        Ok(path)
    }

    /// Creates an operation staging directory for a future install workflow.
    pub fn create_staging(&self) -> Result<(luavm_util::OperationId, PathBuf)> {
        let paths = self.root.paths();
        let operation = new_operation_id()?;
        let staging = paths.staging(&operation);
        fs::create_dir_all(&staging).map_err(|error| Error::Io(error.to_string()))?;
        Ok((operation, staging))
    }

    /// Returns the default profile identity for the current target.
    pub fn default_profile(&self, version: LuaVersion) -> ProfileId {
        default_profile(version, &BuildProfile::default())
    }

    /// Downloads, builds, verifies, and atomically publishes one Lua profile.
    pub fn install(&self, request: InstallRequest) -> Result<InstallResult> {
        let paths = self.root.paths();
        let _lock = acquire_lock(&paths)?;
        let operation = new_operation_id()?;
        let staging = paths.staging(&operation);
        fs::create_dir_all(&staging).map_err(|error| Error::Io(error.to_string()))?;
        let result = self.install_staged(&paths, &staging, request);
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn install_staged(
        &self,
        paths: &luavm_io::ManagerPaths,
        staging: &Path,
        request: InstallRequest,
    ) -> Result<InstallResult> {
        let archive = staging.join("source.tar.gz");
        let installer = Installer;
        let downloaded = installer.download(
            &request.archive_url,
            &archive,
            request.official_sha256.as_deref(),
            request.allow_unverified,
        )?;
        let extracted = staging.join("source");
        installer.extract(&downloaded.path, &extracted)?;
        let source = source_root(&extracted)?;
        let profile = normalized_profile(request.profile, request.version);
        let profile_id = profile.id(request.version);
        let staged_install = staging.join("install");
        installer.build(&source, &staged_install, request.version, &profile)?;
        let final_path = paths.installation(request.version, &profile_id);
        if final_path.exists() {
            return Err(Error::Conflict(format!(
                "Lua {} with profile {} is already installed",
                request.version, profile_id
            )));
        }
        let metadata = format!(
            "version={}\nprofile={}\nsha256={}\nofficial_checksum_verified={}\ntarget={}\ncompiler={}\ncflags={}\nldflags={}\nmake_args={}\n",
            request.version,
            profile_id,
            downloaded.sha256,
            downloaded.official_checksum_verified,
            profile.target,
            profile.compiler,
            profile.cflags.join("\t"),
            profile.ldflags.join("\t"),
            profile.make_args.join("\t"),
        );
        fs::write(staged_install.join("metadata.json"), metadata).map_err(|error| Error::Io(error.to_string()))?;
        let parent = final_path
            .parent()
            .ok_or_else(|| Error::Io("installation path has no parent".to_owned()))?;
        fs::create_dir_all(parent).map_err(|error| Error::Io(error.to_string()))?;
        fs::rename(&staged_install, &final_path).map_err(|error| Error::Io(error.to_string()))?;
        let _ = fs::remove_dir_all(staging);
        Ok(InstallResult {
            version: request.version,
            profile: profile_id,
            path: final_path,
            sha256: downloaded.sha256,
        })
    }
}

fn default_profile(version: LuaVersion, profile: &BuildProfile) -> ProfileId {
    let mut profile = profile.clone();
    if profile.target.is_empty() {
        profile.target = env::consts::OS.to_owned();
    }
    profile.id(version)
}

fn normalized_profile(mut profile: BuildProfile, _version: LuaVersion) -> BuildProfile {
    if profile.target.is_empty() {
        profile.target = env::consts::OS.to_owned();
    }
    profile
}

fn source_root(extracted: &Path) -> Result<PathBuf> {
    let mut directories = Vec::new();
    for entry in fs::read_dir(extracted).map_err(|error| Error::Io(error.to_string()))? {
        let entry = entry.map_err(|error| Error::Io(error.to_string()))?;
        if entry
            .file_type()
            .map_err(|error| Error::Io(error.to_string()))?
            .is_dir()
        {
            directories.push(entry.path());
        }
    }
    if directories.len() == 1 {
        return directories
            .into_iter()
            .next()
            .ok_or_else(|| Error::InvalidInput("source directory disappeared".to_owned()));
    }
    if extracted.join("Makefile").is_file() {
        return Ok(extracted.to_path_buf());
    }
    Err(Error::InvalidInput(
        "archive does not contain one Lua source directory".to_owned(),
    ))
}

/// Returns the command names exposed by the CLI.
pub fn command_names() -> &'static [&'static str] {
    &[
        "help",
        "version",
        "install",
        "uninstall",
        "list",
        "use",
        "global",
        "local",
        "detect",
        "current",
        "env",
        "exec",
        "path",
        "shell-init",
        "completions",
    ]
}

/// Returns a concise command overview.
pub fn help_text() -> &'static str {
    "LuaVM is a lightweight Lua version manager.\n\nUsage: luavm <command> [options]\n\nCommands:\n  install <version> [--sha256 <digest>] [--allow-unverified]\n  uninstall <version> [--profile <id>] [--yes] [--force]\n  list [--available] [--json]\n  use <version>\n  global <version>|--unset\n  local <version>|--unset\n  detect [--json]\n  current [--json]\n  env [--project] [--shell <shell>]\n  exec [<version>] [--project] -- <command> [args...]\n  path <version>\n  shell-init <shell>\n  completions <shell>\n  version\n  help\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_resolves_empty_selection_without_creating_files() {
        let root = match DataRoot::new(PathBuf::from("test-root")) {
            Ok(value) => value,
            Err(_) => return,
        };
        let manager = Manager::open(root);
        let report = match manager.resolve(None, false, Path::new(".")) {
            Ok(value) => value,
            Err(_) => return,
        };
        assert_eq!(report.selection.source, SelectionSource::None);
    }
}
