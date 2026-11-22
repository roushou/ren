//! An instance's daemon in a container of its own (docs/plans/self-build.md,
//! phase 11). It sees, each at its host path, only what its configuration
//! says it works with: the build it runs (read-only), its configuration
//! (read-only), its data, its run roots, places, and scratch folder,
//! `[runs.container]`'s shared paths, what every client keeps under the
//! home (the daemon signs clients in and lists their models itself), the
//! Podman API socket it starts run containers through, and the SSH agent.
//! It shares the host's network and runs as the operator, as a replica's
//! does. What it sees is derived here, purely; starting it is the caller's.

use ren_daemon::config::Config;
use ren_daemon::host::Mount;
use ren_harness::{Harness, host::HostEnv};
use ren_protocol::label::Labeled as _;
use std::path::{Path, PathBuf};

/// One instance's daemon container.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct DaemonContainer {
    name: String,
    image: String,
    ren: PathBuf,
    config: PathBuf,
    data_dir: PathBuf,
    mounts: Vec<Mount>,
    env: Vec<(String, String)>,
}

/// Why an instance's daemon cannot run in a container.
#[derive(Debug, thiserror::Error)]
pub enum DaemonContainerError {
    #[error(
        "a daemon in a container holds its runs in containers: set [runs] hold = \"container\" and [runs.container] podman_socket"
    )]
    RunsNotContained,
    #[error("HOME is not set")]
    NoHome,
}

impl DaemonContainer {
    /// Seconds Podman gives the daemon to stop before killing it: `ren`
    /// gives its requests 10, then drains capture.
    const STOP_SECONDS: u32 = 30;

