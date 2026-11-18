#![forbid(unsafe_code)]
#![doc = "Download, digest, extraction, and platform build adapters."]

use std::fmt::Write as FmtWrite;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use luavm_core::{BuildProfile, LuaVersion};
use luavm_util::{Error, Result};

const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_LISTING_BYTES: usize = 2 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(300);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(900);
const BUILD_TIMEOUT: Duration = Duration::from_secs(900);

/// One release record from the Lua archive index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialRelease {
    /// Lua version.
    pub version: LuaVersion,
    /// source archive URL.
    pub archive_url: String,
    /// SHA-256 checksum, when published.
    pub sha256: Option<String>,
}

/// A host prerequisite needed for compiling official Lua sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrerequisiteStatus {
    /// Human-readable prerequisite name.
    pub name: String,
    /// Whether the executable was found and responded successfully.
    pub available: bool,
    /// Platform-specific installation guidance.
    pub hint: String,
}

/// A verified or locally computed archive digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedArchive {
    /// Archive path.
    pub path: PathBuf,
    /// Lowercase SHA-256 digest.
    pub sha256: String,
    /// Whether an checksum matched.
    pub official_checksum_verified: bool,
}

/// Result of a successful source build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOutcome {
    /// Final staging destination containing the installation.
    pub install_root: PathBuf,
    /// Build executable used.
    pub build_tool: String,
}

/// External-tool adapter for source download and compilation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Installer;

impl Installer {
    /// Checks the host tools required for a source build.
    pub fn prerequisites(&self) -> Vec<PrerequisiteStatus> {
        let (build_tool, _, build_hint) = platform_build_tool();
        let build_available = command_available(&build_tool);
        let (compiler_name, compiler_available, compiler_hint) = if cfg!(windows) {
            (
                "C compiler (gcc or clang)".to_owned(),
                command_available("gcc") || command_available("clang"),
                "Install MinGW-w64 or LLVM and add it to PATH".to_owned(),
            )
        } else {
            (
                "C compiler (cc)".to_owned(),
                command_available("cc"),
                "Install the platform C compiler toolchain".to_owned(),
            )
        };
        vec![
            PrerequisiteStatus {
                name: build_tool,
                available: build_available,
                hint: build_hint.to_owned(),
            },
            PrerequisiteStatus {
                name: compiler_name,
                available: compiler_available,
                hint: compiler_hint,
            },
        ]
    }

