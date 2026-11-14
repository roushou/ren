//! The UI part of `install-vps`: build `ui/` while everything keeps serving,
//! then swap the deployed copy, run it as `ren-ui.service`, and check it.
//!
//! The deployed copy is `~/.local/share/ren-ui` (ui/README.md): the build,
//! `package.json`, `bun.lock`, and production dependencies. The one before it
//! is kept as `ren-ui.previous`.
use crate::install::{InstallError, InstallVps};
use crate::invocation::{Echo, Invocation};
use ren_harness::host::HostEnv;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

/// Where the UI is built and deployed, and the programs it needs.
pub struct UiDeploy {
    /// The checkout being installed, which Redline sends its briefs to.
    checkout: PathBuf,
    source: PathBuf,
    template: PathBuf,
    staged: PathBuf,
    dir: PathBuf,
    previous: PathBuf,
    unit: PathBuf,
    env_file: PathBuf,
    node: PathBuf,
    bun: PathBuf,
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
    /// The template's Node path, replaced with the one found on PATH.
    const EXEC: &str = "ExecStart=/usr/bin/node build";

    pub fn new(env: &HostEnv, home: &Path, repo: &Path) -> Result<Self, InstallError> {
        let share = home.join(".local/share");
        Ok(Self {
            checkout: repo.to_path_buf(),
            source: repo.join("ui"),
            template: repo.join("deploy/ren-ui.service"),
            staged: share.join("ren-ui.next"),
            dir: share.join("ren-ui"),
            previous: share.join("ren-ui.previous"),
            unit: home.join(".config/systemd/user").join(Self::SERVICE),
            env_file: home.join(".config/ren/ui.env"),
            node: Self::program(env, "node")?,
            bun: Self::program(env, "bun")?,
        })
    }

    /// The first executable `name` on PATH, as an absolute path.
    fn program(env: &HostEnv, name: &'static str) -> Result<PathBuf, InstallError> {
        env.get("PATH")
            .unwrap_or_default()
            .split(':')
            .map(|dir| Path::new(dir).join(name))
            .find(|path| {
                path.is_absolute()
                    && fs::metadata(path)
                        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            })
            .ok_or(InstallError::MissingProgram(name))
    }

    /// Builds the UI and stages a complete deployable copy beside the
    /// running one. Nothing running is touched.
    pub fn build(&self) -> Result<(), InstallError> {
        self.build_into(&self.staged)
    }

