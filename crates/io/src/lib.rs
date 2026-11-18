#![forbid(unsafe_code)]
#![doc = "Filesystem, configuration discovery, and state adapters for LuaVM."]

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use luavm_core::{LuaVersion, ProfileId, ProjectConfig};
use luavm_util::{Error, OperationId, Result};

pub mod installer;

pub use installer::{BuildOutcome, DownloadedArchive, Installer};

const DATA_ROOT_ENV: &str = "LUAVM_DATA_ROOT";
const STATE_FILE: &str = "state.json";
const PROJECT_FILE: &str = ".luavm.toml";

/// User-local root for all LuaVM-owned data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoot(PathBuf);

impl DataRoot {
    /// Creates a root after checking it is not empty.
    pub fn new(path: PathBuf) -> Result<Self> {
        if path.as_os_str().is_empty() {
            return Err(Error::InvalidInput("data root cannot be empty".to_owned()));
        }
        Ok(Self(path))
    }

    /// Resolves the root from the explicit override or platform conventions.
    pub fn from_environment() -> Result<Self> {
        if let Some(value) = env::var_os(DATA_ROOT_ENV) {
            return Self::new(PathBuf::from(value));
        }
        #[cfg(windows)]
        {
            let base = env::var_os("LOCALAPPDATA")
                .or_else(|| env::var_os("USERPROFILE"))
                .ok_or_else(|| Error::NotFound("LOCALAPPDATA or USERPROFILE is not set".to_owned()))?;
            return Self::new(PathBuf::from(base).join("luavm"));
        }
        #[cfg(target_os = "macos")]
        {
            let home = env::var_os("HOME").ok_or_else(|| Error::NotFound("HOME is not set".to_owned()))?;
            return Self::new(PathBuf::from(home).join("Library/Application Support/luavm"));
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if let Some(value) = env::var_os("XDG_DATA_HOME") {
                return Self::new(PathBuf::from(value).join("luavm"));
            }
            let home = env::var_os("HOME").ok_or_else(|| Error::NotFound("HOME is not set".to_owned()))?;
            return Self::new(PathBuf::from(home).join(".local/share/luavm"));
        }
        #[allow(unreachable_code)]
        Err(Error::Unsupported("host data-root policy is unavailable".to_owned()))
    }

    /// Borrows the root path.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Returns the paths owned by the manager.
    pub fn paths(&self) -> ManagerPaths {
        ManagerPaths { root: self.0.clone() }
    }
}

/// Derived paths beneath a [`DataRoot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerPaths {
    root: PathBuf,
}

impl ManagerPaths {
    /// Returns the global state path.
    pub fn state(&self) -> PathBuf {
        self.root.join(STATE_FILE)
    }

    /// Returns the project-independent installation path.
    pub fn installation(&self, version: LuaVersion, profile: &ProfileId) -> PathBuf {
        self.root
            .join("installations")
            .join("lua")
            .join(version.to_string())
            .join("profiles")
            .join(profile.as_str())
    }

    /// Returns the operation staging path.
    pub fn staging(&self, operation: &OperationId) -> PathBuf {
        self.root.join("staging").join(operation.as_str())
    }

    /// Returns the path for the manager lock.
    pub fn lock(&self) -> PathBuf {
        self.root.join("operation.lock")
    }

    /// Creates the directories that are safe to initialize eagerly.
    pub fn ensure_base(&self) -> Result<()> {
        for directory in [
            self.root.clone(),
            self.root.join("installations/lua"),
            self.root.join("downloads"),
            self.root.join("staging"),
            self.root.join("logs"),
        ] {
            fs::create_dir_all(&directory).map_err(io_error)?;
        }
        Ok(())
    }
}

/// Global selection persisted by LuaVM.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GlobalState {
    /// Globally selected version.
    pub version: Option<LuaVersion>,
    /// Optional profile selected globally.
    pub profile: Option<ProfileId>,
}

/// Reads global selection state. The file is intentionally tiny and migration-friendly.
pub fn read_global_state(paths: &ManagerPaths) -> Result<GlobalState> {
    let path = paths.state();
    if !path.exists() {
        return Ok(GlobalState::default());
    }
    let mut file = File::open(&path).map_err(io_error)?;
    let mut content = String::new();
    file.read_to_string(&mut content).map_err(io_error)?;
    let lua = json_field(&content, "lua")?;
    let profile = json_field(&content, "profile")?;
    Ok(GlobalState {
        version: lua.map(|value| LuaVersion::parse(&value)).transpose()?,
        profile: profile.map(ProfileId::new).transpose()?,
    })
}

