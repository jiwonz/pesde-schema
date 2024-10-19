use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    manifest::{
        target::{Target, TargetKind},
        DependencyType,
    },
    source::{workspace::WorkspacePackageSource, DependencySpecifiers, PackageRef, PackageSources},
};

/// A workspace package reference
#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct WorkspacePackageRef {
    /// The path of the package
    pub path: RelativePathBuf,
    /// The dependencies of the package
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, (DependencySpecifiers, DependencyType)>,
    /// The target of the package
    pub target: Target,
}
impl PackageRef for WorkspacePackageRef {
    fn dependencies(&self) -> &BTreeMap<String, (DependencySpecifiers, DependencyType)> {
        &self.dependencies
    }

    fn use_new_structure(&self) -> bool {
        true
    }

    fn target_kind(&self) -> TargetKind {
        self.target.kind()
    }

    fn source(&self) -> PackageSources {
        PackageSources::Workspace(WorkspacePackageSource)
    }
}
