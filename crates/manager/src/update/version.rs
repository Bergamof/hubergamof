//! Release versions: `X.Y.Z`, optionally prefixed by `v` (release tags).

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// A `major.minor.patch` version, ordered numerically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

/// Text that is not a plain `X.Y.Z` version.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("`{0}` is not a X.Y.Z version")]
pub struct ParseVersionError(String);

impl Version {
    /// The version `major.minor.patch`.
    #[must_use]
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Version of the running manager.
    ///
    /// # Errors
    ///
    /// [`ParseVersionError`] when the package version is not `X.Y.Z` (a
    /// pre-release build: it checks nothing).
    pub fn current() -> Result<Self, ParseVersionError> {
        env!("CARGO_PKG_VERSION").parse()
    }
}

impl FromStr for Version {
    type Err = ParseVersionError;

    /// Pre-releases (`1.0.0-rc1`) and build metadata are refused: only stable
    /// releases are ever offered.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || ParseVersionError(text.to_owned());
        let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
        let mut number = || {
            let part = parts.next().ok_or_else(error)?;
            let plain = !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            if plain {
                part.parse().map_err(|_| error())
            } else {
                Err(error())
            }
        };
        let version = Self {
            major: number()?,
            minor: number()?,
            patch: number()?,
        };
        if parts.next().is_some() {
            return Err(error());
        }
        Ok(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    #[test]
    fn parses_with_and_without_the_tag_prefix() {
        assert_eq!(version("1.2.3"), version("v1.2.3"));
        assert_eq!(version("v1.2.3").to_string(), "1.2.3");
    }

    #[test]
    fn compares_numerically_not_textually() {
        assert!(version("1.10.0") > version("1.9.9"));
        assert!(version("2.0.0") > version("1.99.99"));
        assert!(version("1.0.1") > version("1.0.0"));
    }

    #[test]
    fn refuses_anything_but_plain_triplets() {
        for text in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1.2.3-rc1",
            "1.2.3+b",
            "01.2.3",
            "-1.2.3",
            "vv1.2.3",
            "1..3",
            " 1.2.3",
            "99999999999999999999.0.0",
        ] {
            assert!(text.parse::<Version>().is_err(), "{text:?} was accepted");
        }
    }

    #[test]
    fn the_package_version_parses() {
        assert!(Version::current().is_ok());
    }
}
