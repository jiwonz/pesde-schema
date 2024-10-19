use anyhow::Context;
use clap::Args;
use colored::Colorize;
use reqwest::{header::AUTHORIZATION, StatusCode};
use semver::VersionReq;
use std::{
    io::{Seek, Write},
    path::Component,
};
use tempfile::tempfile;

use crate::cli::{run_on_workspace_members, up_to_date_lockfile};
use pesde::{
    manifest::{target::Target, DependencyType},
    scripts::ScriptName,
    source::{
        pesde::{specifier::PesdeDependencySpecifier, PesdePackageSource},
        specifiers::DependencySpecifiers,
        traits::PackageSource,
        workspace::{
            specifier::{VersionType, VersionTypeOrReq},
            WorkspacePackageSource,
        },
        IGNORED_DIRS, IGNORED_FILES,
    },
    Project, DEFAULT_INDEX_NAME, MANIFEST_FILE_NAME,
};

#[derive(Debug, Args, Copy, Clone)]
pub struct PublishCommand {
    /// Whether to output a tarball instead of publishing
    #[arg(short, long)]
    dry_run: bool,

    /// Agree to all prompts
    #[arg(short, long)]
    yes: bool,
}

impl PublishCommand {
    fn run_impl(self, project: &Project, reqwest: reqwest::blocking::Client) -> anyhow::Result<()> {
        let mut manifest = project
            .deser_manifest()
            .context("failed to read manifest")?;

        println!(
            "\n{}\n",
            format!("[now publishing {} {}]", manifest.name, manifest.target)
                .bold()
                .on_bright_black()
        );

        if manifest.private {
            println!("{}", "package is private, cannot publish".red().bold());

            return Ok(());
        }

        if manifest.target.lib_path().is_none() && manifest.target.bin_path().is_none() {
            anyhow::bail!("no exports found in target");
        }

        if matches!(
            manifest.target,
            Target::Roblox { .. } | Target::RobloxServer { .. }
        ) {
            if !manifest.target.build_files().is_some_and(|f| !f.is_empty()) {
                anyhow::bail!("no build files found in target");
            }

            match up_to_date_lockfile(project)? {
                Some(lockfile) => {
                    if lockfile
                        .graph
                        .values()
                        .flatten()
                        .filter_map(|(_, node)| node.node.direct.as_ref().map(|_| node))
                        .any(|node| {
                            node.target.build_files().is_none()
                                && !matches!(node.node.ty, DependencyType::Dev)
                        })
                    {
                        anyhow::bail!("roblox packages may not depend on non-roblox packages");
                    }
                }
                None => {
                    anyhow::bail!("outdated lockfile, please run the install command first")
                }
            }
        }

        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            vec![],
            flate2::Compression::best(),
        ));

        let mut display_includes: Vec<String> = vec![MANIFEST_FILE_NAME.to_string()];
        let mut display_build_files: Vec<String> = vec![];

        let (lib_path, bin_path, target_kind) = (
            manifest.target.lib_path().cloned(),
            manifest.target.bin_path().cloned(),
            manifest.target.kind(),
        );

        let mut roblox_target = match &mut manifest.target {
            Target::Roblox { build_files, .. } => Some(build_files),
            Target::RobloxServer { build_files, .. } => Some(build_files),
            _ => None,
        };

        if manifest.includes.insert(MANIFEST_FILE_NAME.to_string()) {
            println!(
                "{}: {MANIFEST_FILE_NAME} was not in includes, adding it",
                "warn".yellow().bold()
            );
        }

        if manifest.includes.remove(".git") {
            println!(
                "{}: .git was in includes, removing it",
                "warn".yellow().bold()
            );
        }

        if !manifest.includes.iter().any(|f| {
            matches!(
                f.to_lowercase().as_str(),
                "readme" | "readme.md" | "readme.txt"
            )
        }) {
            println!(
                "{}: no README file in includes, consider adding one",
                "warn".yellow().bold()
            );
        }

        if !manifest.includes.iter().any(|f| f == "docs") {
            println!(
                "{}: no docs directory in includes, consider adding one",
                "warn".yellow().bold()
            );
        }

        if manifest.includes.remove("default.project.json") {
            println!(
                "{}: default.project.json was in includes, this should be generated by the {} script upon dependants installation",
                "warn".yellow().bold(),
                ScriptName::RobloxSyncConfigGenerator
            );
        }

        for ignored_path in IGNORED_FILES.iter().chain(IGNORED_DIRS.iter()) {
            if manifest.includes.remove(*ignored_path) {
                println!(
                    r#"{}: {ignored_path} was in includes, removing it.
{}: if this was a toolchain manager's manifest file, do not include it due to it possibly messing with user scripts
{}: otherwise, the file was deemed unnecessary, if you don't understand why, please contact the maintainers"#,
                    "warn".yellow().bold(),
                    "info".blue().bold(),
                    "info".blue().bold()
                );
            }
        }

        for (name, path) in [("lib path", lib_path), ("bin path", bin_path)] {
            let Some(export_path) = path else { continue };

            let export_path = export_path.to_path(project.package_dir());
            if !export_path.exists() {
                anyhow::bail!("{name} points to non-existent file");
            }

            if !export_path.is_file() {
                anyhow::bail!("{name} must point to a file");
            }

            let contents =
                std::fs::read_to_string(&export_path).context(format!("failed to read {name}"))?;

            if let Err(err) = full_moon::parse(&contents).map_err(|errs| {
                errs.into_iter()
                    .map(|err| err.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            }) {
                anyhow::bail!("{name} is not a valid Luau file: {err}");
            }

            let first_part = export_path
                .strip_prefix(project.package_dir())
                .context(format!("{name} not within project directory"))?
                .components()
                .next()
                .context(format!("{name} must contain at least one part"))?;

            let first_part = match first_part {
                Component::Normal(part) => part,
                _ => anyhow::bail!("{name} must be within project directory"),
            };

            let first_part_str = first_part.to_string_lossy();

            if manifest.includes.insert(first_part_str.to_string()) {
                println!(
                    "{}: {name} was not in includes, adding {first_part_str}",
                    "warn".yellow().bold()
                );
            }

            if roblox_target.as_mut().map_or(false, |build_files| {
                build_files.insert(first_part_str.to_string())
            }) {
                println!(
                    "{}: {name} was not in build files, adding {first_part_str}",
                    "warn".yellow().bold()
                );
            }
        }

        for included_name in &manifest.includes {
            let included_path = project.package_dir().join(included_name);

            if !included_path.exists() {
                anyhow::bail!("included file {included_name} does not exist");
            }

            // it's already included, and guaranteed to be a file
            if included_name.eq_ignore_ascii_case(MANIFEST_FILE_NAME) {
                continue;
            }

            if included_path.is_file() {
                display_includes.push(included_name.clone());

                archive.append_file(
                    included_name,
                    &mut std::fs::File::open(&included_path)
                        .context(format!("failed to read {included_name}"))?,
                )?;
            } else {
                display_includes.push(format!("{included_name}/*"));

                archive
                    .append_dir_all(included_name, &included_path)
                    .context(format!("failed to include directory {included_name}"))?;
            }
        }

        if let Some(build_files) = &roblox_target {
            for build_file in build_files.iter() {
                if build_file.eq_ignore_ascii_case(MANIFEST_FILE_NAME) {
                    println!(
                        "{}: {MANIFEST_FILE_NAME} is in build files, please remove it",
                        "warn".yellow().bold()
                    );

                    continue;
                }

                let build_file_path = project.package_dir().join(build_file);

                if !build_file_path.exists() {
                    anyhow::bail!("build file {build_file} does not exist");
                }

                if !manifest.includes.contains(build_file) {
                    anyhow::bail!("build file {build_file} is not in includes, please add it");
                }

                if build_file_path.is_file() {
                    display_build_files.push(build_file.clone());
                } else {
                    display_build_files.push(format!("{build_file}/*"));
                }
            }
        }

        #[cfg(feature = "wally-compat")]
        let mut has_wally = false;
        let mut has_git = false;

        for specifier in manifest
            .dependencies
            .values_mut()
            .chain(manifest.dev_dependencies.values_mut())
            .chain(manifest.peer_dependencies.values_mut())
        {
            match specifier {
                DependencySpecifiers::Pesde(specifier) => {
                    let index_name = specifier
                        .index
                        .as_deref()
                        .unwrap_or(DEFAULT_INDEX_NAME)
                        .to_string();
                    specifier.index = Some(
                        manifest
                            .indices
                            .get(&index_name)
                            .context(format!("index {index_name} not found in indices field"))?
                            .to_string(),
                    );
                }
                #[cfg(feature = "wally-compat")]
                DependencySpecifiers::Wally(specifier) => {
                    has_wally = true;

                    let index_name = specifier
                        .index
                        .as_deref()
                        .unwrap_or(DEFAULT_INDEX_NAME)
                        .to_string();
                    specifier.index = Some(
                        manifest
                            .wally_indices
                            .get(&index_name)
                            .context(format!(
                                "index {index_name} not found in wally_indices field"
                            ))?
                            .to_string(),
                    );
                }
                DependencySpecifiers::Git(_) => {
                    has_git = true;
                }
                DependencySpecifiers::Workspace(spec) => {
                    let pkg_ref = WorkspacePackageSource
                        .resolve(spec, project, target_kind)
                        .context("failed to resolve workspace package")?
                        .1
                        .pop_last()
                        .context("no versions found for workspace package")?
                        .1;

                    let manifest = pkg_ref
                        .path
                        .to_path(
                            project
                                .workspace_dir()
                                .context("failed to get workspace directory")?,
                        )
                        .join(MANIFEST_FILE_NAME);
                    let manifest = std::fs::read_to_string(&manifest)
                        .context("failed to read workspace package manifest")?;
                    let manifest = toml::from_str::<pesde::manifest::Manifest>(&manifest)
                        .context("failed to parse workspace package manifest")?;

                    *specifier = DependencySpecifiers::Pesde(PesdeDependencySpecifier {
                        name: spec.name.clone(),
                        version: match spec.version.clone() {
                            VersionTypeOrReq::VersionType(VersionType::Wildcard) => {
                                VersionReq::STAR
                            }
                            VersionTypeOrReq::Req(r) => r,
                            v => VersionReq::parse(&format!("{v}{}", manifest.version))
                                .context(format!("failed to parse version for {v}"))?,
                        },
                        index: Some(
                            manifest
                                .indices
                                .get(DEFAULT_INDEX_NAME)
                                .context("missing default index in workspace package manifest")?
                                .to_string(),
                        ),
                        target: Some(spec.target.unwrap_or(manifest.target.kind())),
                    });
                }
            }
        }

        {
            println!("\n{}", "please confirm the following information:".bold());
            println!("name: {}", manifest.name);
            println!("version: {}", manifest.version);
            println!(
                "description: {}",
                manifest.description.as_deref().unwrap_or("(none)")
            );
            println!(
                "license: {}",
                manifest.license.as_deref().unwrap_or("(none)")
            );
            println!(
                "authors: {}",
                if manifest.authors.is_empty() {
                    "(none)".to_string()
                } else {
                    manifest.authors.join(", ")
                }
            );
            println!(
                "repository: {}",
                manifest
                    .repository
                    .as_ref()
                    .map(|r| r.as_str())
                    .unwrap_or("(none)")
            );

            let roblox_target = roblox_target.is_some_and(|_| true);

            println!("target: {}", manifest.target);
            println!(
                "\tlib path: {}",
                manifest
                    .target
                    .lib_path()
                    .map_or("(none)".to_string(), |p| p.to_string())
            );

            if roblox_target {
                println!("\tbuild files: {}", display_build_files.join(", "));
            } else {
                println!(
                    "\tbin path: {}",
                    manifest
                        .target
                        .bin_path()
                        .map_or("(none)".to_string(), |p| p.to_string())
                );
            }

            println!(
                "includes: {}",
                display_includes.into_iter().collect::<Vec<_>>().join(", ")
            );

            if !self.dry_run
                && !self.yes
                && !inquire::Confirm::new("is this information correct?").prompt()?
            {
                println!("\n{}", "publish aborted".red().bold());

                return Ok(());
            }

            println!();
        }

        let mut temp_manifest = tempfile().context("failed to create temp manifest file")?;
        temp_manifest
            .write_all(
                toml::to_string(&manifest)
                    .context("failed to serialize manifest")?
                    .as_bytes(),
            )
            .context("failed to write temp manifest file")?;
        temp_manifest
            .rewind()
            .context("failed to rewind temp manifest file")?;

        archive.append_file(MANIFEST_FILE_NAME, &mut temp_manifest)?;

        let archive = archive
            .into_inner()
            .context("failed to encode archive")?
            .finish()
            .context("failed to get archive bytes")?;

        let index_url = manifest
            .indices
            .get(DEFAULT_INDEX_NAME)
            .context("missing default index")?;
        let source = PesdePackageSource::new(index_url.clone());
        source
            .refresh(project)
            .context("failed to refresh source")?;
        let config = source
            .config(project)
            .context("failed to get source config")?;

        if archive.len() > config.max_archive_size {
            anyhow::bail!(
                "archive size exceeds maximum size of {} bytes by {} bytes",
                config.max_archive_size,
                archive.len() - config.max_archive_size
            );
        }

        manifest.all_dependencies().context("dependency conflict")?;

        if !config.git_allowed && has_git {
            anyhow::bail!("git dependencies are not allowed on this index");
        }

        #[cfg(feature = "wally-compat")]
        if !config.wally_allowed && has_wally {
            anyhow::bail!("wally dependencies are not allowed on this index");
        }

        if self.dry_run {
            std::fs::write("package.tar.gz", archive)?;

            println!(
                "{}",
                "(dry run) package written to package.tar.gz".green().bold()
            );

            return Ok(());
        }

        let mut request = reqwest
            .post(format!("{}/v0/packages", config.api()))
            .multipart(reqwest::blocking::multipart::Form::new().part(
                "tarball",
                reqwest::blocking::multipart::Part::bytes(archive).file_name("package.tar.gz"),
            ));

        if let Some(token) = project.auth_config().tokens().get(index_url) {
            log::debug!("using token for {index_url}");
            request = request.header(AUTHORIZATION, token);
        }

        let response = request.send().context("failed to send request")?;

        let status = response.status();
        let text = response.text().context("failed to get response text")?;
        match status {
            StatusCode::CONFLICT => {
                println!("{}", "package version already exists".red().bold());
            }
            StatusCode::FORBIDDEN => {
                println!(
                    "{}",
                    "unauthorized to publish under this scope".red().bold()
                );
            }
            StatusCode::BAD_REQUEST => {
                println!("{}: {text}", "invalid package".red().bold());
            }
            code if !code.is_success() => {
                anyhow::bail!("failed to publish package: {code} ({text})");
            }
            _ => {
                println!("{text}");
            }
        }

        Ok(())
    }

    pub fn run(self, project: Project, reqwest: reqwest::blocking::Client) -> anyhow::Result<()> {
        let result = self.run_impl(&project, reqwest.clone());
        if project.workspace_dir().is_some() {
            return result;
        } else if let Err(result) = result {
            println!("an error occurred publishing workspace root: {result}");
        }

        run_on_workspace_members(&project, |project| self.run_impl(&project, reqwest.clone()))
            .map(|_| ())
    }
}
