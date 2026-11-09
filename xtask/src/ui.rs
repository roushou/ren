//! The UI part of `install-vps`: build `ui/` while everything keeps serving,
//! then swap the deployed copy, run it as `tgv-ui.service`, and check it.
//!
//! The deployed copy is `~/.local/share/tgv-ui` (ui/README.md): the build,
//! `package.json`, `bun.lock`, and production dependencies. The one before it
//! is kept as `tgv-ui.previous`.
use crate::install::{InstallError, InstallVps};
use crate::invocation::{Echo, Invocation};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use tgv_harness::host::HostEnv;

/// Where the UI is built and deployed, and the programs it needs.
pub struct UiDeploy {
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
    const SERVICE: &str = "tgv-ui.service";
    /// The listener `deploy/tgv-ui.service` sets (`HOST`, `PORT`).
    pub const TARGET: &str = "http://127.0.0.1:3000";
    /// The template's Node path, replaced with the one found on PATH.
    const EXEC: &str = "ExecStart=/usr/bin/node build";

    pub fn new(env: &HostEnv, home: &Path, repo: &Path) -> Result<Self, InstallError> {
        let share = home.join(".local/share");
        Ok(Self {
            source: repo.join("ui"),
            template: repo.join("deploy/tgv-ui.service"),
            staged: share.join("tgv-ui.next"),
            dir: share.join("tgv-ui"),
            previous: share.join("tgv-ui.previous"),
            unit: home.join(".config/systemd/user").join(Self::SERVICE),
            env_file: home.join(".config/tgv/ui.env"),
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

    /// Keeps an existing `ui.env`, which the operator may have edited.
    fn write_env(&self, urls: &UiUrls) -> Result<(), InstallError> {
        if self.env_file.exists() {
            InstallVps::step(&format!(
                "Keeping existing UI environment {}",
                self.env_file.display()
            ));
            return Ok(());
        }
        InstallVps::step(&format!("Writing {}", self.env_file.display()));
        let contents = format!(
            "TGV_GATEWAY_URL={}\nTGV_PUBLIC_GATEWAY_URL={}\nORIGIN={}\n",
            urls.gateway, urls.public_gateway, urls.origin
        );
        InstallVps::write_private(&self.env_file, contents.as_bytes(), 0o600)
    }

    /// `deploy/tgv-ui.service` with this host's Node.
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
        let rendered = deploy(&repo.join("deploy/tgv-ui.service"))
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
    fn refuses_a_template_without_the_node_line() {
        let dir = tempfile::tempdir().unwrap();
        let template = dir.path().join("tgv-ui.service");
        fs::write(&template, "[Service]\nExecStart=/opt/node build\n").unwrap();
        assert!(matches!(
            deploy(&template).render(),
            Err(InstallError::UiTemplate(_))
        ));
    }
}
