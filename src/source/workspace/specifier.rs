use crate::{manifest::target::TargetKind, names::PackageName, source::DependencySpecifier};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt::Display, str::FromStr};

/// The specifier for a workspace dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub struct WorkspaceDependencySpecifier {
    /// The name of the workspace package
    #[serde(rename = "workspace")]
    pub name: PackageName,
    /// The version type to use when publishing the package
    #[serde(default, rename = "version")]
    pub version: VersionTypeOrReq,
    /// The target of the workspace package
    pub target: Option<TargetKind>,
}
impl DependencySpecifier for WorkspaceDependencySpecifier {}

impl Display for WorkspaceDependencySpecifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "workspace:{}{}", self.version, self.name)
    }
}

/// The type of version to use when publishing a package
#[derive(
    Debug, SerializeDisplay, DeserializeFromStr, Clone, Copy, PartialEq, Eq, Hash, Default, JsonSchema,
)]
pub enum VersionType {
    /// The "^" version type
    #[default]
    Caret,
    /// The "~" version type
    Tilde,
    /// The "=" version type
    Exact,
    /// The "*" version type
    Wildcard,
}

impl Display for VersionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionType::Caret => write!(f, "^"),
            VersionType::Tilde => write!(f, "~"),
            VersionType::Exact => write!(f, "="),
            VersionType::Wildcard => write!(f, "*"),
        }
    }
}

impl FromStr for VersionType {
    type Err = errors::VersionTypeFromStr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "^" => Ok(VersionType::Caret),
            "~" => Ok(VersionType::Tilde),
            "=" => Ok(VersionType::Exact),
            "*" => Ok(VersionType::Wildcard),
            _ => Err(errors::VersionTypeFromStr::InvalidVersionType(
                s.to_string(),
            )),
        }
    }
}

/// Either a version type or a version requirement
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub enum VersionTypeOrReq {
    /// A version type
    VersionType(VersionType),
    /// A version requirement
	#[schemars(with = "semver::Version")]
    Req(semver::VersionReq),
}

impl Default for VersionTypeOrReq {
    fn default() -> Self {
        VersionTypeOrReq::VersionType(VersionType::Caret)
    }
}

impl Display for VersionTypeOrReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionTypeOrReq::VersionType(t) => write!(f, "{t}"),
            VersionTypeOrReq::Req(r) => write!(f, "{r}"),
        }
    }
}

impl FromStr for VersionTypeOrReq {
    type Err = errors::VersionTypeOrReqFromStr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(VersionTypeOrReq::VersionType).or_else(|_| {
            s.parse()
                .map(VersionTypeOrReq::Req)
                .map_err(errors::VersionTypeOrReqFromStr::InvalidVersionReq)
        })
    }
}

/// Errors that can occur when using a version type
pub mod errors {
    use thiserror::Error;

    /// Errors that can occur when parsing a version type
    #[derive(Debug, Error)]
    pub enum VersionTypeFromStr {
        /// The version type is invalid
        #[error("invalid version type {0}")]
        InvalidVersionType(String),
    }

    /// Errors that can occur when parsing a version type or requirement
    #[derive(Debug, Error)]
    pub enum VersionTypeOrReqFromStr {
        /// The version requirement is invalid
        #[error("invalid version requirement {0}")]
        InvalidVersionReq(#[from] semver::Error),
    }
}
