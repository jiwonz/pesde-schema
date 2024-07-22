use std::{
    collections::HashSet,
    fs::create_dir_all,
    sync::{mpsc::Receiver, Arc, Mutex},
};

use crate::{
    lockfile::{DependencyGraph, DownloadedDependencyGraphNode, DownloadedGraph},
    source::{pesde::PesdePackageSource, PackageRefs, PackageSource, PackageSources},
    Project, PACKAGES_CONTAINER_NAME,
};

type MultithreadedGraph = Arc<Mutex<DownloadedGraph>>;

type MultithreadDownloadJob = (
    Receiver<Result<(), errors::DownloadGraphError>>,
    MultithreadedGraph,
);

impl Project {
    pub fn download_graph(
        &self,
        graph: &DependencyGraph,
        refreshed_sources: &mut HashSet<PackageSources>,
        reqwest: &reqwest::blocking::Client,
        threads: usize,
    ) -> Result<MultithreadDownloadJob, errors::DownloadGraphError> {
        let manifest = self.deser_manifest()?;
        let downloaded_graph: MultithreadedGraph = Arc::new(Mutex::new(Default::default()));

        let threadpool = threadpool::ThreadPool::new(threads);
        let (tx, rx) = std::sync::mpsc::channel();

        for (name, versions) in graph {
            for (version_id, node) in versions {
                let source = match &node.pkg_ref {
                    PackageRefs::Pesde(pkg_ref) => {
                        PackageSources::Pesde(PesdePackageSource::new(pkg_ref.index_url.clone()))
                    }
                };

                if refreshed_sources.insert(source.clone()) {
                    source.refresh(self).map_err(Box::new)?;
                }

                let container_folder = node.container_folder(
                    &self
                        .path()
                        .join(node.base_folder(manifest.target.kind(), true))
                        .join(PACKAGES_CONTAINER_NAME),
                    name,
                    version_id.version(),
                );

                create_dir_all(&container_folder)?;

                let tx = tx.clone();

                let name = name.clone();
                let version_id = version_id.clone();
                let node = node.clone();

                let project = Arc::new(self.clone());
                let reqwest = reqwest.clone();
                let downloaded_graph = downloaded_graph.clone();

                threadpool.execute(move || {
                    let project = project.clone();

                    let target =
                        match source.download(&node.pkg_ref, &container_folder, &project, &reqwest)
                        {
                            Ok(target) => target,
                            Err(e) => {
                                tx.send(Err(e.into())).unwrap();
                                return;
                            }
                        };

                    let mut downloaded_graph = downloaded_graph.lock().unwrap();
                    downloaded_graph
                        .entry(name)
                        .or_default()
                        .insert(version_id, DownloadedDependencyGraphNode { node, target });

                    tx.send(Ok(())).unwrap();
                });
            }
        }

        Ok((rx, downloaded_graph))
    }
}

pub mod errors {
    use thiserror::Error;

    #[derive(Debug, Error)]
    #[non_exhaustive]
    pub enum DownloadGraphError {
        #[error("error deserializing project manifest")]
        ManifestDeserializationFailed(#[from] crate::errors::ManifestReadError),

        #[error("failed to refresh package source")]
        RefreshFailed(#[from] Box<crate::source::errors::RefreshError>),

        #[error("error interacting with filesystem")]
        Io(#[from] std::io::Error),

        #[error("failed to download package")]
        DownloadFailed(#[from] crate::source::errors::DownloadError),
    }
}
