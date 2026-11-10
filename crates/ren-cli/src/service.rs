//! Native service definitions, with no manager execution or implicit installation.
use clap::ValueEnum;
use ren_daemon::config::{Config, ConfigError};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Which service manager a definition is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Manager {
    Launchd,
    Systemd,
}

/// Why a service definition was refused or not written.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("a background service requires a fixed, nonzero listen port")]
    EphemeralPort,
    #[error(
        "--label and --log-dir apply only to launchd; systemd uses its unit filename and journal"
    )]
    LaunchdOnly,
    #[error(
        "launchd label must start with a letter and contain only ASCII letters, digits, dots, hyphens, or underscores (maximum 128 characters)"
    )]
    Label,
    #[error("service paths must not be empty")]
    EmptyPath,
    #[error("cannot resolve absolute service path")]
    Absolute(#[source] std::io::Error),
    #[error("cannot inspect service executable")]
    Executable(#[source] std::io::Error),
    #[error("service executable must be a regular file")]
    NotAFile,
    #[error("service executable has no executable mode bits")]
    NotExecutable,
    #[error("service paths must be valid UTF-8")]
    NotUnicode,
    #[error("service paths must not contain control characters or XML noncharacters")]
    Unprintable,
    #[error("cannot create temporary service definition; output parent must exist")]
    Temporary(#[source] std::io::Error),
    #[error("cannot write service definition")]
    Write(#[source] std::io::Error),
    #[error("cannot publish service definition; existing files and symlinks are never overwritten")]
    Exists,
}

/// What a service definition is built from.
#[derive(Debug, Clone, Copy)]
pub struct ServiceOptions<'a> {
    pub manager: Manager,
    pub config_path: Option<&'a Path>,
    pub data_dir: &'a Path,
    pub binary: &'a Path,
    /// launchd only.
    pub log_dir: Option<&'a Path>,
    /// launchd only.
    pub label: Option<&'a str>,
}

/// A validated service definition, ready to render.
#[derive(Debug)]
pub struct Definition {
    manager: Manager,
    arguments: Vec<String>,
    log_dir: String,
    label: String,
}

impl Definition {
    const DEFAULT_LABEL: &'static str = "dev.ren.gateway";

    pub fn new(config: &Config, options: ServiceOptions<'_>) -> Result<Self, ServiceError> {
        // The service's own environment supplies operator keys at start.
        config.check()?;
        if config.listen.port() == 0 {
            return Err(ServiceError::EphemeralPort);
        }
        match options.manager {
            Manager::Systemd if options.label.is_some() || options.log_dir.is_some() => {
                return Err(ServiceError::LaunchdOnly);
            }
            Manager::Systemd | Manager::Launchd => {}
        }
        let label = options.label.unwrap_or(Self::DEFAULT_LABEL);
        if label.len() > 128
            || !label.starts_with(|c: char| c.is_ascii_alphabetic())
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            return Err(ServiceError::Label);
        }
        let binary = Self::absolute(options.binary)?;
        let metadata = std::fs::metadata(&binary).map_err(ServiceError::Executable)?;
        if !metadata.is_file() {
            return Err(ServiceError::NotAFile);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(ServiceError::NotExecutable);
            }
        }
        let mut arguments = vec![Self::text(&binary)?];
        if let Some(path) = options.config_path {
            arguments.push("--config".into());
            arguments.push(Self::text(&Self::absolute(path)?)?);
        }
        let data_dir = Self::absolute(options.data_dir)?;
        arguments.push("--data-dir".into());
        arguments.push(Self::text(&data_dir)?);
        arguments.push("serve".into());
        let log_dir = Self::absolute(
            &options
                .log_dir
                .map_or_else(|| data_dir.join("logs"), Path::to_path_buf),
        )?;
        Ok(Self {
            manager: options.manager,
            arguments,
            log_dir: Self::text(&log_dir)?,
            label: label.into(),
        })
    }

    pub fn render(&self) -> String {
        match self.manager {
            Manager::Systemd => {
                // systemd's ':' prefix disables dollar expansion, including in
                // arguments; '%' specifiers still need doubling. No shell runs.
                let command = self
                    .arguments
                    .iter()
                    .map(|value| Self::systemd_word(value))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "# Generated by ren service render. Run as a user service.\n[Unit]\nDescription=Ren gateway for coding agents\nStartLimitIntervalSec=60\nStartLimitBurst=5\n\n[Service]\nType=exec\nExecStart=:{command}\nWorkingDirectory=/\nRestart=on-failure\nRestartSec=5\nTimeoutStopSec=30\nKillSignal=SIGTERM\nKillMode=control-group\nUMask=0077\nNoNewPrivileges=true\nLimitCORE=0\nStandardOutput=journal\nStandardError=journal\nSyslogIdentifier=ren\n\n[Install]\nWantedBy=default.target\n"
                )
            }
            Manager::Launchd => {
                let args = self
                    .arguments
                    .iter()
                    .map(|arg| format!("    <string>{}</string>\n", Self::xml(arg)))
                    .collect::<String>();
                let log = Self::xml(&format!(
                    "{}/daemon.log",
                    self.log_dir.trim_end_matches('/')
                ));
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key><string>{}</string>\n  <key>ProgramArguments</key>\n  <array>\n{args}  </array>\n  <key>WorkingDirectory</key><string>/</string>\n  <key>RunAtLoad</key><true/>\n  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n  <key>ThrottleInterval</key><integer>10</integer>\n  <key>ExitTimeOut</key><integer>30</integer>\n  <key>Umask</key><integer>63</integer>\n  <key>SoftResourceLimits</key><dict><key>Core</key><integer>0</integer></dict>\n  <key>HardResourceLimits</key><dict><key>Core</key><integer>0</integer></dict>\n  <key>StandardOutPath</key><string>{log}</string>\n  <key>StandardErrorPath</key><string>{log}</string>\n</dict>\n</plist>\n",
                    Self::xml(&self.label)
                )
            }
        }
    }

    fn absolute(path: &Path) -> Result<PathBuf, ServiceError> {
        if path.as_os_str().is_empty() {
            return Err(ServiceError::EmptyPath);
        }
        std::path::absolute(path).map_err(ServiceError::Absolute)
    }

    fn text(path: &Path) -> Result<String, ServiceError> {
        let text = path.to_str().ok_or(ServiceError::NotUnicode)?;
        if text
            .chars()
            .any(|c| c.is_control() || c == '\u{fffe}' || c == '\u{ffff}')
        {
            return Err(ServiceError::Unprintable);
        }
        Ok(text.into())
    }

    fn xml(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn systemd_word(text: &str) -> String {
        format!(
            "\"{}\"",
            text.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
        )
    }

    /// Publish a complete, synced file at `path`, never replacing an existing
    /// file or symlink: a failed write must not leave a half-written
    /// LaunchAgent where a later login could load it.
    pub fn write_new(path: &Path, contents: &str) -> Result<(), ServiceError> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(ServiceError::Temporary)?;
        file.write_all(contents.as_bytes())
            .map_err(ServiceError::Write)?;
        file.as_file().sync_all().map_err(ServiceError::Write)?;
        file.persist_noclobber(path)
            .map_err(|_| ServiceError::Exists)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, Definition, Manager, Path, ServiceOptions};

    fn options<'a>(
        manager: Manager,
        config_path: Option<&'a Path>,
        data_dir: &'a Path,
        binary: &'a Path,
        log_dir: Option<&'a Path>,
        label: Option<&'a str>,
    ) -> ServiceOptions<'a> {
        ServiceOptions {
            manager,
            config_path,
            data_dir,
            binary,
            log_dir,
            label,
        }
    }
    #[test]
    fn escapes_service_syntax_without_shell_or_environment_expansion() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir
            .path()
            .join("settings & <\"quote\"> ' $HOME %h \\ .toml");
        let state = dir.path().join("state $USER %n");
        let binary = std::env::current_exe().unwrap();
        let systemd = Definition::new(
            &Config::default(),
            options(Manager::Systemd, Some(&config), &state, &binary, None, None),
        )
        .unwrap()
        .render();
        assert!(systemd.contains("ExecStart=:\"/"));
        assert!(systemd.contains("$HOME %%h \\\\"));
        assert!(systemd.contains("\\\"quote\\\""));
        assert!(systemd.contains("--data-dir\" \"/"));
        let plist = Definition::new(
            &Config::default(),
            options(Manager::Launchd, Some(&config), &state, &binary, None, None),
        )
        .unwrap()
        .render();
        assert!(plist.contains("&amp; &lt;&quot;quote&quot;&gt; &apos; $HOME %h"));
        assert!(plist.contains("<integer>63</integer>"));
    }
    #[test]
    fn refuses_unstable_or_invalid_service_configuration() {
        let binary = std::env::current_exe().unwrap();
        let config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            ..Config::default()
        };
        assert!(
            Definition::new(
                &config,
                options(
                    Manager::Launchd,
                    None,
                    Path::new("state"),
                    &binary,
                    None,
                    None
                ),
            )
            .is_err()
        );
        for label in ["", "bad\nkey", "../bad", "0bad"] {
            assert!(
                Definition::new(
                    &Config::default(),
                    options(
                        Manager::Launchd,
                        None,
                        Path::new("state"),
                        &binary,
                        None,
                        Some(label)
                    ),
                )
                .is_err()
            );
        }
        assert!(
            Definition::new(
                &Config::default(),
                options(
                    Manager::Systemd,
                    None,
                    Path::new("bad\npath"),
                    &binary,
                    None,
                    None
                ),
            )
            .is_err()
        );
        assert!(
            Definition::new(
                &Config::default(),
                options(
                    Manager::Systemd,
                    None,
                    Path::new("state"),
                    &binary,
                    Some(Path::new("logs")),
                    None
                ),
            )
            .is_err()
        );
    }
    #[test]
    fn output_never_replaces_existing_files_or_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ren.service");
        Definition::write_new(&file, "original").unwrap();
        assert!(Definition::write_new(&file, "replacement").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let link = dir.path().join("link");
            symlink(&file, &link).unwrap();
            assert!(Definition::write_new(&link, "replacement").is_err());
            assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
        }
    }
}
