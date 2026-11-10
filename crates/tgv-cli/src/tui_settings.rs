//! This terminal client's preferences, separate from gateway and daemon settings.

use serde::{Deserialize, Serialize};
use std::{
    io::Write as _,
    path::{Path, PathBuf},
};
use tgv_harness::host::HostEnv;
use tgv_protocol::label::Stored;
use tgv_tui::ThemeName;

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TuiSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<Stored<ThemeName>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SettingsError {
    #[error("cannot locate TUI settings: set HOME or XDG_CONFIG_HOME to an absolute path")]
    NoConfigDir,
    #[error("cannot read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TOML or unknown field in {}", path.display())]
    Syntax { path: PathBuf },
    #[error("unknown theme in {}; choose terminal, dark, or light", path.display())]
    UnknownTheme { path: PathBuf },
    #[error("cannot write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl TuiSettings {
    fn path(env: &HostEnv) -> Option<PathBuf> {
        env.absolute_path("XDG_CONFIG_HOME")
            .or_else(|| env.absolute_path("HOME").map(|home| home.join(".config")))
            .map(|dir| dir.join("tgv/tui.toml"))
    }

    fn load(path: &Path) -> Result<Self, SettingsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(source) => {
                return Err(SettingsError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        // Do not echo arbitrary file contents in a TOML diagnostic.
        toml::from_str(&text).map_err(|_| SettingsError::Syntax {
            path: path.to_owned(),
        })
    }

    /// A flag overrides even an unreadable saved preference. No home/config
    /// directory is needed to use the default or a one-run override.
    pub(crate) fn select(
        flag: Option<ThemeName>,
        env: &HostEnv,
    ) -> Result<Option<ThemeName>, SettingsError> {
        if flag.is_some() {
            return Ok(flag);
        }
        let Some(path) = Self::path(env) else {
            return Ok(None);
        };
        match Self::load(&path)?.theme {
            Some(Stored::Known(theme)) => Ok(Some(theme)),
            Some(Stored::Unknown(_)) => Err(SettingsError::UnknownTheme { path }),
            None => Ok(None),
        }
    }

    /// Save atomically and owner-only; no daemon or terminal is needed.
    pub(crate) fn save(theme: ThemeName, env: &HostEnv) -> Result<PathBuf, SettingsError> {
        let path = Self::path(env).ok_or(SettingsError::NoConfigDir)?;
        let mut settings = Self::load(&path)?;
        settings.theme = Some(Stored::Known(theme));
        let write = |source| SettingsError::Write {
            path: path.clone(),
            source,
        };
        let dir = path.parent().unwrap_or(Path::new("."));
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(write)?;
        }
        let text =
            toml::to_string(&settings).map_err(|error| write(std::io::Error::other(error)))?;
        let mut staged = tempfile::NamedTempFile::new_in(dir).map_err(write)?;
        staged.write_all(text.as_bytes()).map_err(write)?;
        staged.as_file().sync_all().map_err(write)?;
        staged.persist(&path).map_err(|error| write(error.error))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::TuiSettings;
    use tgv_harness::host::HostEnv;
    use tgv_tui::ThemeName;

    #[test]
    fn local_preference_round_trips_and_flags_override_it() {
        let temp = tempfile::tempdir().unwrap();
        let env = HostEnv::from_pairs([("XDG_CONFIG_HOME", temp.path().to_str().unwrap())]);
        assert_eq!(TuiSettings::select(None, &env).unwrap(), None);
        for theme in [ThemeName::Dark, ThemeName::Light, ThemeName::Terminal] {
            let path = TuiSettings::save(theme, &env).unwrap();
            assert_eq!(path, temp.path().join("tgv/tui.toml"));
            assert_eq!(TuiSettings::select(None, &env).unwrap(), Some(theme));
            assert_eq!(
                TuiSettings::select(Some(ThemeName::Dark), &env).unwrap(),
                Some(ThemeName::Dark)
            );
        }
        std::fs::write(temp.path().join("tgv/tui.toml"), "theme = 'future'\n").unwrap();
        assert!(TuiSettings::select(None, &env).is_err());
        assert_eq!(
            TuiSettings::select(Some(ThemeName::Light), &env).unwrap(),
            Some(ThemeName::Light)
        );
        // An explicit save can replace an unknown theme, but not unknown settings.
        TuiSettings::save(ThemeName::Terminal, &env).unwrap();
        std::fs::write(temp.path().join("tgv/tui.toml"), "future = true\n").unwrap();
        assert!(TuiSettings::save(ThemeName::Dark, &env).is_err());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("tgv/tui.toml")).unwrap(),
            "future = true\n"
        );
    }

    #[test]
    fn config_location_uses_xdg_then_home_and_is_optional_until_saving() {
        let env = HostEnv::from_pairs([("HOME", "/home/fixture"), ("XDG_CONFIG_HOME", "relative")]);
        assert_eq!(
            TuiSettings::path(&env).unwrap().to_str().unwrap(),
            "/home/fixture/.config/tgv/tui.toml"
        );
        let none = HostEnv::default();
        assert_eq!(TuiSettings::select(None, &none).unwrap(), None);
        assert_eq!(
            TuiSettings::select(Some(ThemeName::Dark), &none).unwrap(),
            Some(ThemeName::Dark)
        );
        assert!(TuiSettings::save(ThemeName::Dark, &none).is_err());
    }
}
