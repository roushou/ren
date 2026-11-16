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
    file::{FileError, FileMode},
    invocation::{Echo, Invocation},
    progress::Progress,
    ui::UiBuild,
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

    /// Swaps in the staged copy, writes `ui.env` if missing, starts the
    /// service, and checks that the UI answers through to the daemon. If it
    /// does not, the copy it replaced is put back with its unit.
    pub fn install(&self, urls: &UiUrls) -> Result<(), InstallError> {
        self.write_env(urls)?;
        let rendered = self.render()?;
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
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
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