    /// Fetches and parses the Lua release index.
    pub fn available_releases(&self) -> Result<Vec<OfficialRelease>> {
        let mut command = Command::new("curl");
        command.args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "15",
            "--max-time",
            "60",
            "--max-filesize",
            "2097152",
            "https://www.lua.org/ftp/",
        ]);
        let output = run_command(&mut command, COMMAND_TIMEOUT, MAX_LISTING_BYTES).map_err(|error| match error {
            Error::NotFound(detail) => Error::NotFound(format!("curl is required for release discovery: {detail}")),
            other => other,
        })?;
        if output.truncated {
            return Err(Error::InvalidInput("release index exceeds the size limit".to_owned()));
        }
        if !output.status.success() {
            return Err(Error::Process(command_failure_detail("curl", &output)));
        }
        let html = String::from_utf8(output.stdout)
            .map_err(|_| Error::InvalidInput("release index is not UTF-8".to_owned()))?;
        parse_release_index(&html)
    }

    /// Fetches the record for one Lua version.
    pub fn release(&self, version: LuaVersion) -> Result<OfficialRelease> {
        self.available_releases()?
            .into_iter()
            .find(|release| release.version == version)
            .ok_or_else(|| Error::NotFound(format!("Lua release {version} was not found")))
    }

    /// Fetches and parses only the Lua release versions.
    pub fn available_versions(&self) -> Result<Vec<LuaVersion>> {
        Ok(self
            .available_releases()?
            .into_iter()
            .map(|release| release.version)
            .collect())
    }

    /*
     * Keep the public release catalog separate from the download operation so
     * callers can display or test metadata without downloading bytes.
     */
    /// Downloads an HTTPS archive to a temporary path and verifies its digest.
    pub fn download(
        &self,
        url: &str,
        destination: &Path,
        expected_version: LuaVersion,
        official_sha256: Option<&str>,
        allow_unverified: bool,
    ) -> Result<DownloadedArchive> {
        let archive_version = validate_https_url(url)?;
        if archive_version != expected_version {
            return Err(Error::InvalidInput(format!(
                "archive URL version {archive_version} does not match requested Lua {expected_version}"
            )));
        }
        let parent = destination
            .parent()
            .ok_or_else(|| Error::InvalidInput("archive destination has no parent".to_owned()))?;
        fs::create_dir_all(parent).map_err(io_error)?;
        let max_bytes = MAX_ARCHIVE_BYTES.to_string();
        let destination_text = destination.to_string_lossy().into_owned();
        let mut command = Command::new("curl");
        command.args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "15",
            "--max-time",
            "900",
            "--retry",
            "2",
            "--retry-delay",
            "1",
            "--max-filesize",
            max_bytes.as_str(),
            "--output",
            destination_text.as_str(),
            url,
        ]);
        let output = run_command(&mut command, DOWNLOAD_TIMEOUT, 64 * 1024).map_err(|error| match error {
            Error::NotFound(detail) => Error::NotFound(format!("curl is required for downloads: {detail}")),
            other => other,
        })?;
        if !output.status.success() {
            let _ = fs::remove_file(destination);
            return Err(Error::Process(command_failure_detail("curl", &output)));
        }
        let size = fs::metadata(destination).map_err(io_error)?.len();
        if size > MAX_ARCHIVE_BYTES {
            let _ = fs::remove_file(destination);
            return Err(Error::InvalidInput(
                "downloaded archive exceeds the size limit".to_owned(),
            ));
        }
        let sha256 = sha256_file(destination)?;
        let official_checksum_verified = match official_sha256 {
            Some(expected) => {
                if !is_sha256(expected) || !constant_time_equal(&sha256, &expected.to_ascii_lowercase()) {
                    let _ = fs::remove_file(destination);
                    return Err(Error::Conflict("SHA-256 checksum mismatch".to_owned()));
                }
                true
            },
            None if !allow_unverified => {
                let _ = fs::remove_file(destination);
                return Err(Error::Conflict(
                    "no checksum is available; pass explicit unverified confirmation".to_owned(),
                ));
            },
            None => false,
        };
        Ok(DownloadedArchive {
            path: destination.to_path_buf(),
            sha256,
            official_checksum_verified,
        })
    }

    /// Validates an archive listing and extracts it into an empty directory.
    pub fn extract(&self, archive: &Path, destination: &Path) -> Result<()> {
        if destination.exists() {
            if !destination.is_dir() {
                return Err(Error::InvalidInput("archive destination is not a directory".to_owned()));
            }
            let mut entries = fs::read_dir(destination).map_err(io_error)?;
            if entries.next().transpose().map_err(io_error)?.is_some() {
                return Err(Error::Conflict("archive destination must be empty".to_owned()));
            }
        } else {
            fs::create_dir_all(destination).map_err(io_error)?;
        }
        let archive_text = archive.to_string_lossy().into_owned();
        let destination_text = destination.to_string_lossy().into_owned();
        let mut listing_command = Command::new("tar");
        listing_command.args(["-tzf", archive_text.as_str()]);
        let listing = run_command(&mut listing_command, COMMAND_TIMEOUT, MAX_LISTING_BYTES)
            .map_err(|error| command_error("tar", "examine source archive", error))?;
        if listing.truncated {
            return Err(Error::InvalidInput("archive listing exceeds the size limit".to_owned()));
        }
        if !listing.status.success() {
            return Err(Error::Process("unable to inspect source archive".to_owned()));
        }
        for raw_name in String::from_utf8_lossy(&listing.stdout).lines() {
            validate_archive_path(raw_name)?;
        }
        let mut details_command = Command::new("tar");
        details_command.args(["-tvzf", archive_text.as_str()]);
        let details = run_command(&mut details_command, COMMAND_TIMEOUT, MAX_LISTING_BYTES)
            .map_err(|error| command_error("tar", "inspect archive entry types", error))?;
        if details.truncated {
            return Err(Error::InvalidInput(
                "archive detail listing exceeds the size limit".to_owned(),
            ));
        }
        if !details.status.success() {
            return Err(Error::Process("unable to inspect archive entry types".to_owned()));
        }
        for line in String::from_utf8_lossy(&details.stdout).lines() {
            if matches!(line.chars().next(), Some('l' | 'h')) {
                return Err(Error::InvalidInput(
                    "symbolic and hard links are not accepted in source archives".to_owned(),
                ));
            }
        }
        let mut extract_command = Command::new("tar");
        extract_command.args(["-xzf", archive_text.as_str(), "-C", destination_text.as_str()]);
        let extracted = run_command(&mut extract_command, COMMAND_TIMEOUT, 64 * 1024)
            .map_err(|error| command_error("tar", "extract source archive", error))?;
        if !extracted.status.success() {
            return Err(Error::Process(command_failure_detail("tar", &extracted)));
        }
        validate_extracted_tree(destination)?;
        Ok(())
    }

    /// Builds an extracted Lua source tree into an isolated install root.
    pub fn build(
        &self,
        source: &Path,
        install_root: &Path,
        version: LuaVersion,
        profile: &BuildProfile,
    ) -> Result<BuildOutcome> {
        validate_build_values(profile)?;
        fs::create_dir_all(install_root).map_err(io_error)?;
        if cfg!(windows) {
            // MinGW make compares target names case-insensitively, unlike the
            // upstream Lua makefile's AIX/aix and FreeBSD/freebsd aliases.
            prepare_windows_makefile(source)?;
        }
        let (tool, target, _) = platform_build_tool();
        let mut command = Command::new(&tool);
        command.current_dir(source).arg(target);
        command.args(&profile.make_args);
        if !cfg!(windows) {
            // Lua's install recipes use Unix mkdir/install commands, which
            // are not available when mingw32-make falls back to cmd.exe.
            command.arg("install");
            command.arg(format!("INSTALL_TOP={}", make_tool_path(install_root)));
        }
        if !profile.cflags.is_empty() {
            command.env("MYCFLAGS", profile.cflags.join(" "));
        }
        if !profile.ldflags.is_empty() {
            command.env("MYLDFLAGS", profile.ldflags.join(" "));
        }
        let output = run_command(&mut command, BUILD_TIMEOUT, 256 * 1024).map_err(|error| match error {
            Error::NotFound(detail) => {
                Error::NotFound(format!("build tool `{tool}` is required for Lua {version}: {detail}"))
            },
            Error::Process(detail) => Error::Process(format!("{tool} failed for Lua {version}: {detail}")),
            other => other,
        })?;
        if !output.status.success() {
            return Err(Error::Process(command_failure_detail(&tool, &output)));
        }
        if cfg!(windows) {
            // Install the MinGW outputs without invoking Lua's Unix-only
            // install recipe.
            install_windows_artifacts(source, install_root, version)?;
        }
        verify_installation(install_root, version)?;
        Ok(BuildOutcome {
            install_root: install_root.to_path_buf(),
            build_tool: tool,
        })
    }
}

