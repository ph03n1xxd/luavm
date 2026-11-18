#![forbid(unsafe_code)]
#![doc = "Pure LuaVM domain types and deterministic selection logic."]

use std::fmt::{Display, Formatter};
use std::path::PathBuf;

use luavm_util::{Error, Result};

/// A normalized Lua release version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LuaVersion {
    /// Major release component.
    pub major: u32,
    /// Minor release component.
    pub minor: u32,
    /// Patch release component. Two-component input is normalized to zero.
    pub patch: u32,
}

impl LuaVersion {
    /// Parses `M.m` or `M.m.p`, optionally prefixed with `v`.
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.strip_prefix('v').unwrap_or(input);
        if trimmed.is_empty() || trimmed.chars().any(|ch| !ch.is_ascii_digit() && ch != '.') {
            return Err(Error::InvalidInput(format!("invalid Lua version `{input}`")));
        }
        let components: Vec<&str> = trimmed.split('.').collect();
        if components.len() != 2 && components.len() != 3 {
            return Err(Error::InvalidInput(format!(
                "Lua version `{input}` must contain two or three numeric components"
            )));
        }
        let major = parse_component(components.first().copied(), input)?;
        let minor = parse_component(components.get(1).copied(), input)?;
        let patch = match components.get(2).copied() {
            Some(value) => parse_component(Some(value), input)?,
            None => 0,
        };
        Ok(Self { major, minor, patch })
    }

    /// Returns the canonical archive component, such as `5.4.8`.
    pub fn canonical(&self) -> String {
        self.to_string()
    }
}

fn parse_component(component: Option<&str>, input: &str) -> Result<u32> {
    let value = component
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::InvalidInput(format!("invalid Lua version `{input}`")))?;
    value
        .parse::<u32>()
        .map_err(|_| Error::InvalidInput(format!("Lua version component `{value}` is out of range")))
}

impl Display for LuaVersion {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Build settings that influence the produced installation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildProfile {
    /// Compiler flags passed as individual arguments.
    pub cflags: Vec<String>,
    /// Linker flags passed as individual arguments.
    pub ldflags: Vec<String>,
    /// Build-tool arguments passed as individual arguments.
    pub make_args: Vec<String>,
    /// Optional compiler identity supplied by the platform adapter.
    pub compiler: String,
    /// Target triple or platform identity.
    pub target: String,
}

impl BuildProfile {
    /// Returns a stable profile identifier based on normalized build inputs.
    pub fn id(&self, version: LuaVersion) -> ProfileId {
        let mut hasher = Fnv1a::new();
        hasher.write(version.to_string().as_bytes());
        hasher.write(b"\0");
        hasher.write(self.target.trim().as_bytes());
        hasher.write(b"\0");
        hasher.write(self.compiler.trim().as_bytes());
        for values in [&self.cflags, &self.ldflags, &self.make_args] {
            hasher.write(b"\0");
            for value in values {
                hasher.write(value.trim().as_bytes());
                hasher.write(b"\0");
            }
        }
        ProfileId(format!("p{:016x}", hasher.finish()))
    }
}

/// A stable identifier for a version/build profile combination.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProfileId(String);

impl ProfileId {
    /// Constructs a profile ID after validating its path-safe shape.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || value
                .chars()
                .any(|ch| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
        {
            return Err(Error::InvalidInput("invalid build profile id".to_owned()));
        }
        Ok(Self(value))
    }

    /// Borrows the identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ProfileId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy)]
struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn finish(self) -> u64 {
        self.0
    }
}

/// Project-local build settings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectConfig {
    /// Required Lua version, if declared.
    pub lua: Option<LuaVersion>,
    /// Optional pre-built profile selected by this project.
    pub profile: Option<ProfileId>,
    /// Build settings for this project.
    pub build: BuildProfile,
}