    /// Builds the UI and stages a complete deployable copy in `staged`:
    /// the build, `package.json`, `bun.lock`, and production dependencies.
    pub fn build_into(&self, staged: &Path) -> Result<(), InstallError> {
        InstallVps::step("Building the UI");
        Invocation::new(&self.bun)
            .args(["install", "--frozen-lockfile"])
            .current_dir(&self.source)
            .run()?;
        Invocation::new(&self.bun)
            .args(["run", "build"])
            .current_dir(&self.source)
            .run()?;
        InstallVps::step(&format!("Staging the UI in {}", staged.display()));
        if staged.exists() {
            fs::remove_dir_all(staged).map_err(InstallError::file(staged))?;
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(staged)
            .map_err(InstallError::file(staged))?;
        Invocation::new("cp")
            .arg("-R")
            .arg(self.source.join("build"))
            .arg(staged.join("build"))
            .run()?;
        for file in ["package.json", "bun.lock"] {
            let target = staged.join(file);
            fs::copy(self.source.join(file), &target).map_err(InstallError::file(&target))?;
        }
        Invocation::new(&self.bun)
            .args([
                "install",
                "--production",
                "--frozen-lockfile",
                "--ignore-scripts",
            ])
            .current_dir(staged)
            .run()
    }

    /// The Node that runs the UI.
    pub fn node(&self) -> &Path {
        &self.node
    }

    /// Swaps in the staged copy, writes `ui.env` if missing, starts the
    /// service, and checks that the UI answers through to the daemon.
    pub fn install(&self, urls: &UiUrls) -> Result<(), InstallError> {
        self.write_env(urls)?;
        let rendered = self.render()?;
        if InstallVps::active(Self::SERVICE)? {
            InstallVps::step("Stopping the UI");
            Invocation::new("systemctl")
                .args(["--user", "stop", Self::SERVICE])
                .run()?;
        }
        if self.previous.exists() {
            fs::remove_dir_all(&self.previous).map_err(InstallError::file(&self.previous))?;
        }
        if self.dir.exists() {
            fs::rename(&self.dir, &self.previous).map_err(InstallError::file(&self.previous))?;
        }
        InstallVps::step(&format!("Installing the UI in {}", self.dir.display()));
        fs::rename(&self.staged, &self.dir).map_err(InstallError::file(&self.dir))?;
        InstallVps::start_unit(&self.unit, &rendered, Self::SERVICE)?;
        self.check()
    }

    /// Keeps an existing `ui.env`, which the operator may have edited, only
    /// adding `REN_SOURCE_DIR` (this checkout) when it names none.
    fn write_env(&self, urls: &UiUrls) -> Result<(), InstallError> {
        if self.env_file.exists() {
            let existing =
                fs::read_to_string(&self.env_file).map_err(InstallError::file(&self.env_file))?;
            let Some(contents) = Self::with_source(&existing, &self.checkout) else {
                InstallVps::step(&format!(
                    "Keeping existing UI environment {}",
                    self.env_file.display()
                ));
                return Ok(());
            };
            InstallVps::step(&format!(
                "Adding {}={} to {}",
                Self::SOURCE_KEY,
                self.checkout.display(),
                self.env_file.display()
            ));
            return InstallVps::write_private(&self.env_file, contents.as_bytes(), 0o600);
        }
        InstallVps::step(&format!("Writing {}", self.env_file.display()));
        let contents = format!(
            "REN_GATEWAY_URL={}\nREN_PUBLIC_GATEWAY_URL={}\nORIGIN={}\n{}={}\n",
            urls.gateway,
            urls.public_gateway,
            urls.origin,
            Self::SOURCE_KEY,
            self.checkout.display()
        );
        InstallVps::write_private(&self.env_file, contents.as_bytes(), 0o600)
    }

    /// The UI's setting for Ren's source checkout (`ui/src/env.ts`).
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
        let template =
            fs::read_to_string(&self.template).map_err(InstallError::file(&self.template))?;
        if !template.lines().any(|line| line == Self::EXEC) {
            return Err(InstallError::UiTemplate(Self::EXEC));
        }
        Ok(template.replace(
            Self::EXEC,
            &format!("ExecStart={} build", self.node.display()),
        ))
    }

    /// The UI's `/health` relays the daemon's, so success means both run.
    fn check(&self) -> Result<(), InstallError> {
        InstallVps::step("Checking the UI");
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
        Err(InstallError::UiHealth {
            url,
            previous: self.previous.clone(),
            dir: self.dir.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deploy(template: &Path) -> UiDeploy {
        let none = PathBuf::new();
        UiDeploy {
            checkout: none.clone(),
            source: none.clone(),
            template: template.to_path_buf(),
            staged: none.clone(),
            dir: none.clone(),
            previous: none.clone(),
            unit: none.clone(),
            env_file: none.clone(),
            node: PathBuf::from("/usr/local/bin/node"),
            bun: none,
        }
    }

    #[test]
    fn renders_the_unit_with_this_hosts_node() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let rendered = deploy(&repo.join("deploy/ren-ui.service"))
            .render()
            .unwrap();
        assert!(
            rendered
                .lines()
                .any(|line| line == "ExecStart=/usr/local/bin/node build")
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
            deploy(&template).render(),
            Err(InstallError::UiTemplate(_))
        ));
    }
}
