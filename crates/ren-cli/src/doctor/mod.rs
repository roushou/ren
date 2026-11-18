//! Bounded, credential-free diagnostics. Never sign in or issue inference requests.
//!
//! This host's checks are here; what a gateway reports about itself, read
//! through its API, is in `gateway`.

mod gateway;

use crate::client::GatewayChoice;
use crate::style::{Styling, Tone};
use gateway::GatewayProbe;
pub use gateway::ProbeError;
use ren_daemon::config::{Config, ToolsOptions};
use ren_daemon::tools::HostTools;
use ren_depot::{
    Surveyed, ToolHost,
    package::{Kind, PackageId, Requirement},
    provision::{Fit, Provision},
    version::ObservedVersion,
};
use ren_harness::{Harness, host::HostEnv};
use ren_protocol::label::Labeled;
use ren_store::{Reader, query::CallQuery};
use serde::Serialize;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::path::{Path, PathBuf};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How a check came out.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, SerializeDisplay, DeserializeFromStr)]
pub enum Status {
    Ok,
    Warning,
    Failed,
    Skipped,
}

/// What a check looks at. Its label is the name `ren doctor` prints and
/// reports in JSON, which other tools read (`cargo xtask install-vps` reads
/// `build`).
#[derive(
    Display,
    EnumString,
    IntoStaticStr,
    VariantArray,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Debug,
    SerializeDisplay,
    DeserializeFromStr,
)]
pub enum CheckId {
    #[strum(to_string = "configuration")]
    Configuration,
    #[strum(to_string = "gateway choice")]
    GatewayChoice,
    #[strum(to_string = "local storage")]
    LocalStorage,
    #[strum(to_string = "upstream config")]
    UpstreamConfig,
    #[strum(to_string = "auth environment")]
    AuthEnvironment,
    #[strum(to_string = "gateway URL")]
    GatewayUrl,
    #[strum(to_string = "subscription")]
    Subscription,
    #[strum(to_string = "storage permissions")]
    StoragePermissions,
    #[strum(to_string = "tool catalog")]
    ToolCatalog,
    #[strum(to_string = "build")]
    Build,
    #[strum(to_string = "gateway")]
    Gateway,
    #[strum(to_string = "capture")]
    Capture,
    #[strum(to_string = "storage API")]
    StorageApi,
    #[strum(to_string = "instance")]
    Instance,
    #[strum(to_string = "caller")]
    Caller,
    #[strum(to_string = "digests")]
    Digests,
}

/// A check's name: one of Ren's own checks, or a package the host provides.
#[derive(Clone, PartialEq, Eq, Debug, SerializeDisplay)]
pub enum CheckName {
    Check(CheckId),
    Package(PackageId),
}

impl std::fmt::Display for CheckName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `pad`, so a report's column width applies.
        match self {
            Self::Check(id) => f.pad(id.label()),
            Self::Package(package) => f.pad(package.as_str()),
        }
    }
}

impl Status {
    /// The text-report badge and its tone.
    fn badge(self) -> (&'static str, Tone) {
        match self {
            Self::Ok => ("OK", Tone::Good),
            Self::Warning => ("WARN", Tone::Warn),
            Self::Failed => ("FAIL", Tone::Bad),
            Self::Skipped => ("SKIP", Tone::Muted),
        }
    }

    /// The tone a check with this status is shown in.
    pub(crate) fn tone(self) -> Tone {
        self.badge().1
    }

    /// How the summary counts `n` checks with this status.
    fn tally(self, n: usize) -> String {
        match self {
            Self::Ok => format!("{n} ok"),
            Self::Warning if n == 1 => "1 warning".to_owned(),
            Self::Warning => format!("{n} warnings"),
            Self::Failed => format!("{n} failed"),
            Self::Skipped => format!("{n} skipped"),
        }
    }
}

#[derive(Serialize)]
pub struct Check {
    pub name: CheckName,
    pub status: Status,
    pub message: String,
}