fn parse_release_index(html: &str) -> Result<Vec<OfficialRelease>> {
    let tokens = html
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '.' && character != '-')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut releases = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let Some(version_text) = token.strip_prefix("lua-") else {
            continue;
        };
        let Some(version_text) = version_text.strip_suffix(".tar.gz") else {
            continue;
        };
        let Ok(version) = LuaVersion::parse(version_text) else {
            continue;
        };
        let mut checksum = None;
        for candidate in tokens.iter().skip(index.saturating_add(1)).take(32) {
            if candidate != token && candidate.starts_with("lua-") && candidate.ends_with(".tar.gz") {
                break;
            }
            if is_sha256(candidate) {
                checksum = Some((*candidate).to_ascii_lowercase());
                break;
            }
        }
        releases.push(OfficialRelease {
            version,
            archive_url: format!("https://www.lua.org/ftp/{token}"),
            sha256: checksum,
        });
    }
    releases.sort_by_key(|release| release.version);
    releases.dedup_by(|left, right| left.version == right.version);
    if releases.is_empty() {
        return Err(Error::InvalidInput("release index contains no Lua archives".to_owned()));
    }
    Ok(releases)
}

fn run_command(command: &mut Command, timeout: Duration, max_output: usize) -> Result<CapturedOutput> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error::NotFound(error.to_string()))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err(Error::Process("command stdout was not available".to_owned()));
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        return Err(Error::Process("command stderr was not available".to_owned()));
    };
    let stdout_reader = thread::spawn(move || read_bounded(stdout, max_output));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, max_output));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(io_error)? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(Error::Process(format!(
                    "command timed out after {} seconds",
                    timeout.as_secs()
                )));
            },
            None => thread::sleep(Duration::from_millis(25)),
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| Error::Process("stdout reader failed".to_owned()))?
        .map_err(io_error)?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| Error::Process("stderr reader failed".to_owned()))?
        .map_err(io_error)?;
    Ok(CapturedOutput {
        status,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        truncated: stdout.truncated || stderr.truncated,
    })
}

