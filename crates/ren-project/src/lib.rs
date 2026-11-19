//! A project's contract with Ren (docs/plans/self-build.md, phase 6), pure:
//! how a repository is built, served, checked, and quiesced, declared in
//! [`ProjectContract::FILE`] at its root, so a workspace of it gets checks
//! and a preview as Ren's own do.
//!
//! ```toml
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
//! Deploying runs in a build the canary checked, when the operator's
//! promotion policy says (`ren project canary`, `ren project promote`).
//!
//! Every command is an argv, never a shell line. Arguments and values may
//! name the place a step runs for ([`Place`]): `{port}` (its first port),
//! `{port+N}` (another of its ports), `{data}` (its data folder), and
//! `{workspace}` (its folder). The contract is parsed once
//! ([`ProjectContract::read`]); an unknown placeholder is refused there, and
//! a port past the place's range when a step is expanded.

mod template;

pub use template::{Template, TemplateError};

use serde::Deserialize;
use std::{collections::BTreeMap, path::PathBuf};

/// How a project is built, served, checked, and quiesced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectContract {
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

/// Where a step runs: a workspace's folder, data folder, and ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub workspace: PathBuf,
    pub data: PathBuf,
    /// Its ports, first and last; none outside a workspace, where a step
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

/// Why a contract does not read.
#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("ren-project.toml does not read: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("a command in ren-project.toml is empty")]
    EmptyCommand,
    #[error("check {0:?} is named twice in ren-project.toml")]
    DuplicateCheck(String),
    #[error("a check in ren-project.toml has no name")]
    UnnamedCheck,
    #[error("the serve health path must start with `/`: {0:?}")]
    Health(String),
    #[error("an environment variable in ren-project.toml has an invalid name: {0:?}")]
    EnvName(String),
    #[error(transparent)]
    Template(#[from] TemplateError),
}

/// The file's own shape, read once and parsed into the contract.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractFile {
    #[serde(default)]
    build: Vec<StepFile>,
    serve: Option<ServeFile>,
    #[serde(default)]
    check: Vec<CheckFile>,
    quiesce: Option<StepFile>,
    deploy: Option<StepFile>,
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

impl ProjectContract {
    /// The contract's file, at the repository's root.
    pub const FILE: &str = "ren-project.toml";

    /// The contract `text` declares.
    pub fn read(text: &str) -> Result<Self, ContractError> {
        let file: ContractFile = toml::from_str(text)?;
        let build = file
            .build
            .into_iter()
            .map(|step| Step::read(step.run, step.env))
            .collect::<Result<_, _>>()?;
        let serve = file
            .serve
            .map(|serve| -> Result<Serve, ContractError> {
                if !serve.health.starts_with('/') {
                    return Err(ContractError::Health(serve.health));
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
                return Err(ContractError::UnnamedCheck);
            }
            if checks.iter().any(|known| known.name == name) {
                return Err(ContractError::DuplicateCheck(name));
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
        Ok(Self {
            build,
            serve,
            checks,
            quiesce,
            deploy,
        })
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
    fn read(run: Vec<String>, env: BTreeMap<String, String>) -> Result<Self, ContractError> {
        if run.first().is_none_or(|program| program.trim().is_empty()) {
            return Err(ContractError::EmptyCommand);
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
                    Err(ContractError::EnvName(name))
                }
            })
            .collect::<Result<_, ContractError>>()?;
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
    use super::{ContractError, Place, ProjectContract};
    use std::path::PathBuf;

    const CONTRACT: &str = r#"
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
run = ["./deploy.sh", "{workspace}"]
"#;

    fn place() -> Place {
        Place {
            workspace: PathBuf::from("/data/worktrees/acme-web-2"),
            data: PathBuf::from("/data/worktrees/.data/acme-web-2"),
            ports: Some((20_100, 20_199)),
        }
    }

    #[test]
    fn a_contract_reads_and_expands_for_its_place() {
        let contract = ProjectContract::read(CONTRACT).unwrap();
        assert_eq!(contract.build().len(), 2);
        let serve = contract.serve().unwrap();
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
        assert_eq!(contract.checks()[0].name(), "tests");
        assert_eq!(
            contract.quiesce().unwrap().expand(&place()).unwrap().argv[4],
            "http://127.0.0.1:20100/drain"
        );
        assert_eq!(
            contract.deploy().unwrap().expand(&place()).unwrap().argv,
            ["./deploy.sh", "/data/worktrees/acme-web-2"]
        );
        // A contract may declare as little as nothing.
        let empty = ProjectContract::read("").unwrap();
        assert!(empty.serve().is_none() && empty.checks().is_empty());
    }

    #[test]
    fn a_contract_that_cannot_run_is_refused_as_it_is_read() {
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
            (
                "[[build]]\nrun = [\"x\"]\nenv = { \"1BAD\" = \"v\" }",
                "invalid name",
            ),
        ] {
            let error = ProjectContract::read(text).unwrap_err();
            assert!(error.to_string().contains(refused), "{text}: {error}");
        }
        assert!(matches!(
            ProjectContract::read("[[build]]\nrun = []"),
            Err(ContractError::EmptyCommand)
        ));
    }

    #[test]
    fn a_port_past_the_place_is_refused_when_expanded() {
        let contract = ProjectContract::read("[[build]]\nrun = [\"x\", \"{port+100}\"]").unwrap();
        assert!(contract.build()[0].expand(&place()).is_err());
        let contract = ProjectContract::read("[[build]]\nrun = [\"x\", \"{port+99}\"]").unwrap();
        assert_eq!(
            contract.build()[0].expand(&place()).unwrap().argv[1],
            "20199"
        );
    }
}