impl ProjectConfig {
    /// Parses the supported, deliberately small TOML subset.
    pub fn parse(input: &str) -> Result<Self> {
        let mut config = Self::default();
        let mut section = Section::Root;
        for (line_number, raw_line) in input.lines().enumerate() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if line == "[build]" {
                section = Section::Build;
                continue;
            }
            if line.starts_with('[') {
                return Err(Error::InvalidInput(format!(
                    "unsupported section on line {}",
                    line_number + 1
                )));
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| Error::InvalidInput(format!("expected key=value on line {}", line_number + 1)))?;
            let key = key.trim();
            let value = value.trim();
            match (section, key) {
                (Section::Root, "lua") => config.lua = Some(LuaVersion::parse(&parse_string(value)?)?),
                (Section::Root, "profile") => config.profile = Some(ProfileId::new(parse_string(value)?)?),
                (Section::Root, _) => {
                    return Err(Error::InvalidInput(format!(
                        "unknown project key `{key}` on line {}",
                        line_number + 1
                    )));
                },
                (Section::Build, "cflags") => config.build.cflags = parse_array(value)?,
                (Section::Build, "ldflags") => config.build.ldflags = parse_array(value)?,
                (Section::Build, "make_args") => config.build.make_args = parse_array(value)?,
                (Section::Build, _) => {
                    return Err(Error::InvalidInput(format!(
                        "unknown build key `{key}` on line {}",
                        line_number + 1
                    )));
                },
            }
        }
        if config.lua.is_none() {
            return Err(Error::InvalidInput("project config must declare `lua`".to_owned()));
        }
        Ok(config)
    }

    /// Serializes a project config in the canonical form used by `local`.
    pub fn to_toml(&self) -> Result<String> {
        let lua = self
            .lua
            .ok_or_else(|| Error::InvalidInput("project config must declare `lua`".to_owned()))?;
        let profile = self
            .profile
            .as_ref()
            .map(|value| format!("profile = \"{value}\"\n"))
            .unwrap_or_default();
        Ok(format!(
            "lua = \"{lua}\"\n{profile}\n[build]\ncflags = {}\nldflags = {}\nmake_args = {}\n",
            format_array(&self.build.cflags)?,
            format_array(&self.build.ldflags)?,
            format_array(&self.build.make_args)?
        ))
    }
}

#[derive(Debug, Clone, Copy)]
enum Section {
    Root,
    Build,
}

fn strip_comment(value: &str) -> &str {
    let mut quoted = false;
    for (position, character) in value.char_indices() {
        if character == '"' {
            quoted = !quoted;
        } else if character == '#' && !quoted {
            return value.get(..position).map_or(value, |prefix| prefix);
        }
    }
    value
}

fn parse_string(value: &str) -> Result<String> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(Error::InvalidInput("expected a quoted string".to_owned()));
    }
    let inner = value
        .get(1..value.len().saturating_sub(1))
        .ok_or_else(|| Error::InvalidInput("invalid quoted string".to_owned()))?;
    let mut output = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            let value = match character {
                '"' => '"',
                '\\' => '\\',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                _ => return Err(Error::InvalidInput("unsupported string escape".to_owned())),
            };
            output.push(value);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if matches!(character, '\r' | '\n' | '\0') {
            return Err(Error::InvalidInput(
                "quoted strings cannot contain raw control characters".to_owned(),
            ));
        } else {
            output.push(character);
        }
    }
    if escaped {
        return Err(Error::InvalidInput("quoted string ends with an escape".to_owned()));
    }
    Ok(output)
}

fn parse_array(value: &str) -> Result<Vec<String>> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('[') || !value.ends_with(']') {
        return Err(Error::InvalidInput("expected an array of strings".to_owned()));
    }
    let inner = value
        .get(1..value.len().saturating_sub(1))
        .ok_or_else(|| Error::InvalidInput("invalid string array".to_owned()))?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut segments = Vec::new();
    let mut start = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for (position, character) in inner.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else if character == '"' {
            quoted = true;
        } else if character == ',' {
            segments.push(inner.get(start..position).unwrap_or_default().trim());
            start = position.saturating_add(character.len_utf8());
        }
    }
    if quoted || escaped {
        return Err(Error::InvalidInput("unterminated string in array".to_owned()));
    }
    segments.push(inner.get(start..).unwrap_or_default().trim());
    segments
        .into_iter()
        .map(|segment| {
            if segment.is_empty() {
                return Err(Error::InvalidInput("array contains an empty item".to_owned()));
            }
            parse_string(segment)
        })
        .collect()
}