fn read_bounded<R: Read>(mut reader: R, max_output: usize) -> std::io::Result<BoundedOutput> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = max_output.saturating_sub(bytes.len());
        let stored = count.min(remaining);
        let chunk = buffer
            .get(..stored)
            .ok_or_else(|| std::io::Error::other("bounded command output range failed"))?;
        bytes.extend_from_slice(chunk);
        if stored < count {
            truncated = true;
        }
    }
    Ok(BoundedOutput { bytes, truncated })
}

#[derive(Debug)]
struct BoundedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

#[derive(Debug)]
struct CapturedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

fn command_error(tool: &str, operation: &str, error: Error) -> Error {
    match error {
        Error::NotFound(detail) => Error::NotFound(format!("{tool} is required to {operation}: {detail}")),
        other => other,
    }
}

fn command_failure_detail(tool: &str, output: &CapturedOutput) -> String {
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if detail.is_empty() {
        format!("{tool} exited with {}", output.status)
    } else {
        detail
    }
}

fn platform_build_tool() -> (String, &'static str, &'static str) {
    if cfg!(windows) {
        (
            "mingw32-make".to_owned(),
            "mingw",
            "Install MinGW-w64 and add mingw32-make to PATH",
        )
    } else if cfg!(target_os = "macos") {
        ("make".to_owned(), "macosx", "Install Xcode Command Line Tools")
    } else {
        ("make".to_owned(), "posix", "Install the system make package")
    }
}

fn make_tool_path(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    if cfg!(windows) { value.replace('\\', "/") } else { value }
}

fn prepare_windows_makefile(source: &Path) -> Result<()> {
    let makefile = source.join("src").join("Makefile");
    let contents = fs::read_to_string(&makefile).map_err(io_error)?;
    let normalized = normalize_windows_makefile(&contents);
    if normalized != contents {
        fs::write(makefile, normalized).map_err(io_error)?;
    }
    Ok(())
}

fn normalize_windows_makefile(contents: &str) -> String {
    contents
        .replace("AIX aix:", "aix:")
        .replace("FreeBSD NetBSD OpenBSD freebsd:", "NetBSD OpenBSD freebsd:")
}

fn install_windows_artifacts(source: &Path, install_root: &Path, version: LuaVersion) -> Result<()> {
    let source = source.join("src");
    let bin = install_root.join("bin");
    let include = install_root.join("include");
    let lib = install_root.join("lib");
    for directory in [&bin, &include, &lib] {
        fs::create_dir_all(directory).map_err(io_error)?;
    }
    for name in ["lua.exe", "luac.exe"] {
        copy_required_file(&source.join(name), &bin.join(name))?;
    }
    let dll_name = format!("lua{}{}.dll", version.major, version.minor);
    copy_required_file(&source.join(&dll_name), &bin.join(&dll_name))?;
    for name in ["lua.h", "luaconf.h", "lualib.h", "lauxlib.h", "lua.hpp"] {
        copy_required_file(&source.join(name), &include.join(name))?;
    }
    copy_required_file(&source.join("liblua.a"), &lib.join("liblua.a"))
}

