//! The UI part of `install-vps`: build `ui/` while everything keeps serving,
//! then swap the deployed copy, run it as `ren-ui.service`, and check it.
//!
//! The deployed copy is `~/.local/share/ren-ui` (ui/README.md), built by
//! [`UiBuild`]. The one it replaces waits as `ren-ui.rollback` until the
//! new one answers, and is put back if it does not.
use super::{
    InstallError, InstallVps,
    rollback::{Held, Part, Upgrade},
};
use crate::{
    daemon_container::Contain,
    file::{FileError, FileMode},
    invocation::{Echo, Invocation},
    pod::Pod,
    progress::Progress,
    ui::{UiBuild, UiContainer},
    unit::ContainerUnit,
};
use ren_harness::host::HostEnv;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

/// Where the UI is built and deployed, and the programs it needs.
pub struct UiDeploy {
    build: UiBuild,
    /// The checkout being installed, which Redline sends its briefs to.
    checkout: PathBuf,
    template: PathBuf,
    staged: PathBuf,
    dir: PathBuf,
    /// Where the deployed copy waits while a new one is checked.
    rollback: PathBuf,
    unit: PathBuf,
    env_file: PathBuf,
}

/// The addresses written to a new `ui.env`.
pub struct UiUrls {
    /// The daemon as the UI server reaches it (its loopback listener).
    pub gateway: String,
    /// The daemon as clients reach it, shown in copied commands.
    pub public_gateway: String,
    /// The UI's own external origin.
    pub origin: String,
}

impl UiDeploy {
    const SERVICE: &str = "ren-ui.service";
    /// The listener `deploy/ren-ui.service` sets (`HOST`, `PORT`).
    pub const TARGET: &str = "http://127.0.0.1:3000";
    /// The template's Node, replaced with the one found on PATH.
    const TEMPLATE_NODE: &str = "/usr/bin/node";

    /// The unit's start line, with `node`.
    fn exec(node: &Path) -> String {
        format!(
            "ExecStart={} {}",
            node.display(),
            UiBuild::entry().display()
        )
    }

    pub fn new(env: &HostEnv, home: &Path, repo: &Path) -> Result<Self, InstallError> {
        let share = home.join(".local/share");
        Ok(Self {
            build: UiBuild::new(env, repo)?,
            checkout: repo.to_path_buf(),
            template: repo.join("deploy/ren-ui.service"),
            staged: share.join("ren-ui.next"),
            dir: share.join("ren-ui"),
            rollback: share.join("ren-ui.rollback"),
            unit: home.join(".config/systemd/user").join(Self::SERVICE),
            env_file: home.join(".config/ren/ui.env"),
        })
    }

    /// Builds the UI and stages a complete deployable copy beside the
    /// running one. Nothing running is touched.
    pub fn build(&self) -> Result<(), InstallError> {
        Ok(self.build.build_into(&self.staged)?)
    }

    /// Stages a copy of a deployable UI already built (a kept build's),
    /// beside the running one. Nothing running is touched.
    pub fn stage(&self, built: &Path) -> Result<(), InstallError> {
        Progress(&format!("Staging the UI in {}", self.staged.display())).show();
        if self.staged.exists() {
            fs::remove_dir_all(&self.staged).map_err(FileError::at(&self.staged))?;
        }
        Invocation::new("cp")
            .arg("-R")
            .arg(built)
            .arg(&self.staged)
            .run()?;
        Ok(())
    }

    /// Swaps in the staged copy, writes `ui.env` if missing, starts the
    /// service (in a container of its own when `contain` says the instance
    /// runs all in containers), and checks that the UI answers through to
    /// the daemon. If it does not, the copy it replaced is put back with
    /// its unit.
    pub fn install(&self, urls: &UiUrls, contain: Contain) -> Result<(), InstallError> {
        self.write_env(urls)?;
        let rendered = match contain {
            Contain::None | Contain::Runs => self.render()?,
            Contain::All => {
                let container = self.container()?;
                Progress(&format!("In container {}", container.name())).show();
                ContainerUnit::new(container.run_args()).render(&self.render()?)?
            }
        };
        if InstallVps::active(Self::SERVICE)? {
            Progress("Stopping the UI").show();
            Invocation::new("systemctl")
                .args(["--user", "stop", Self::SERVICE])
                .run()?;
        }
        let held = Held::hold(&self.dir, &self.rollback, &self.unit)?;
        Upgrade::attempt(
            held,
            Self::SERVICE,
            || {
                Progress(&format!("Installing the UI in {}", self.dir.display())).show();
                fs::rename(&self.staged, &self.dir).map_err(FileError::at(&self.dir))?;
                InstallVps::start_unit(&self.unit, &rendered, Self::SERVICE)?;
                self.check()
            },
            || self.check(),
        )
        .finish(Part::Ui)
    }

