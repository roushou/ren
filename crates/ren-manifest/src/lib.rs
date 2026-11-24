//! A project's manifest for Ren (docs/plans/self-build.md, phase 6), pure:
//! how a repository is built, served, checked, and quiesced, declared in
//! [`Manifest::FILE`] (`.ren/project.toml`), so a worktree of it gets checks
//! and a preview as Ren's own do.
//!
//! ```toml
//! packages = ["node"]
//! reach = ["registry.npmjs.org:443", "*.example.com:443"]
//!
//! [[build]]
//! run = ["npm", "ci"]
//!
//! [[build]]
//! run = ["npm", "run", "build"]
//!
//! [serve]
//! run = ["node", "build/index.js"]
//! env = { PORT = "{port}", HOST = "127.0.0.1" }
//! open = "http://127.0.0.1:{port}"
//! health = "/health"
//!
//! [[check]]
//! name = "tests"
//! run = ["npm", "test"]
//!
//! [quiesce]
//! run = ["curl", "-fsS", "-X", "POST", "http://127.0.0.1:{port}/drain"]
//!
//! [deploy]
//! run = ["./deploy.sh", "production"]
//! ```
//!
//! `reach` lists where the project's relayed runs may connect through
//! Ren's relay, besides what Ren's configuration and each client list:
//! `host:port`, or `*.parent:port` for any name under one. Ren reads it
//! from the default branch on GitHub, never from a run's checkout, which
//! the run could change.
//!
//! Deploying runs in a build the canary checked, when the operator's
//! promotion policy says (`ren project canary`, `ren project promote`).
//!
//! `packages` names the packages of Ren's depot its steps need (`ren
//! packages list`); every step runs with Ren's package set first on its
//! search path. The key was `tools` before; a manifest that still says
//! `tools` is refused with its new name.
//!
//! Every command is an argv, never a shell line. Arguments and values may
//! name the place a step runs for ([`Place`]): `{port}` (its first port),
//! `{port+N}` (another of its ports), `{data}` (its data folder), and
//! `{worktree}` (its folder; `{workspace}`, its old name, is read the
//! same). The manifest is parsed once
//! ([`Manifest::read`]); an unknown placeholder is refused there, and
//! a port past the place's range when a step is expanded.

mod template;

pub use template::{Template, TemplateError};

use ren_depot::package::PackageId;
use ren_destination::Destination;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

/// How a project is built, served, checked, and quiesced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    packages: BTreeSet<PackageId>,
    reach: Vec<Destination>,
    build: Vec<Step>,
    serve: Option<Serve>,
    checks: Vec<NamedCheck>,
    quiesce: Option<Step>,
    deploy: Option<Step>,
}

/// One command: its argv and the environment it adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    run: Vec<Template>,
    env: BTreeMap<String, Template>,
}

/// How the project runs: its command, where its page opens, and the path
/// that answers once it is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Serve {
    step: Step,
    open: Template,
    health: String,
}

/// One named check: it passes when its command exits successfully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedCheck {
    name: String,
    step: Step,
}

/// Where a step runs: a worktree's folder, data folder, and ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub worktree: PathBuf,
    pub data: PathBuf,
    /// Its ports, first and last; none outside a worktree, where a step
    /// that names a port does not expand.
    pub ports: Option<(u16, u16)>,
}

/// A step made concrete for a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expanded {
    /// The program, then its arguments.
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Why a manifest does not read.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error(".ren/project.toml does not read: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("a command in .ren/project.toml is empty")]
    EmptyCommand,
    #[error("check {0:?} is named twice in .ren/project.toml")]
    DuplicateCheck(String),
    #[error("a check in .ren/project.toml has no name")]
    UnnamedCheck,
    #[error("the serve health path must start with `/`: {0:?}")]
    Health(String),
    #[error("an environment variable in .ren/project.toml has an invalid name: {0:?}")]
    EnvName(String),
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error("reach in .ren/project.toml lists {0:?}, which is not `host:port` or `*.parent:port`")]
    Reach(String),
}

/// The `tools` key, renamed to `packages`: a manifest that still has it
/// is refused with the new name rather than read without it.
#[derive(Default)]
struct RenamedTools;

impl<'de> Deserialize<'de> for RenamedTools {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "`tools` was renamed to `packages`",
        ))
    }
}

