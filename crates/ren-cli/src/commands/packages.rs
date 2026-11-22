//! `ren packages`: the runtimes and utilities Ren installs on this host for
//! its agents, from the catalog built into this Ren. It acts on this
//! host's data directory through the same `HostPackages` as the daemon's
//! `/control/v1/packages`, beside or without a running daemon; the store's
//! lock keeps the two from changing it at once.
use crate::context::Context;
use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use ren_daemon::packages::{HostPackages, HostPrograms, PackageStore, fetch::ReqwestFetch};
use ren_depot::{
    package::{Origin, PackageId},
    package_set::{Edit, Generation},
};
use ren_gateway::packages::{ChangeKind, ChangeOutcome, PackageListing, PackageSetChange};
use ren_store::{Reader, Store};
use std::{collections::BTreeSet, process::ExitCode};

/// Install runtimes and utilities for Ren's agents on this host.
#[derive(Args)]
pub struct PackagesCmd {
    #[command(subcommand)]
    action: PackagesAction,
}

#[derive(Subcommand)]
enum PackagesAction {
    /// Every package Ren knows: what this host's PATH has, and what Ren
    /// installed.
    List(ListArgs),
    /// Download, verify, and install packages as a new generation.
    Install(PackageNames),
    /// Leave packages out of a new generation. Their files stay until `gc`.
    Remove(PackageNames),
    /// Make the next generation hold what a kept generation held, from
    /// installs still in the store.
    Rollback(RollbackArgs),
    /// Where the program a package provides is, as a new run finds it.
    Which(WhichArgs),
    /// Delete every generation but the current one and those runs still
    /// use, and every install none of them holds.
    Gc,
}

#[derive(Args)]
struct RollbackArgs {
    /// The generation to go back to, as `ren packages list --json` lists them.
    generation: Generation,
}

#[derive(Args)]
struct WhichArgs {
    /// A package name, as `ren packages list` shows it.
    package: PackageId,
}

#[derive(Args)]
struct ListArgs {
    /// Print the listing `/control/v1/packages` answers with.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct PackageNames {
    /// Package names, as `ren packages list` shows them.
    #[arg(required = true)]
    packages: Vec<PackageId>,
}