    /// Keeps an existing `ui.env`, which the operator may have edited, only
    /// adding `REN_SOURCE_DIR` (this checkout) when it names none.
    fn write_env(&self, urls: &UiUrls) -> Result<(), InstallError> {
        if self.env_file.exists() {
            let existing =
                fs::read_to_string(&self.env_file).map_err(FileError::at(&self.env_file))?;
            let Some(contents) = Self::with_source(&existing, &self.checkout) else {
                Progress(&format!(
                    "Keeping existing UI environment {}",
                    self.env_file.display()
                ))
                .show();
                return Ok(());
            };
            Progress(&format!(
                "Adding {}={} to {}",
                Self::SOURCE_KEY,
                self.checkout.display(),
                self.env_file.display()
            ))
            .show();
            return Ok(FileMode::Private.write(&self.env_file, contents.as_bytes())?);
        }
        Progress(&format!("Writing {}", self.env_file.display())).show();
        let contents = format!(
            "REN_GATEWAY_URL={}\nREN_PUBLIC_GATEWAY_URL={}\nORIGIN={}\n{}={}\n",
            urls.gateway,
            urls.public_gateway,
            urls.origin,
            Self::SOURCE_KEY,
            self.checkout.display()
        );
        Ok(FileMode::Private.write(&self.env_file, contents.as_bytes())?)
    }

    /// The deployed copy's container, reading the control token file
    /// `ui.env` names. The instance image is built by the daemon's part.
    fn container(&self) -> Result<UiContainer, InstallError> {
        let env = fs::read_to_string(&self.env_file).map_err(FileError::at(&self.env_file))?;
        let token = Self::token_file(&env);
        Ok(UiContainer::new(
            "production",
            Pod::image(),
            &self.dir,
            token.as_deref(),
        ))
    }

    /// The UI's setting for the daemon's control token file
    /// (`ui/apps/web/src/env.ts`).
    const TOKEN_KEY: &str = "REN_CONTROL_TOKEN_FILE";