/// The file's own shape, read once and parsed into the manifest.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    #[serde(default)]
    packages: BTreeSet<PackageId>,
    /// `packages`' old name, refused by its new one.
    #[serde(default)]
    #[expect(dead_code, reason = "present only to refuse the old key")]
    tools: RenamedTools,
    #[serde(default)]
    build: Vec<StepFile>,
    serve: Option<ServeFile>,
    #[serde(default)]
    check: Vec<CheckFile>,
    quiesce: Option<StepFile>,
    deploy: Option<StepFile>,
    #[serde(default)]
    reach: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepFile {
    run: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServeFile {
    run: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    open: String,
    #[serde(default = "ServeFile::root")]
    health: String,
}

impl ServeFile {
    fn root() -> String {
        "/".to_owned()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckFile {
    name: String,
    run: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

impl Manifest {
    /// The manifest's file, relative to the repository's root: in `.ren`,
    /// the folder that holds what Ren reads from a repository.
    pub const FILE: &str = ".ren/project.toml";

    /// The manifest `text` declares.
    pub fn read(text: &str) -> Result<Self, ManifestError> {
        let file: ManifestFile = toml::from_str(text)?;
        let build = file
            .build
            .into_iter()
            .map(|step| Step::read(step.run, step.env))
            .collect::<Result<_, _>>()?;
        let serve = file
            .serve
            .map(|serve| -> Result<Serve, ManifestError> {
                if !serve.health.starts_with('/') {
                    return Err(ManifestError::Health(serve.health));
                }
                Ok(Serve {
                    step: Step::read(serve.run, serve.env)?,
                    open: serve.open.parse::<Template>()?,
                    health: serve.health,
                })
            })
            .transpose()?;
        let mut checks: Vec<NamedCheck> = Vec::new();
        for check in file.check {
            let name = check.name.trim().to_owned();
            if name.is_empty() {
                return Err(ManifestError::UnnamedCheck);
            }
            if checks.iter().any(|known| known.name == name) {
                return Err(ManifestError::DuplicateCheck(name));
            }
            checks.push(NamedCheck {
                name,
                step: Step::read(check.run, check.env)?,
            });
        }
        let quiesce = file
            .quiesce
            .map(|step| Step::read(step.run, step.env))
            .transpose()?;
        let deploy = file
            .deploy
            .map(|step| Step::read(step.run, step.env))
            .transpose()?;
        let reach = file
            .reach
            .into_iter()
            .map(|text| {
                text.parse::<Destination>()
                    .map_err(|_| ManifestError::Reach(text))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            packages: file.packages,
            build,
            serve,
            checks,
            quiesce,
            deploy,
            reach,
        })
    }

    /// Where its relayed runs may connect through the relay, besides what
    /// Ren's configuration and their clients list.
    pub fn reach(&self) -> &[Destination] {
        &self.reach
    }

    /// The depot's packages its steps need.
    pub fn packages(&self) -> &BTreeSet<PackageId> {
        &self.packages
    }

    /// Its build steps, in order.
    pub fn build(&self) -> &[Step] {
        &self.build
    }

    pub fn serve(&self) -> Option<&Serve> {
        self.serve.as_ref()
    }

    pub fn checks(&self) -> &[NamedCheck] {
        &self.checks
    }

    /// What drains it before it is stopped, if anything.
    pub fn quiesce(&self) -> Option<&Step> {
        self.quiesce.as_ref()
    }

    /// What puts a checked build into production, if the project declares
    /// how; without it, it is never promoted.
    pub fn deploy(&self) -> Option<&Step> {
        self.deploy.as_ref()
    }
}

impl Step {
    fn read(run: Vec<String>, env: BTreeMap<String, String>) -> Result<Self, ManifestError> {
        if run.first().is_none_or(|program| program.trim().is_empty()) {
            return Err(ManifestError::EmptyCommand);
        }
        let env = env
            .into_iter()
            .map(|(name, value)| {
                let named = !name.is_empty()
                    && !name.starts_with(|first: char| first.is_ascii_digit())
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                if named {
                    Ok((name, value.parse()?))
                } else {
                    Err(ManifestError::EnvName(name))
                }
            })
            .collect::<Result<_, ManifestError>>()?;
        Ok(Self {
            run: run
                .iter()
                .map(|arg| arg.parse())
                .collect::<Result<_, _>>()?,
            env,
        })
    }

    /// The step, for `place`.
    pub fn expand(&self, place: &Place) -> Result<Expanded, TemplateError> {
        Ok(Expanded {
            argv: self
                .run
                .iter()
                .map(|arg| arg.expand(place))
                .collect::<Result<_, _>>()?,
            env: self
                .env
                .iter()
                .map(|(name, value)| Ok((name.clone(), value.expand(place)?)))
                .collect::<Result<_, TemplateError>>()?,
        })
    }
}

impl Serve {
    pub fn step(&self) -> &Step {
        &self.step
    }

    /// Where its pages open, for `place`.
    pub fn open(&self, place: &Place) -> Result<String, TemplateError> {
        self.open.expand(place)
    }

    /// The path that answers once it is up.
    pub fn health(&self) -> &str {
        &self.health
    }
}

impl NamedCheck {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn step(&self) -> &Step {
        &self.step
    }
}

#[cfg(test)]
mod tests {
    use super::{Manifest, ManifestError, Place};
    use std::path::PathBuf;

    const MANIFEST: &str = r#"
packages = ["node", "jq", "node"]

[[build]]
run = ["npm", "ci"]

[[build]]
run = ["npm", "run", "build"]

[serve]
run = ["node", "build/index.js", "--data", "{data}/acme"]
env = { PORT = "{port}", ADMIN_PORT = "{port+1}", HOST = "127.0.0.1" }
open = "http://127.0.0.1:{port}"
health = "/health"

[[check]]
name = "tests"
run = ["npm", "test"]

[quiesce]
run = ["curl", "-fsS", "-X", "POST", "http://127.0.0.1:{port}/drain"]

[deploy]
run = ["./deploy.sh", "{worktree}"]
"#;

    fn place() -> Place {
        Place {
            worktree: PathBuf::from("/data/worktrees/acme-web-2"),
            data: PathBuf::from("/data/worktrees/.data/acme-web-2"),
            ports: Some((20_100, 20_199)),
        }
    }

    #[test]
    fn a_manifest_reads_and_expands_for_its_place() {
        let manifest = Manifest::read(MANIFEST).unwrap();
        assert_eq!(manifest.build().len(), 2);
        assert_eq!(
            manifest
                .packages()
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>(),
            ["jq", "node"]
        );
        let serve = manifest.serve().unwrap();
        let expanded = serve.step().expand(&place()).unwrap();
        assert_eq!(
            expanded.argv,
            [
                "node",
                "build/index.js",
                "--data",
                "/data/worktrees/.data/acme-web-2/acme"
            ]
        );
        assert_eq!(
            expanded.env,
            [
                ("ADMIN_PORT".to_owned(), "20101".to_owned()),
                ("HOST".to_owned(), "127.0.0.1".to_owned()),
                ("PORT".to_owned(), "20100".to_owned()),
            ]
        );
        assert_eq!(serve.open(&place()).unwrap(), "http://127.0.0.1:20100");
        assert_eq!(serve.health(), "/health");
        assert_eq!(manifest.checks()[0].name(), "tests");
        assert_eq!(
            manifest.quiesce().unwrap().expand(&place()).unwrap().argv[4],
            "http://127.0.0.1:20100/drain"
        );
        assert_eq!(
            manifest.deploy().unwrap().expand(&place()).unwrap().argv,
            ["./deploy.sh", "/data/worktrees/acme-web-2"]
        );
        // A manifest may declare as little as nothing.
        let empty = Manifest::read("").unwrap();
        assert!(
            empty.serve().is_none() && empty.checks().is_empty() && empty.packages().is_empty()
        );
    }

    #[test]
    fn a_manifest_that_cannot_run_is_refused_as_it_is_read() {
        for (text, refused) in [
            ("[[build]]\nrun = []", "empty"),
            ("[[build]]\nrun = [\"make\"]\nshell = true", "unknown field"),
            (
                "[[check]]\nname = \"a\"\nrun = [\"x\"]\n[[check]]\nname = \"a\"\nrun = [\"y\"]",
                "twice",
            ),
            ("[[check]]\nname = \" \"\nrun = [\"x\"]", "no name"),
            (
                "[serve]\nrun = [\"x\"]\nopen = \"http://h:{port}\"\nhealth = \"health\"",
                "health",
            ),
            ("[[build]]\nrun = [\"x\", \"{home}\"]", "placeholder"),
            ("packages = [\"Node 22\"]", "package"),
            ("tools = [\"node\"]", "renamed to `packages`"),
            ("reach = [\"crates.io\"]", "not `host:port`"),
            (
                "[[build]]\nrun = [\"x\"]\nenv = { \"1BAD\" = \"v\" }",
                "invalid name",
            ),
        ] {
            let error = Manifest::read(text).unwrap_err();
            assert!(error.to_string().contains(refused), "{text}: {error}");
        }
        assert!(matches!(
            Manifest::read("[[build]]\nrun = []"),
            Err(ManifestError::EmptyCommand)
        ));
    }

    #[test]
    fn a_manifest_lists_where_its_runs_reach_and_none_without_reach() {
        let manifest =
            Manifest::read("reach = [\"registry.npmjs.org:443\", \"*.Example.com:443\"]\n")
                .unwrap();
        assert_eq!(
            manifest
                .reach()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["registry.npmjs.org:443", "*.example.com:443"]
        );
        assert!(Manifest::read(MANIFEST).unwrap().reach().is_empty());
    }

    #[test]
    fn a_port_past_the_place_is_refused_when_expanded() {
        let manifest = Manifest::read("[[build]]\nrun = [\"x\", \"{port+100}\"]").unwrap();
        assert!(manifest.build()[0].expand(&place()).is_err());
        let manifest = Manifest::read("[[build]]\nrun = [\"x\", \"{port+99}\"]").unwrap();
        assert_eq!(
            manifest.build()[0].expand(&place()).unwrap().argv[1],
            "20199"
        );
    }
}
