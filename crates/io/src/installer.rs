#![forbid(unsafe_code)]
#![doc = "Download, digest, extraction, and platform build adapters."]

use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use luavm_core::{BuildProfile, LuaVersion};
use luavm_util::{Error, Result};

const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_LISTING_BYTES: usize = 2 * 1024 * 1024;

/// A verified or locally computed archive digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedArchive {
    /// Archive path.
    pub path: PathBuf,
    /// Lowercase SHA-256 digest.
    pub sha256: String,
    /// Whether an official checksum matched.
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
    /// Fetches and parses the official Lua release index.
    pub fn available_versions(&self) -> Result<Vec<LuaVersion>> {
        let output = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--max-redirs",
                "3",
                "--max-filesize",
                "2097152",
                "https://www.lua.org/ftp/",
            ])
            .output()
            .map_err(|error| Error::NotFound(format!("curl is required for release discovery: {error}")))?;
        if !output.status.success() {
            return Err(Error::Process(
                "unable to fetch the official Lua release index".to_owned(),
            ));
        }
        let html = String::from_utf8(output.stdout)
            .map_err(|_| Error::InvalidInput("official release index is not UTF-8".to_owned()))?;
        let mut versions = BTreeSet::new();
        for token in
            html.split(|character: char| !character.is_ascii_alphanumeric() && character != '.' && character != '-')
        {
            let Some(version_text) = token.strip_prefix("lua-") else {
                continue;
            };
            let Some(version_text) = version_text.strip_suffix(".tar.gz") else {
                continue;
            };
            if let Ok(version) = LuaVersion::parse(version_text) {
                versions.insert(version);
            }
        }
        Ok(versions.into_iter().collect())
    }

    /// Downloads an HTTPS archive to a temporary path and verifies its digest.
    pub fn download(
        &self,
        url: &str,
        destination: &Path,
        official_sha256: Option<&str>,
        allow_unverified: bool,
    ) -> Result<DownloadedArchive> {
        validate_https_url(url)?;
        let parent = destination
            .parent()
            .ok_or_else(|| Error::InvalidInput("archive destination has no parent".to_owned()))?;
        fs::create_dir_all(parent).map_err(io_error)?;
        let max_bytes = MAX_ARCHIVE_BYTES.to_string();
        let destination_text = destination.to_string_lossy().into_owned();
        let output = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--tlsv1.2",
                "--max-redirs",
                "3",
                "--max-filesize",
                max_bytes.as_str(),
                "--output",
                destination_text.as_str(),
                url,
            ])
            .output()
            .map_err(|error| Error::NotFound(format!("curl is required for downloads: {error}")))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let _ = fs::remove_file(destination);
            return Err(Error::Process(if detail.is_empty() {
                format!("curl exited with {}", output.status)
            } else {
                detail
            }));
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
                    return Err(Error::Conflict("official SHA-256 checksum mismatch".to_owned()));
                }
                true
            },
            None if !allow_unverified => {
                let _ = fs::remove_file(destination);
                return Err(Error::Conflict(
                    "no official checksum is available; pass explicit unverified confirmation".to_owned(),
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
        fs::create_dir_all(destination).map_err(io_error)?;
        let archive_text = archive.to_string_lossy().into_owned();
        let destination_text = destination.to_string_lossy().into_owned();
        let listing = Command::new("tar")
            .args(["-tzf", archive_text.as_str()])
            .output()
            .map_err(|error| Error::NotFound(format!("tar is required for extraction: {error}")))?;
        if !listing.status.success() {
            return Err(Error::Process("unable to inspect source archive".to_owned()));
        }
        if listing.stdout.len() > MAX_LISTING_BYTES {
            return Err(Error::InvalidInput("archive listing exceeds the size limit".to_owned()));
        }
        for raw_name in String::from_utf8_lossy(&listing.stdout).lines() {
            validate_archive_path(raw_name)?;
        }
        let details = Command::new("tar")
            .args(["-tvzf", archive_text.as_str()])
            .output()
            .map_err(|error| Error::Process(error.to_string()))?;
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
        let extracted = Command::new("tar")
            .args(["-xzf", archive_text.as_str(), "-C", destination_text.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| Error::Process(error.to_string()))?;
        if !extracted.status.success() {
            let detail = String::from_utf8_lossy(&extracted.stderr).trim().to_owned();
            return Err(Error::Process(if detail.is_empty() {
                "tar failed to extract the archive".to_owned()
            } else {
                detail
            }));
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
        let (tool, target) = platform_build_tool();
        let mut command = Command::new(&tool);
        command.current_dir(source).arg(target);
        command.args(&profile.make_args);
        command.arg("install");
        command.arg(format!("INSTALL_TOP={}", install_root.display()));
        if !profile.cflags.is_empty() {
            command.env("MYCFLAGS", profile.cflags.join(" "));
        }
        if !profile.ldflags.is_empty() {
            command.env("MYLDFLAGS", profile.ldflags.join(" "));
        }
        let output = command
            .output()
            .map_err(|error| Error::NotFound(format!("build tool `{tool}` is required for Lua {version}: {error}")))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(Error::Process(if detail.is_empty() {
                format!("{tool} failed with {}", output.status)
            } else {
                detail
            }));
        }
        verify_installation(install_root)?;
        Ok(BuildOutcome {
            install_root: install_root.to_path_buf(),
            build_tool: tool,
        })
    }
}

fn platform_build_tool() -> (String, &'static str) {
    if cfg!(windows) {
        ("mingw32-make".to_owned(), "mingw")
    } else if cfg!(target_os = "macos") {
        ("make".to_owned(), "macosx")
    } else {
        ("make".to_owned(), "posix")
    }
}

fn validate_build_values(profile: &BuildProfile) -> Result<()> {
    for value in profile
        .cflags
        .iter()
        .chain(profile.ldflags.iter())
        .chain(profile.make_args.iter())
    {
        if value
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n' | '\t'))
        {
            return Err(Error::InvalidInput(
                "build flags cannot contain NULs, tabs, or newlines".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_https_url(url: &str) -> Result<()> {
    if !url.starts_with("https://") || url.chars().any(|ch| matches!(ch, '\r' | '\n' | '"' | '\'')) {
        return Err(Error::InvalidInput("only clean HTTPS URLs are allowed".to_owned()));
    }
    Ok(())
}

fn validate_archive_path(name: &str) -> Result<()> {
    let path = Path::new(name);
    if path.is_absolute() || name.starts_with('\\') || name.contains(':') {
        return Err(Error::InvalidInput(format!(
            "archive contains an absolute path: {name}"
        )));
    }
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(Error::InvalidInput(format!("archive contains traversal: {name}")));
        }
    }
    Ok(())
}

fn validate_extracted_tree(root: &Path) -> Result<()> {
    let canonical_root = root.canonicalize().map_err(io_error)?;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
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
        }
    }
    Ok(())
}

fn verify_installation(root: &Path) -> Result<()> {
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
        assert!(validate_https_url("https://example.test/a").is_ok());
    }

    #[test]
    fn rejects_archive_traversal() {
        assert!(validate_archive_path("../escape").is_err());
        assert!(validate_archive_path("lua-5.4.8/src/lua.c").is_ok());
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