fn copy_required_file(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_file() {
        return Err(Error::Process(format!("built file is missing: {}", source.display())));
    }
    fs::copy(source, destination).map_err(io_error)?;
    Ok(())
}

fn command_available(command_name: &str) -> bool {
    let mut command = Command::new(command_name);
    command.arg("--version");
    run_command(&mut command, Duration::from_secs(10), 4096)
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn validate_build_values(profile: &BuildProfile) -> Result<()> {
    for value in profile
        .cflags
        .iter()
        .chain(profile.ldflags.iter())
        .chain(profile.make_args.iter())
    {
        if value.chars().any(|character| {
            matches!(
                character,
                '\0' | '\r' | '\n' | '\t' | ';' | '&' | '|' | '$' | '`' | '<' | '>' | '"' | '\'' | '%' | '^'
            )
        }) {
            return Err(Error::InvalidInput(
                "build values cannot contain control characters or shell metacharacters".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_https_url(url: &str) -> Result<LuaVersion> {
    let Some(name) = url.strip_prefix("https://www.lua.org/ftp/") else {
        return Err(Error::InvalidInput(
            "only official Lua HTTPS archive URLs are allowed".to_owned(),
        ));
    };
    let Some(version_text) = name
        .strip_prefix("lua-")
        .and_then(|value| value.strip_suffix(".tar.gz"))
    else {
        return Err(Error::InvalidInput(
            "only official Lua source archives are allowed".to_owned(),
        ));
    };
    let version = LuaVersion::parse(version_text);
    if version.is_err() || url.chars().any(|ch| matches!(ch, '\r' | '\n' | '"' | '\'' | '?' | '#')) {
        return Err(Error::InvalidInput("only clean HTTPS URLs are allowed".to_owned()));
    }
    version
}

fn validate_archive_path(name: &str) -> Result<()> {
    let normalized = name.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() || normalized.starts_with('/') || normalized.contains(':') {
        return Err(Error::InvalidInput(format!(
            "archive contains an absolute path: {name}"
        )));
    }
    for component in normalized.split('/') {
        if component == ".." {
            return Err(Error::InvalidInput(format!("archive contains traversal: {name}")));
        }
    }
    Ok(())
}

fn validate_extracted_tree(root: &Path) -> Result<()> {
    let canonical_root = root.canonicalize().map_err(io_error)?;
    let mut pending = vec![root.to_path_buf()];
    let mut entries = 0_usize;
    let mut expanded_bytes = 0_u64;
    while let Some(path) = pending.pop() {
        entries = entries
            .checked_add(1)
            .ok_or_else(|| Error::InvalidInput("archive entry count overflowed".to_owned()))?;
        if entries > MAX_ARCHIVE_ENTRIES {
            return Err(Error::InvalidInput("archive contains too many entries".to_owned()));
        }
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path).map_err(io_error)?;
            let base = path
                .parent()
                .ok_or_else(|| Error::InvalidInput("archive link has no parent".to_owned()))?
                .canonicalize()
                .map_err(io_error)?;
            let resolved = base.join(target).canonicalize().map_err(io_error)?;
            if !resolved.starts_with(&canonical_root) {
                return Err(Error::InvalidInput("archive link escapes staging".to_owned()));
            }
            continue;
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(io_error)? {
                pending.push(entry.map_err(io_error)?.path());
            }
        } else if metadata.is_file() {
            expanded_bytes = expanded_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| Error::InvalidInput("expanded archive size overflowed".to_owned()))?;
            if expanded_bytes > MAX_EXPANDED_BYTES {
                return Err(Error::InvalidInput(
                    "expanded archive exceeds the size limit".to_owned(),
                ));
            }
        } else {
            return Err(Error::InvalidInput(
                "archive contains an unsupported file type".to_owned(),
            ));
        }
    }
    Ok(())
}

fn verify_installation(root: &Path, version: LuaVersion) -> Result<()> {
    let binary = if cfg!(windows) {
        root.join("bin/lua.exe")
    } else {
        root.join("bin/lua")
    };
    if !binary.is_file() {
        return Err(Error::Process(format!(
            "built Lua binary is missing: {}",
            binary.display()
        )));
    }
    let binary = binary.canonicalize().map_err(io_error)?;
    let mut command = Command::new(binary);
    command.arg("-v");
    let output = run_command(&mut command, COMMAND_TIMEOUT, 64 * 1024)?;
    if !output.status.success() {
        return Err(Error::Process(format!("built Lua {version} failed its version check")));
    }
    let mut reported = String::from_utf8_lossy(&output.stdout).into_owned();
    reported.push_str(&String::from_utf8_lossy(&output.stderr));
    if !reported.contains(&format!("Lua {version}")) {
        return Err(Error::Conflict(format!(
            "built binary reported an unexpected Lua version: {}",
            reported.trim()
        )));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut state = Sha256::new();
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        state.update(
            buffer
                .get(..count)
                .ok_or_else(|| Error::Io("digest buffer range failed".to_owned()))?,
        );
    }
    Ok(state.finish())
}

#[derive(Debug, Clone)]
struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    length: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: Vec::with_capacity(64),
            length: 0,
        }
    }

    fn update(&mut self, input: &[u8]) {
        self.length = self.length.saturating_add(input.len() as u64);
        self.buffer.extend_from_slice(input);
        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self
                .buffer
                .drain(..64)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or([0; 64]);
            self.compress(&block);
        }
    }

    fn finish(mut self) -> String {
        let bit_length = self.length.saturating_mul(8);
        self.buffer.push(0x80);
        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0);
        }
        self.buffer.extend_from_slice(&bit_length.to_be_bytes());
        while let Some(block_vec) = take_block(&mut self.buffer) {
            let block: [u8; 64] = block_vec.try_into().unwrap_or([0; 64]);
            self.compress(&block);
        }
        let mut output = String::with_capacity(64);
        for word in self.state {
            let _ = write!(&mut output, "{word:08x}");
        }
        output
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
            0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
            0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
            0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
            0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
            0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
            0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut schedule = [0_u32; 64];
        for (index, slot) in schedule.iter_mut().enumerate().take(16) {
            let offset = index.saturating_mul(4);
            let bytes = block.get(offset..offset.saturating_add(4)).unwrap_or(&[0; 4]);
            *slot = u32::from_be_bytes([
                bytes.first().copied().unwrap_or(0),
                bytes.get(1).copied().unwrap_or(0),
                bytes.get(2).copied().unwrap_or(0),
                bytes.get(3).copied().unwrap_or(0),
            ]);
        }
        for index in 16_usize..64_usize {
            let a = schedule_value(&schedule, index.saturating_sub(15));
            let b = schedule_value(&schedule, index.saturating_sub(2));
            let small_sigma0 = a.rotate_right(7) ^ a.rotate_right(18) ^ (a >> 3);
            let small_sigma1 = b.rotate_right(17) ^ b.rotate_right(19) ^ (b >> 10);
            let value = schedule_value(&schedule, index.saturating_sub(16))
                .wrapping_add(small_sigma0)
                .wrapping_add(schedule_value(&schedule, index.saturating_sub(7)))
                .wrapping_add(small_sigma1);
            if let Some(slot) = schedule.get_mut(index) {
                *slot = value;
            }
        }
        let mut working = self.state;
        for index in 0..64 {
            let [a, b, c, d, e, f, g, h] = working;
            let big_sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(big_sigma1)
                .wrapping_add(choice)
                .wrapping_add(K.get(index).copied().unwrap_or(0))
                .wrapping_add(schedule_value(&schedule, index));
            let big_sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = big_sigma0.wrapping_add(majority);
            working = [
                temporary1.wrapping_add(temporary2),
                a,
                b,
                c,
                d.wrapping_add(temporary1),
                e,
                f,
                g,
            ];
        }
        for (state, value) in self.state.iter_mut().zip(working) {
            *state = state.wrapping_add(value);
        }
    }
}