impl PackagesCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let config = ctx.config()?;
        let data_dir = ctx.data_dir(&config)?;
        // Which generations runs use. Without a database, no run was ever
        // recorded here.
        let reader = if data_dir.join(Store::METADATA_FILE).exists() {
            Some(Reader::open(&data_dir, 1).await?)
        } else {
            None
        };
        let packages = HostPackages::new(
            PackageStore::new(
                data_dir.join(PackageStore::FOLDER),
                ReqwestFetch::new().context("cannot start downloads")?,
            ),
            config.packages.depot()?,
            HostPrograms::new(ctx.env.search_path()),
            HostPrograms::platform(),
            reader,
        );
        match self.action {
            PackagesAction::List(args) => {
                let listing = packages.read_listing().await?;
                if args.json {
                    // As the control API sends it.
                    let sent = ren_gateway::api::control::body::packages::PackageListing::from(
                        listing.clone(),
                    );
                    println!("{}", serde_json::to_string_pretty(&sent)?);
                } else {
                    print!("{}", Self::table(&listing));
                }
                Ok(ExitCode::SUCCESS)
            }
            PackagesAction::Install(args) => {
                Self::change(
                    &packages,
                    Edit {
                        add: args.packages.into_iter().collect(),
                        remove: BTreeSet::new(),
                    },
                )
                .await
            }
            PackagesAction::Remove(args) => {
                Self::change(
                    &packages,
                    Edit {
                        add: BTreeSet::new(),
                        remove: args.packages.into_iter().collect(),
                    },
                )
                .await
            }
            PackagesAction::Rollback(args) => {
                Self::report(packages.go_back(args.generation).await?)
            }
            PackagesAction::Which(args) => {
                let listing = packages.read_listing().await?;
                let Some(package) = listing.packages.iter().find(|p| p.id == args.package) else {
                    bail!("no package is named {}", args.package);
                };
                let installed = listing
                    .bin
                    .as_ref()
                    .filter(|_| package.installed.is_some())
                    .map(|bin| std::path::Path::new(bin).join(package.program.as_str()));
                match (installed, &package.system) {
                    (Some(path), _) => println!("{}", path.display()),
                    (None, Some(system)) => println!("{}", system.path),
                    (None, None) => bail!("{} is neither installed by Ren nor on PATH", package.id),
                }
                Ok(ExitCode::SUCCESS)
            }
            PackagesAction::Gc => {
                let collected = packages.sweep().await?;
                for generation in &collected.generations {
                    println!("removed generation {generation}");
                }
                for install in &collected.installs {
                    println!("removed {install}");
                }
                if collected.generations.is_empty() && collected.installs.is_empty() {
                    println!("nothing to remove");
                }
                Ok(ExitCode::SUCCESS)
            }
        }
    }

    /// Apply `edit`: install what it adds and make the next generation
    /// current; nothing changes unless every install succeeds.
    async fn change(packages: &HostPackages<ReqwestFetch>, edit: Edit) -> Result<ExitCode> {
        Self::report(packages.apply(edit).await?)
    }

    /// Print how a change ended.
    fn report(changed: PackageSetChange) -> Result<ExitCode> {
        match changed.outcome {
            ChangeOutcome::Unchanged => {
                println!("nothing to change (generation {})", changed.generation);
                Ok(ExitCode::SUCCESS)
            }
            ChangeOutcome::Failed => {
                for failure in &changed.failures {
                    eprintln!(
                        "{} {}: {} failed: {}",
                        failure.package, failure.version, failure.stage, failure.reason
                    );
                }
                bail!(
                    "nothing changed; generation {} is still current",
                    changed.generation
                )
            }
            ChangeOutcome::Activated => {
                for change in &changed.changes {
                    let from = change.from.as_ref().map(ToString::to_string);
                    let to = change.to.as_ref().map(ToString::to_string);
                    let (from, to) = (from.unwrap_or_default(), to.unwrap_or_default());
                    match change.change {
                        ChangeKind::Add => println!("+ {} {to}", change.package),
                        ChangeKind::Replace => println!("~ {} {from} -> {to}", change.package),
                        ChangeKind::Remove => println!("- {} {from}", change.package),
                    }
                }
                println!("generation {} is current", changed.generation);
                Ok(ExitCode::SUCCESS)
            }
        }
    }

    fn table(listing: &PackageListing) -> String {
        let rows: Vec<[String; 5]> = listing
            .packages
            .iter()
            .map(|package| {
                [
                    package.id.to_string(),
                    match (package.origin, package.kind) {
                        (Some(Origin::Builtin), Some(kind)) => kind.to_string(),
                        (Some(Origin::Operator), Some(kind)) => format!("{kind} (configured)"),
                        (None, _) | (Some(_), None) => "not in catalog".to_owned(),
                    },
                    match (&package.release, package.origin) {
                        (Some(release), _) => release.to_string(),
                        (None, Some(_)) => "system only".to_owned(),
                        (None, None) => "-".to_owned(),
                    },
                    package
                        .installed
                        .as_ref()
                        .map_or_else(|| "-".to_owned(), |installed| installed.version.to_string()),
                    match &package.system {
                        None => "not found".to_owned(),
                        Some(system) => {
                            let version = system
                                .version
                                .as_ref()
                                .map(ToString::to_string)
                                .or_else(|| system.unread.clone())
                                .unwrap_or_default();
                            format!("{version} at {}", system.path)
                        }
                    },
                ]
            })
            .collect();
        let header = ["PACKAGE", "KIND", "RELEASE", "INSTALLED", "ON PATH"].map(String::from);
        let widths: Vec<usize> = (0..4)
            .map(|column| {
                rows.iter()
                    .chain([&header])
                    .map(|row| row[column].len())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let mut out = format!("generation {}\n", listing.generation);
        for row in [&header].into_iter().chain(&rows) {
            for (column, width) in widths.iter().enumerate() {
                out.push_str(&format!("{:<width$}  ", row[column]));
            }
            out.push_str(&row[4]);
            out.push('\n');
        }
        out
    }
}