    /// `name`'s daemon, running `ren` from `image` with the configuration
    /// at `config_path` (`config`, as read) and data in `data_dir`, on a
    /// host whose environment is `env`; `vars` are set for it.
    pub fn new(
        name: &str,
        image: String,
        ren: &Path,
        (config_path, config): (&Path, &Config),
        data_dir: &Path,
        env: &HostEnv,
        vars: Vec<(String, String)>,
    ) -> Result<Self, DaemonContainerError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(DaemonContainerError::NoHome)?;
        let runs = &config.runs;
        let containers = runs.container.clone();
        if runs.hold != ren_daemon::config::RunHold::Container || containers.podman_socket.is_none()
        {
            return Err(DaemonContainerError::RunsNotContained);
        }
        let anchored = |path: &Path| match path.strip_prefix("~") {
            Ok(rest) => home.join(rest),
            Err(_) => path.to_path_buf(),
        };
        let places = runs.places_in(env);
        let mut mounts = vec![
            Mount::read_only(ren),
            Mount::read_only(config_path),
            Mount::read_write(data_dir),
        ];
        mounts.extend(
            containers
                .read_only
                .iter()
                .map(|path| Mount::read_only(anchored(path))),
        );
        mounts.extend(
            containers
                .read_write
                .iter()
                .map(|path| Mount::read_write(anchored(path))),
        );
        mounts.extend(
            runs.roots
                .iter()
                .chain(&places.places)
                .chain(&places.scratch)
                .map(Mount::read_write),
        );
        mounts.extend(
            Harness::ALL
                .iter()
                .flat_map(|harness| harness.profile().keeps)
                .map(|entry| entry.under(&home))
                .filter(|path| path.exists())
                .map(Mount::read_write),
        );
        let mut vars = vars;
        vars.push(("HOME".to_owned(), home.display().to_string()));
        if let Some(socket) = containers.podman_socket.as_deref() {
            // Its folder, not the socket itself: a socket made again (its
            // unit restarted) is then still the one the daemon sees.
            let socket = anchored(socket);
            mounts.push(Mount::read_write(socket.parent().unwrap_or(&socket)));
        }
        if let Some(agent) = env.absolute_path("SSH_AUTH_SOCK") {
            vars.push(("SSH_AUTH_SOCK".to_owned(), agent.display().to_string()));
            mounts.push(Mount::read_write(agent));
        }
        Ok(Self {
            name: format!("ren-{name}-daemon"),
            image,
            ren: ren.to_path_buf(),
            config: config_path.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            mounts,
            env: vars,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `podman run` arguments that run it in the foreground, as a unit's
    /// main process: replacing a container of its name left behind, and
    /// removed when it exits.
    pub fn run_args(&self) -> Vec<String> {
        let mut args = vec![
            "run".to_owned(),
            "--rm".to_owned(),
            "--replace".to_owned(),
            format!("--name={}", self.name),
            "--network=host".to_owned(),
            "--userns=keep-id".to_owned(),
            format!("--stop-timeout={}", Self::STOP_SECONDS),
        ];
        args.extend(self.mounts.iter().map(|mount| {
            let access = match mount.access {
                ren_daemon::host::Access::ReadOnly => "ro",
                ren_daemon::host::Access::ReadWrite => "rw",
            };
            let path = mount.path.display();
            format!("--volume={path}:{path}:{access}")
        }));
        args.extend(
            self.env
                .iter()
                .map(|(key, value)| format!("--env={key}={value}")),
        );
        args.push(format!("--entrypoint={}", self.ren.display()));
        args.push(self.image.clone());
        args.extend([
            "--config".to_owned(),
            self.config.display().to_string(),
            "--data-dir".to_owned(),
            self.data_dir.display().to_string(),
            "serve".to_owned(),
        ]);
        args
    }
}

#[cfg(test)]
mod tests {
    use super::{DaemonContainer, DaemonContainerError};
    use ren_daemon::config::Config;
    use ren_harness::host::HostEnv;
    use std::path::Path;

    const CONTAINED: &str = r#"
[runs]
roots = ["/home/op/dev"]
places = ["~/.config/nvim"]
scratch = "~/.local/share/ren/scratch"
hold = "container"
[runs.container]
image = "localhost/ren-instance:abc"
read_only = ["/usr", "~/.local/bin"]
read_write = ["~/.cargo"]
podman_socket = "/run/user/1000/podman/podman.sock"
"#;

    fn container(text: &str) -> Result<DaemonContainer, DaemonContainerError> {
        let config: Config = toml::from_str(text).unwrap();
        DaemonContainer::new(
            "production",
            "localhost/ren-instance:abc".into(),
            Path::new("/home/op/.local/share/ren/builds/c0ffee/ren"),
            (Path::new("/home/op/.config/ren/ren.toml"), &config),
            Path::new("/home/op/.local/state/ren"),
            &HostEnv::from_pairs([("HOME", "/home/op"), ("SSH_AUTH_SOCK", "/run/agent")]),
            vec![("REN_KEY".into(), "k".into())],
        )
    }

    #[test]
    fn it_sees_what_its_configuration_says_it_works_with_and_nothing_else_of_the_home() {
        let args = container(CONTAINED).unwrap().run_args();
        let volumes: Vec<&str> = args
            .iter()
            .filter_map(|arg| arg.strip_prefix("--volume="))
            .map(|volume| volume.rsplit_once(':').unwrap().0)
            .map(|volume| volume.split_once(':').unwrap().0)
            .collect();
        // The test host's home has no client folders, so none is mounted.
        assert_eq!(
            volumes,
            [
                "/home/op/.local/share/ren/builds/c0ffee/ren",
                "/home/op/.config/ren/ren.toml",
                "/home/op/.local/state/ren",
                "/usr",
                "/home/op/.local/bin",
                "/home/op/.cargo",
                "/home/op/dev",
                "/home/op/.config/nvim",
                "/home/op/.local/share/ren/scratch",
                "/run/user/1000/podman",
                "/run/agent",
            ]
        );
        assert!(
            args.iter().any(|arg| arg
                == "--volume=/home/op/.config/ren/ren.toml:/home/op/.config/ren/ren.toml:ro")
        );
        assert!(!volumes.contains(&"/home/op"));
    }

    #[test]
    fn it_runs_ren_serve_in_the_foreground_as_the_operator() {
        let container = container(CONTAINED).unwrap();
        let args = container.run_args();
        assert_eq!(container.name(), "ren-production-daemon");
        for flag in ["--rm", "--replace", "--network=host", "--userns=keep-id"] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}");
        }
        assert!(!args.iter().any(|arg| arg == "--detach"));
        for var in [
            "--env=REN_KEY=k",
            "--env=HOME=/home/op",
            "--env=SSH_AUTH_SOCK=/run/agent",
        ] {
            assert!(args.iter().any(|arg| arg == var), "{var}");
        }
        assert_eq!(
            &args[args.len() - 7..],
            [
                "--entrypoint=/home/op/.local/share/ren/builds/c0ffee/ren",
                "localhost/ren-instance:abc",
                "--config",
                "/home/op/.config/ren/ren.toml",
                "--data-dir",
                "/home/op/.local/state/ren",
                "serve",
            ]
        );
    }

    #[test]
    fn a_daemon_whose_runs_are_not_contained_is_refused() {
        // Under systemd units it would hold runs it cannot reach from a
        // container; without the socket it could not start their containers.
        assert!(matches!(
            container("[runs]\nroots = [\"/home/op/dev\"]\n"),
            Err(DaemonContainerError::RunsNotContained)
        ));
        let no_socket = CONTAINED.replace(
            "podman_socket = \"/run/user/1000/podman/podman.sock\"\n",
            "",
        );
        assert!(matches!(
            container(&no_socket),
            Err(DaemonContainerError::RunsNotContained)
        ));
    }
}