/// Atomically writes global selection state.
pub fn write_global_state(paths: &ManagerPaths, state: &GlobalState) -> Result<()> {
    paths.ensure_base()?;
    let temporary = paths.state().with_extension("tmp");
    let mut file = File::create(&temporary).map_err(io_error)?;
    let version = state
        .version
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "null".to_owned());
    let profile = state
        .profile
        .as_ref()
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "null".to_owned());
    writeln!(file, "{{\"schema\":1,\"lua\":{version},\"profile\":{profile}}}").map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    fs::rename(&temporary, paths.state()).map_err(io_error)
}

/// Finds the nearest project configuration by walking toward the filesystem root.
pub fn discover_project_file(start: &Path) -> Result<Option<PathBuf>> {
    let start = if start.is_dir() {
        start.to_path_buf()
    } else {
        start
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| Error::InvalidInput("project start has no parent".to_owned()))?
    };
    let mut current = fs::canonicalize(start).map_err(io_error)?;
    loop {
        let candidate = current.join(PROJECT_FILE);
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
        let Some(parent) = current.parent() else {
            return Ok(None);
        };
        if parent == current {
            return Ok(None);
        }
        current = parent.to_path_buf();
    }
}

/// Loads the nearest project configuration.
pub fn discover_project_config(start: &Path) -> Result<Option<(PathBuf, ProjectConfig)>> {
    let Some(path) = discover_project_file(start)? else {
        return Ok(None);
    };
    let content = fs::read_to_string(&path).map_err(io_error)?;
    Ok(Some((path, ProjectConfig::parse(&content)?)))
}

/// Creates an exclusive operation lock.
pub fn acquire_lock(paths: &ManagerPaths) -> Result<OperationLock> {
    paths.ensure_base()?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(paths.lock())
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::Conflict("another LuaVM operation is already running".to_owned())
            } else {
                io_error(error)
            }
        })?;
    Ok(OperationLock {
        path: paths.lock(),
        _file: file,
    })
}

/// An exclusive lock released when dropped.
#[derive(Debug)]
pub struct OperationLock {
    path: PathBuf,
    _file: File,
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Creates an operation ID without external dependencies.
pub fn new_operation_id() -> Result<OperationId> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Io("system clock is before the Unix epoch".to_owned()))?
        .as_nanos();
    OperationId::new(format!("op-{nanos:x}"))
}

/// Reads the host PATH as separate entries.
pub fn path_entries(path: Option<&str>) -> Vec<PathBuf> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    path.unwrap_or_default()
        .split(separator)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn io_error(error: std::io::Error) -> Error {
    Error::Io(error.to_string())
}

fn json_field(content: &str, key: &str) -> Result<Option<String>> {
    let marker = format!("\"{key}\"");
    let Some(start) = content.find(&marker) else {
        return Ok(None);
    };
    let after_key = content
        .get(start.saturating_add(marker.len())..)
        .ok_or_else(|| Error::InvalidInput("state JSON is truncated".to_owned()))?
        .trim_start();
    let value = after_key
        .strip_prefix(':')
        .ok_or_else(|| Error::InvalidInput("state JSON field is missing a colon".to_owned()))?
        .trim_start();
    if value.starts_with("null") {
        return Ok(None);
    }
    let quoted = value
        .strip_prefix('"')
        .ok_or_else(|| Error::InvalidInput("state JSON field must be a string or null".to_owned()))?;
    let end = quoted
        .find('"')
        .ok_or_else(|| Error::InvalidInput("state JSON string is unterminated".to_owned()))?;
    let value = quoted
        .get(..end)
        .ok_or_else(|| Error::InvalidInput("state JSON string boundary is invalid".to_owned()))?;
    Ok(Some(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_entries_ignores_empty_segments() {
        let separator = if cfg!(windows) { ';' } else { ':' };
        let value = format!("a{separator}{separator}b");
        assert_eq!(path_entries(Some(&value)).len(), 2);
    }

    #[test]
    fn state_is_valid_json_and_round_trips() {
        let root = DataRoot::new(std::env::temp_dir().join("luavm-state-test"));
        let Ok(root) = root else {
            return;
        };
        let paths = root.paths();
        let version = LuaVersion::parse("5.4.8");
        let Ok(version) = version else {
            return;
        };
        let profile = ProfileId::new("p-test");
        let Ok(profile) = profile else {
            return;
        };
        let state = GlobalState {
            version: Some(version),
            profile: Some(profile),
        };
        if write_global_state(&paths, &state).is_err() {
            return;
        }
        let content = fs::read_to_string(paths.state());
        assert!(content.as_ref().is_ok_and(|value| value.starts_with('{')));
        assert_eq!(read_global_state(&paths), Ok(state));
        let _ = fs::remove_dir_all(root.path());
    }
}