    /// The control token file `env`, a `ui.env`, names, if any: its last
    /// setting that is not commented out, as systemd reads it (set empty,
    /// it names none).
    fn token_file(env: &str) -> Option<PathBuf> {
        env.lines()
            .filter_map(|line| line.trim_start().strip_prefix(Self::TOKEN_KEY))
            .filter_map(|rest| rest.trim_start().strip_prefix('='))
            .map(|value| value.trim().trim_matches('"'))
            .next_back()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    /// The UI's setting for Ren's source checkout (`ui/apps/web/src/env.ts`).
    const SOURCE_KEY: &str = "REN_SOURCE_DIR";

    /// `existing` with `REN_SOURCE_DIR=<checkout>` added, or `None` when it
    /// already sets it (commented out counts: the operator chose).
    fn with_source(existing: &str, checkout: &Path) -> Option<String> {
        let named = existing.lines().any(|line| {
            line.trim_start()
                .trim_start_matches('#')
                .trim_start()
                .strip_prefix(Self::SOURCE_KEY)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        });
        if named {
            return None;
        }
        let mut contents = existing.to_owned();
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(&format!("{}={}\n", Self::SOURCE_KEY, checkout.display()));
        Some(contents)
    }

    /// `deploy/ren-ui.service` with this host's Node.
    fn render(&self) -> Result<String, InstallError> {
        Self::render_template(&self.template, self.build.node())
    }

    /// The unit at `template` with `node` in its start line.
    fn render_template(template: &Path, node: &Path) -> Result<String, InstallError> {
        let text = fs::read_to_string(template).map_err(FileError::at(template))?;
        let exec = Self::exec(Path::new(Self::TEMPLATE_NODE));
        if !text.lines().any(|line| line == exec) {
            return Err(InstallError::UiTemplate(exec));
        }
        Ok(text.replace(&exec, &Self::exec(node)))
    }

    /// The UI's `/health` relays the daemon's, so success means both run.
    fn check(&self) -> Result<(), InstallError> {
        Progress("Checking the UI").show();
        let url = format!("{}/health", Self::TARGET);
        let curl = Invocation::new("curl").args([
            "--fail",
            "--silent",
            "--max-time",
            "2",
            "--output",
            "/dev/null",
            &url,
        ]);
        for _ in 0..15 {
            if curl.succeeds(Echo::Hide)? {
                return Ok(());
            }
            thread::sleep(Duration::from_secs(1));
        }
        Err(InstallError::UiHealth { url })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: &str = "/usr/local/bin/node";

    #[test]
    fn renders_the_unit_with_this_hosts_node() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let rendered =
            UiDeploy::render_template(&repo.join("deploy/ren-ui.service"), Path::new(NODE))
                .unwrap();
        assert!(
            rendered
                .lines()
                .any(|line| line == UiDeploy::exec(Path::new(NODE)))
        );
        assert!(!rendered.contains("/usr/bin/node"));
        assert!(rendered.contains("Environment=HOST=127.0.0.1 PORT=3000"));
    }

    #[test]
    fn adds_the_source_checkout_to_an_env_that_names_none() {
        let checkout = Path::new("/home/you/ren");
        assert_eq!(
            UiDeploy::with_source("REN_GATEWAY_URL=http://127.0.0.1:8787", checkout).as_deref(),
            Some("REN_GATEWAY_URL=http://127.0.0.1:8787\nREN_SOURCE_DIR=/home/you/ren\n")
        );
        assert_eq!(
            UiDeploy::with_source("", checkout).as_deref(),
            Some("REN_SOURCE_DIR=/home/you/ren\n")
        );
        // Set, or commented out, it is the operator's.
        for kept in [
            "ORIGIN=x\nREN_SOURCE_DIR=/srv/ren\n",
            "# REN_SOURCE_DIR = /srv/ren\n",
        ] {
            assert_eq!(UiDeploy::with_source(kept, checkout), None);
        }
        assert!(UiDeploy::with_source("REN_SOURCE_DIRS=/x\n", checkout).is_some());
    }

    #[test]
    fn reads_the_token_file_its_env_names() {
        let named = "ORIGIN=x\n# REN_CONTROL_TOKEN_FILE=/old\nREN_CONTROL_TOKEN_FILE = \"/s/control.token\"\n";
        assert_eq!(
            UiDeploy::token_file(named).as_deref(),
            Some(Path::new("/s/control.token"))
        );
        for none in [
            "",
            "ORIGIN=x\n",
            "REN_CONTROL_TOKEN_FILE=\n",
            "REN_CONTROL_TOKEN_FILES=/x\n",
        ] {
            assert_eq!(UiDeploy::token_file(none), None, "{none:?}");
        }
    }

    #[test]
    fn its_contained_unit_runs_the_ui_in_a_container() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let rendered =
            UiDeploy::render_template(&repo.join("deploy/ren-ui.service"), Path::new(NODE))
                .unwrap();
        let container = UiContainer::new(
            "production",
            "localhost/ren-instance:abc".to_owned(),
            Path::new("/home/op/.local/share/ren-ui"),
            None,
        );
        let unit = ContainerUnit::new(container.run_args())
            .render(&rendered)
            .unwrap();
        assert!(unit.contains("\nExecStart=:\"/usr/bin/podman\" \"run\""));
        assert!(unit.contains("\"node\" \"apps/web/build\"\n"));
        assert!(unit.contains("EnvironmentFile=%h/.config/ren/ui.env"));
        assert!(!unit.contains(NODE));
        assert!(!unit.contains("NoNewPrivileges"));
    }

    #[test]
    fn refuses_a_template_without_the_node_line() {
        let dir = tempfile::tempdir().unwrap();
        let template = dir.path().join("ren-ui.service");
        fs::write(&template, "[Service]\nExecStart=/opt/node build\n").unwrap();
        assert!(matches!(
            UiDeploy::render_template(&template, Path::new(NODE)),
            Err(InstallError::UiTemplate(_))
        ));
    }
}