#[derive(Default, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn success(&self) -> bool {
        !self.checks.iter().any(|c| c.status == Status::Failed)
    }
    /// A report with one failed check, for a diagnosis that cannot start.
    pub(crate) fn failed(name: CheckId, message: impl Into<String>) -> Self {
        let mut report = Self::default();
        report.add(name, Status::Failed, message);
        report
    }

    fn add(&mut self, name: CheckId, status: Status, message: impl Into<String>) {
        self.checks.push(Check {
            name: CheckName::Check(name),
            status,
            message: message.into(),
        });
    }
    /// Prints the report: JSON, or one aligned line per check and a summary.
    pub fn print(&self, json: bool, styling: Styling) -> Result<(), serde_json::Error> {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"ok": self.success(), "checks": self.checks})
                )?
            );
            return Ok(());
        }
        for check in &self.checks {
            let (label, tone) = check.status.badge();
            // Pad before painting: escape codes would count as width.
            let label = styling.paint(tone, &format!("{label:4}"));
            let name = format!("{:18}", check.name);
            match check.status {
                Status::Skipped => println!(
                    "{label} {} {}",
                    styling.paint(Tone::Muted, &name),
                    styling.paint(Tone::Muted, &check.message)
                ),
                Status::Ok | Status::Warning | Status::Failed => {
                    println!(
                        "{label} {} {}",
                        styling.paint(Tone::Strong, &name),
                        check.message
                    );
                }
            }
        }
        println!("\n{}", self.summary(styling));
        Ok(())
    }

    /// "8 ok, 1 warning, 0 failed, 3 skipped", each nonzero count in its tone.
    fn summary(&self, styling: Styling) -> String {
        [Status::Ok, Status::Warning, Status::Failed, Status::Skipped]
            .into_iter()
            .map(|status| {
                let n = self.checks.iter().filter(|c| c.status == status).count();
                let tone = if n == 0 { Tone::Muted } else { status.tone() };
                styling.paint(tone, &status.tally(n))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Report {
    /// Diagnose this host: configuration, storage, client executables, and the
    /// gateway: the selected one, or the local listener when none is.
    pub async fn local(
        env: &HostEnv,
        config_path: Option<&Path>,
        data_dir: Option<PathBuf>,
        gateway: &GatewayChoice,
    ) -> Self {
        let mut report = Self::default();
        let config = match Config::load(config_path) {
            Ok(config) => {
                report.add(
                    CheckId::Configuration,
                    Status::Ok,
                    "local configuration is valid",
                );
                config
            }
            Err(error) => {
                report.add(CheckId::Configuration, Status::Failed, error.to_string());
                return report;
            }
        };
        report.add(CheckId::GatewayChoice, Status::Ok, gateway.describe());
        let remote = match gateway {
            GatewayChoice::Selected { .. } => true,
            GatewayChoice::LocalListener => false,
        };
        if remote {
            report.add(
                CheckId::LocalStorage,
                Status::Skipped,
                "gateway selected; storage is checked through its API",
            );
            report.add(
                CheckId::UpstreamConfig,
                Status::Skipped,
                "local upstream settings do not describe the selected gateway",
            );
        } else {
            let valid_upstreams = config.check().is_ok();
            report.add(
                CheckId::UpstreamConfig,
                if valid_upstreams {
                    Status::Ok
                } else {
                    Status::Failed
                },
                if valid_upstreams {
                    "upstream URLs and transport configuration are valid; connectivity not tested"
                } else {
                    "invalid upstream URL or transport configuration"
                },
            );
            match config.resolve_data_dir(data_dir, env) {
                Ok(dir) => report.local_storage(&dir).await,
                Err(error) => report.add(CheckId::LocalStorage, Status::Failed, error.to_string()),
            }
        }
        report.tools(&config.tools, &HostTools::new(env.search_path()));
        let overrides = Self::credential_overrides(env);
        if !overrides.is_empty() {
            report.add(
                CheckId::AuthEnvironment,
                Status::Warning,
                format!(
                    "{} set; these can change client authentication. Values are not printed",
                    overrides.join(", ")
                ),
            );
        }
        match gateway.endpoint(config.listen) {
            Ok(gateway) => report
                .checks
                .extend(Self::gateway(&gateway, env).await.checks),
            Err(error) => report.add(CheckId::GatewayUrl, Status::Failed, error.to_string()),
        }
        report.add(CheckId::Subscription, Status::Skipped, "sign-in, provider connectivity, and entitlement require a real client request; no inference was sent");
        report
    }

    async fn local_storage(&mut self, dir: &Path) {
        let report = self;
        let db = dir.join("ren.sqlite3");
        match std::fs::metadata(&db) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                report.add(
                    CheckId::LocalStorage,
                    Status::Warning,
                    "database not created yet; start ren serve with this data directory",
                );
                return;
            }
            Err(_) => {
                report.add(
                    CheckId::LocalStorage,
                    Status::Failed,
                    "cannot inspect the local database",
                );
                return;
            }
            Ok(metadata) if !metadata.is_file() => {
                report.add(
                    CheckId::LocalStorage,
                    Status::Failed,
                    "database path is not a regular file",
                );
                return;
            }
            Ok(_) => {}
        }
        let readable = tokio::time::timeout(GatewayProbe::DEADLINE, async {
            Reader::open(dir, 1)
                .await?
                .calls(&CallQuery::default())
                .await
        })
        .await;
        report.add(
        CheckId::LocalStorage,
        if matches!(readable, Ok(Ok(_))) {
            Status::Ok
        } else {
            Status::Failed
        },
        if matches!(readable, Ok(Ok(_))) {
            "read-only history query passed; no migration or write probe performed"
        } else {
            "history query failed or timed out; check permissions, schema, and database integrity"
        },
    );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let private = [dir.to_path_buf(), db].iter().all(|path| {
                std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 == 0)
            });
            report.add(
                CheckId::StoragePermissions,
                if private { Status::Ok } else { Status::Warning },
                if private {
                    "directory and database have owner-only mode bits"
                } else {
                    "directory or database is accessible to group/others; review capture privacy"
                },
            );
        }
    }

    /// Environment variables that change client authentication, by name.
    /// Presence only; auth files, credential helpers, and values are never read.
    pub fn credential_overrides(env: &HostEnv) -> Vec<&'static str> {
        [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
        ]
        .into_iter()
        .filter(|key| env.is_set(key))
        .collect()
    }

    /// One check per package the depot knows: where this host's search path
    /// has it and which version it reported. A missing client is a warning;
    /// a missing runtime or utility is one only when a client found here
    /// needs it.
    fn tools(&mut self, tools: &ToolsOptions, host: &impl ToolHost) {
        let depot = match tools.depot() {
            Ok(depot) => depot,
            Err(error) => {
                self.add(CheckId::ToolCatalog, Status::Failed, error.to_string());
                return;
            }
        };
        let survey = depot.survey(host);
        let present = |id: &PackageId| {
            survey.iter().any(|surveyed| {
                surveyed.package.id() == id && !matches!(surveyed.provision, Provision::Missing)
            })
        };
        let requirements: Vec<(Harness, Requirement)> = Harness::ALL
            .iter()
            .flat_map(|harness| {
                harness
                    .requirements()
                    .into_iter()
                    .map(|requirement| (*harness, requirement))
            })
            .collect();
        for Surveyed { package, provision } in &survey {
            // Requirements of this package from clients installed here.
            let needed_by: Vec<&(Harness, Requirement)> = requirements
                .iter()
                .filter(|(harness, requirement)| {
                    requirement.package() == package.id()
                        && harness
                            .requirements()
                            .iter()
                            .filter(|other| other.package() != package.id())
                            .all(|other| present(other.package()))
                })
                .collect();
            let (status, message) = match provision {
                Provision::Missing => match package.kind() {
                    Kind::Client => (
                        Status::Warning,
                        "not found on PATH; install it if you use this client".to_owned(),
                    ),
                    Kind::Runtime | Kind::Utility if needed_by.is_empty() => (
                        Status::Skipped,
                        "not found on PATH; no client found here needs it".to_owned(),
                    ),
                    Kind::Runtime | Kind::Utility => (
                        Status::Warning,
                        format!(
                            "not found on PATH; {} needs it",
                            Self::names(needed_by.iter().map(|(harness, _)| *harness))
                        ),
                    ),
                },
                Provision::System(found) => {
                    let outside: Vec<&(Harness, Requirement)> = needed_by
                        .iter()
                        .copied()
                        .filter(|(_, requirement)| {
                            matches!(
                                provision.fit(requirement.wants()),
                                Fit::Outside | Fit::Unverified
                            )
                        })
                        .collect();
                    let at = found.path.display();
                    let version = match &found.version {
                        ObservedVersion::Read(version) => format!("{version} at {at}"),
                        ObservedVersion::Unparsed(raw) => {
                            format!("found at {at}; version not read from \"{raw}\"")
                        }
                        ObservedVersion::Unread(failure) => {
                            format!("found at {at}; version not read: {failure}")
                        }
                    };
                    if outside.is_empty() {
                        (Status::Ok, version)
                    } else {
                        (
                            Status::Warning,
                            format!(
                                "{version}; not a version {} accepts",
                                Self::names(outside.iter().map(|(harness, _)| *harness))
                            ),
                        )
                    }
                }
            };
            self.checks.push(Check {
                name: CheckName::Package(package.id().clone()),
                status,
                message,
            });
        }
    }

    fn names(harnesses: impl Iterator<Item = Harness>) -> String {
        harnesses
            .map(|harness| harness.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckId, CheckName};

    #[test]
    fn a_check_name_fills_the_report_column_and_reports_its_label() {
        let name = CheckName::Check(CheckId::GatewayUrl);
        assert_eq!(format!("{name:18}|"), "gateway URL       |");
        assert_eq!(serde_json::to_string(&name).unwrap(), r#""gateway URL""#);
        let package = CheckName::Package("node".parse().unwrap());
        assert_eq!(format!("{package:6}|"), "node  |");
    }
}