fn format_array(values: &[String]) -> Result<String> {
    let mut output = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if value.chars().any(|ch| matches!(ch, '\r' | '\n' | '\0')) {
            return Err(Error::InvalidInput(
                "build flags cannot contain raw control characters".to_owned(),
            ));
        }
        if index > 0 {
            output.push_str(", ");
        }
        output.push('"');
        output.push_str(&value.replace('\\', "\\\\").replace('"', "\\\""));
        output.push('"');
    }
    output.push(']');
    Ok(output)
}

/// The origin of the resolved Lua version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    /// A direct CLI argument.
    Explicit,
    /// A discovered project file.
    Project,
    /// The global default.
    Global,
    /// No version was available.
    None,
}

impl Display for SelectionSource {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Explicit => "explicit",
            Self::Project => "project",
            Self::Global => "global",
            Self::None => "none",
        };
        formatter.write_str(value)
    }
}

/// A selected version and the reason it won.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Selected version, if any.
    pub version: Option<LuaVersion>,
    /// Selection source.
    pub source: SelectionSource,
}

/// Applies explicit, project, global, then empty selection precedence.
pub fn resolve_selection(
    explicit: Option<LuaVersion>,
    project: Option<LuaVersion>,
    global: Option<LuaVersion>,
) -> Selection {
    if let Some(version) = explicit {
        return Selection {
            version: Some(version),
            source: SelectionSource::Explicit,
        };
    }
    if let Some(version) = project {
        return Selection {
            version: Some(version),
            source: SelectionSource::Project,
        };
    }
    if let Some(version) = global {
        return Selection {
            version: Some(version),
            source: SelectionSource::Global,
        };
    }
    Selection {
        version: None,
        source: SelectionSource::None,
    }
}

/// A resolved installation location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    /// Lua version.
    pub version: LuaVersion,
    /// Profile identifier.
    pub profile: ProfileId,
    /// Final installation path.
    pub path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_versions() {
        assert_eq!(
            LuaVersion::parse("v5.4").map(|value| value.to_string()),
            Ok("5.4.0".to_owned())
        );
        assert!(LuaVersion::parse("5").is_err());
        assert!(LuaVersion::parse("5.4/evil").is_err());
    }

    #[test]
    fn project_config_parses_build_flags() {
        let config = ProjectConfig::parse(
            "lua = \"5.4.8\"\n[build]\ncflags = [\"-O2\", \"-IC:\\\\Lua\\\\include\", \"-DVALUE=1,2\"]\n",
        );
        assert_eq!(config.as_ref().map(|value| value.build.cflags.len()), Ok(3));
        assert_eq!(
            config.ok().and_then(|value| value.lua).map(|value| value.to_string()),
            Some("5.4.8".to_owned())
        );
    }

    #[test]
    fn project_config_round_trips_escaped_flags() {
        let config = ProjectConfig {
            lua: LuaVersion::parse("5.4.8").ok(),
            build: BuildProfile {
                cflags: vec!["-I/usr/local/include".to_owned(), "-DNAME=\"lua\"".to_owned()],
                ..BuildProfile::default()
            },
            ..ProjectConfig::default()
        };
        let serialized = config.to_toml();
        assert!(serialized.is_ok());
        assert_eq!(
            serialized.ok().and_then(|value| ProjectConfig::parse(&value).ok()),
            Some(config)
        );
    }

    #[test]
    fn explicit_selection_wins() {
        let selection = resolve_selection(
            Some(LuaVersion::parse("5.4.8").unwrap_or(LuaVersion {
                major: 0,
                minor: 0,
                patch: 0,
            })),
            LuaVersion::parse("5.3.6").ok(),
            LuaVersion::parse("5.2.4").ok(),
        );
        assert_eq!(selection.source, SelectionSource::Explicit);
    }
}
