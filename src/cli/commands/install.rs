use crate::cli::{
    bin_dir, download_graph, files::make_executable, run_on_workspace_members, up_to_date_lockfile,
};
use anyhow::Context;
use clap::Args;
use colored::{ColoredString, Colorize};
use indicatif::MultiProgress;
use pesde::{
    lockfile::Lockfile,
    manifest::{target::TargetKind, DependencyType},
    Project, MANIFEST_FILE_NAME,
};
use std::collections::{BTreeSet, HashSet};

#[derive(Debug, Args, Copy, Clone)]
pub struct InstallCommand {
    /// The amount of threads to use for downloading
    #[arg(short, long, default_value_t = 6, value_parser = clap::value_parser!(u64).range(1..=128))]
    threads: u64,

    /// Whether to error on changes in the lockfile
    #[arg(long)]
    locked: bool,

    /// Whether to not install dev dependencies
    #[arg(long)]
    prod: bool,
}

fn bin_link_file(alias: &str) -> String {
    let mut all_combinations = BTreeSet::new();

    for a in TargetKind::VARIANTS {
        for b in TargetKind::VARIANTS {
            all_combinations.insert((a, b));
        }
    }

    let all_folders = all_combinations
        .into_iter()
        .map(|(a, b)| format!("{:?}", a.packages_folder(b)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");

    #[cfg(not(unix))]
    let prefix = String::new();
    #[cfg(unix)]
    let prefix = "#!/usr/bin/env -S lune run\n";
    format!(
        r#"{prefix}local process = require("@lune/process")
local fs = require("@lune/fs")
local stdio = require("@lune/stdio")

local project_root = process.cwd
local path_components = string.split(string.gsub(project_root, "\\", "/"), "/")

for i = #path_components, 1, -1 do
    local path = table.concat(path_components, "/", 1, i)
    if fs.isFile(path .. "/{MANIFEST_FILE_NAME}") then
        project_root = path
        break
    end
end

for _, packages_folder in {{ {all_folders} }} do
    local path = `{{project_root}}/{{packages_folder}}/{alias}.bin.luau`
    
    if fs.isFile(path) then
        require(path)
        return
    end
end

stdio.ewrite(stdio.color("red") .. "binary `{alias}` not found. are you in the right directory?" .. stdio.color("reset") .. "\n")
    "#,
    )
}

#[cfg(feature = "patches")]
const JOBS: u8 = 6;
#[cfg(not(feature = "patches"))]
const JOBS: u8 = 5;

fn job(n: u8) -> ColoredString {
    format!("[{n}/{JOBS}]").dimmed().bold()
}

impl InstallCommand {
    pub fn run(
        self,
        project: Project,
        multi: MultiProgress,
        reqwest: reqwest::blocking::Client,
    ) -> anyhow::Result<()> {
        let mut refreshed_sources = HashSet::new();

        let manifest = project
            .deser_manifest()
            .context("failed to read manifest")?;

        let lockfile = if self.locked {
            match up_to_date_lockfile(&project)? {
                None => {
                    anyhow::bail!(
                        "lockfile is out of sync, run `{} install` to update it",
                        env!("CARGO_BIN_NAME")
                    );
                }
                file => file,
            }
        } else {
            match project.deser_lockfile() {
                Ok(lockfile) => {
                    if lockfile.overrides != manifest.overrides {
                        log::debug!("overrides are different");
                        None
                    } else if lockfile.target != manifest.target.kind() {
                        log::debug!("target kind is different");
                        None
                    } else {
                        Some(lockfile)
                    }
                }
                Err(pesde::errors::LockfileReadError::Io(e))
                    if e.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(e) => return Err(e.into()),
            }
        };

        println!(
            "\n{}\n",
            format!("[now installing {} {}]", manifest.name, manifest.target)
                .bold()
                .on_bright_black()
        );

        println!("{} ❌ removing current package folders", job(1));

        {
            let mut deleted_folders = HashSet::new();

            for target_kind in TargetKind::VARIANTS {
                let folder = manifest.target.kind().packages_folder(target_kind);

                if deleted_folders.insert(folder.to_string()) {
                    log::debug!("deleting the {folder} folder");

                    if let Some(e) = std::fs::remove_dir_all(project.package_dir().join(&folder))
                        .err()
                        .filter(|e| e.kind() != std::io::ErrorKind::NotFound)
                    {
                        return Err(e).context(format!("failed to remove the {folder} folder"));
                    };
                }
            }
        }

        let old_graph = lockfile.map(|lockfile| {
            lockfile
                .graph
                .into_iter()
                .map(|(name, versions)| {
                    (
                        name,
                        versions
                            .into_iter()
                            .map(|(version, node)| (version, node.node))
                            .collect(),
                    )
                })
                .collect()
        });

        println!("{} 📦 building dependency graph", job(2));

        let graph = project
            .dependency_graph(old_graph.as_ref(), &mut refreshed_sources)
            .context("failed to build dependency graph")?;

        let downloaded_graph = download_graph(
            &project,
            &mut refreshed_sources,
            &graph,
            &multi,
            &reqwest,
            self.threads as usize,
            self.prod,
            true,
            format!("{} 📥 downloading dependencies", job(3)),
            format!("{} 📥 downloaded dependencies", job(3)),
        )?;

        let filtered_graph = if self.prod {
            downloaded_graph
                .clone()
                .into_iter()
                .map(|(n, v)| {
                    (
                        n,
                        v.into_iter()
                            .filter(|(_, n)| n.node.ty != DependencyType::Dev)
                            .collect(),
                    )
                })
                .collect()
        } else {
            downloaded_graph.clone()
        };

        println!("{} 🗺️ linking dependencies", job(4));

        project
            .link_dependencies(&filtered_graph)
            .context("failed to link dependencies")?;

        let bin_folder = bin_dir()?;

        for versions in filtered_graph.values() {
            for node in versions.values() {
                if node.target.bin_path().is_none() {
                    continue;
                }

                let Some((alias, _)) = &node.node.direct else {
                    continue;
                };

                if alias == env!("CARGO_BIN_NAME") {
                    log::warn!("package {alias} has the same name as the CLI, skipping bin link");
                    continue;
                }

                let bin_file = bin_folder.join(alias);
                std::fs::write(&bin_file, bin_link_file(alias))
                    .context("failed to write bin link file")?;

                make_executable(&bin_file).context("failed to make bin link executable")?;

                #[cfg(windows)]
                {
                    let bin_file = bin_file.with_extension(std::env::consts::EXE_EXTENSION);
                    std::fs::copy(
                        std::env::current_exe().context("failed to get current executable path")?,
                        &bin_file,
                    )
                    .context("failed to copy bin link file")?;
                }
            }
        }

        #[cfg(feature = "patches")]
        {
            println!("{} 🩹 applying patches", job(5));

            project
                .apply_patches(&filtered_graph)
                .context("failed to apply patches")?;
        }

        println!("{} 🧹 finishing up", job(JOBS));

        project
            .write_lockfile(Lockfile {
                name: manifest.name,
                version: manifest.version,
                target: manifest.target.kind(),
                overrides: manifest.overrides,

                graph: downloaded_graph,

                workspace: run_on_workspace_members(&project, |project| {
                    self.run(project, multi.clone(), reqwest.clone())
                })?,
            })
            .context("failed to write lockfile")?;

        Ok(())
    }
}