fn schedule_value(schedule: &[u32; 64], index: usize) -> u32 {
    schedule.get(index).copied().unwrap_or(0)
}

fn take_block(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    if buffer.len() < 64 {
        return None;
    }
    Some(buffer.drain(..64).collect())
}

fn io_error(error: std::io::Error) -> Error {
    Error::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn accepts_only_https_urls() {
        assert!(validate_https_url("http://example.test/a").is_err());
        assert!(validate_https_url("https://example.test/a").is_err());
        assert_eq!(
            validate_https_url("https://www.lua.org/ftp/lua-5.4.8.tar.gz"),
            LuaVersion::parse("5.4.8")
        );
    }

    #[test]
    fn rejects_archive_traversal() {
        assert!(validate_archive_path("../escape").is_err());
        assert!(validate_archive_path(r"..\escape").is_err());
        assert!(validate_archive_path("/absolute").is_err());
        assert!(validate_archive_path("lua-5.4.8/src/lua.c").is_ok());
    }

    #[test]
    fn rejects_shell_metacharacters_in_build_values() {
        let profile = BuildProfile {
            cflags: vec!["-DNAME=$(touch pwned)".to_owned()],
            ..BuildProfile::default()
        };
        assert!(validate_build_values(&profile).is_err());
    }

    #[test]
    fn formats_paths_for_the_platform_build_tool() {
        let path = if cfg!(windows) {
            Path::new(r"D:\a\_temp\luavm\install")
        } else {
            Path::new("/tmp/luavm/install")
        };
        let formatted = make_tool_path(path);
        if cfg!(windows) {
            assert_eq!(formatted, "D:/a/_temp/luavm/install");
        } else {
            assert_eq!(formatted, "/tmp/luavm/install");
        }
    }

    #[test]
    fn removes_case_colliding_lua_make_aliases_for_windows_make() {
        let makefile = "AIX aix:\n\t$(MAKE) all\nFreeBSD NetBSD OpenBSD freebsd:\n\t$(MAKE) all\n";
        let normalized = normalize_windows_makefile(makefile);
        assert_eq!(
            normalized,
            "aix:\n\t$(MAKE) all\nNetBSD OpenBSD freebsd:\n\t$(MAKE) all\n"
        );
        assert_eq!(normalize_windows_makefile(&normalized), normalized);
    }

    #[test]
    fn parses_official_release_checksum() {
        let html = r#"
            <A HREF="lua-5.4.8.tar.gz">lua-5.4.8.tar.gz</A>
            <TD>2025-05-21</TD><TD>374332</TD>
            <TD>4f18ddae154e793e46eeab727c59ef1c0c0c2b744e7b94219710d76f530629ae</TD>
        "#;
        let releases = parse_release_index(html);
        assert_eq!(releases.as_ref().map(Vec::len), Ok(1));
        let release = releases.ok().and_then(|mut values| values.pop());
        assert_eq!(
            release.as_ref().and_then(|value| value.sha256.as_deref()),
            Some("4f18ddae154e793e46eeab727c59ef1c0c0c2b744e7b94219710d76f530629ae")
        );
    }

    #[test]
    fn sha256_matches_known_empty_value() {
        let path = std::env::temp_dir().join("luavm-sha256-empty");
        let Ok(mut file) = File::create(&path) else {
            return;
        };
        if file.write_all(b"").is_err() {
            let _ = fs::remove_file(&path);
            return;
        }
        let Ok(digest) = sha256_file(&path) else {
            let _ = fs::remove_file(&path);
            return;
        };
        let _ = fs::remove_file(path);
        assert_eq!(
            digest,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_matches_known_text_value() {
        let mut state = Sha256::new();
        state.update(b"abc");
        assert_eq!(
            state.finish(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
