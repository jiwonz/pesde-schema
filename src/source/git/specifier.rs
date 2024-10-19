use relative_path::RelativePathBuf;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use url::Url;

use crate::source::DependencySpecifier;

/// The specifier for a Git dependency
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub struct GitDependencySpecifier {
    /// The repository of the package
    #[serde(
        serialize_with = "crate::util::serialize_gix_url",
        deserialize_with = "crate::util::deserialize_git_like_url"
    )]
	#[schemars(with = "Url")]
    pub repo: gix::Url,
    /// The revision of the package
    pub rev: String,
    /// The path of the package in the repository
    #[serde(default, skip_serializing_if = "Option::is_none")]
	#[schemars(with = "Option<std::path::PathBuf>")]
    pub path: Option<RelativePathBuf>,
}
impl DependencySpecifier for GitDependencySpecifier {}

impl Display for GitDependencySpecifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.repo, self.rev)
    }
}
