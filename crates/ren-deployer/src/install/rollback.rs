//! What an upgrade replaces, held aside until the new version passes its
//! check: then discarded, or put back with the unit it ran under.
//!
//! `install-vps` holds the daemon's binary and the UI's deployed copy this
//! way, so a failed check restores what ran before on its own and nothing
//! is left behind after an install.
use crate::file::{FileError, FileMode};
use crate::install::InstallError;
use crate::invocation::Invocation;
use crate::progress::Progress;
use std::fs;
use std::path::{Path, PathBuf};

/// An installation moved aside while its replacement is tried.
#[derive(Debug)]
#[must_use = "a held installation is discarded or put back"]
pub(crate) struct Held {
    /// Where it is installed, and where it waits meanwhile.
    installed: PathBuf,
    aside: PathBuf,
    /// The unit it ran under, as it was; none when there was no unit.
    unit: PathBuf,
    unit_text: Option<String>,
}

/// What an upgrade replaces.
#[derive(Debug, Clone)]
pub enum Part {
    /// The daemon, whose data directory holds the history backup.
    Daemon {
        data_dir: PathBuf,
    },
    Ui,
}

impl Part {
    fn name(&self) -> &'static str {
        match self {
            Self::Daemon { .. } => "the daemon",
            Self::Ui => "the UI",
        }
    }

    /// What to do when what ran before could not be put back either.
    pub(crate) fn advice(&self) -> String {
        match self {
            Self::Daemon { data_dir } => format!(
                "see `journalctl --user -u ren`. If the new build migrated the database, the old one cannot open it: install a fixed build, or restore the backup in {} with the old binary",
                data_dir.join("backup").display()
            ),
            Self::Ui => "see `journalctl --user -u ren-ui`".to_owned(),
        }
    }
}

impl std::fmt::Display for Part {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

/// How an upgrade ended once its check ran.
#[derive(Debug)]
pub(crate) enum Upgrade {
    /// The new version passed; what ran before is gone.
    Kept,
    /// The new version failed and what ran before runs again.
    RolledBack { failure: InstallError },
    /// The new version failed, and putting back what ran before failed too.
    Stuck {
        failure: InstallError,
        rollback: InstallError,
    },
    /// The new version failed, with nothing before it to put back.
    Failed { failure: InstallError },
}

impl Held {
    /// Moves `installed` to `aside`, remembering `unit` as it is. `None`
    /// when nothing is installed: a first install has nothing to put back.
    /// Anything left at `aside` by an install that was cut short is
    /// replaced.
    pub(crate) fn hold(
        installed: &Path,
        aside: &Path,
        unit: &Path,
    ) -> Result<Option<Self>, InstallError> {
        if !installed.exists() {
            return Ok(None);
        }
        Self::clear(aside)?;
        let unit_text = match fs::read_to_string(unit) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(FileError::at(unit)(error).into()),
        };
        fs::rename(installed, aside).map_err(FileError::at(aside))?;
        Ok(Some(Self {
            installed: installed.to_path_buf(),
            aside: aside.to_path_buf(),
            unit: unit.to_path_buf(),
            unit_text,
        }))
    }

    /// The new version passed: what ran before is removed.
    pub(crate) fn discard(self) -> Result<(), InstallError> {
        Self::clear(&self.aside)
    }

    /// Puts what ran before back in place, with its unit, replacing the new
    /// version. The service is the caller's to stop first and start after.
    pub(crate) fn put_back(self) -> Result<(), InstallError> {
        Self::clear(&self.installed)?;
        fs::rename(&self.aside, &self.installed).map_err(FileError::at(&self.installed))?;
        match &self.unit_text {
            Some(text) => {
                FileMode::Readable.write(&self.unit, text.as_bytes())?;
            }
            None => Self::clear(&self.unit)?,
        }
        Ok(())
    }

    /// Removes `path`, a file or a folder, if it is there.
    fn clear(path: &Path) -> Result<(), InstallError> {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.is_dir() => Ok(fs::remove_dir_all(path).map_err(FileError::at(path))?),
            Ok(_) => Ok(fs::remove_file(path).map_err(FileError::at(path))?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(FileError::at(path)(error).into()),
        }
    }
}

impl Upgrade {
    /// Tries the new version with `install` (which replaces what `held`
    /// holds and checks the result). On success what ran before is
    /// discarded; on failure it is put back under `service` and checked
    /// with `recheck`.
    pub(crate) fn attempt(
        held: Option<Held>,
        service: &str,
        install: impl FnOnce() -> Result<(), InstallError>,
        recheck: impl FnOnce() -> Result<(), InstallError>,
    ) -> Self {
        let failure = match install() {
            Ok(()) => {
                if let Some(Err(error)) = held.map(Held::discard) {
                    // The new version runs; only the old copy stays behind.
                    eprintln!("warning: could not remove what ran before: {error}");
                }
                return Self::Kept;
            }
            Err(failure) => failure,
        };
        let Some(held) = held else {
            return Self::Failed { failure };
        };
        Progress(&format!(
            "The new version failed; putting back what ran before ({failure})"
        ))
        .show();
        let restored = Self::restart(service, held).and_then(|()| recheck());
        match restored {
            Ok(()) => Self::RolledBack { failure },
            Err(rollback) => Self::Stuck { failure, rollback },
        }
    }

    /// The upgrade's end as a result for `part`: an error unless kept.
    pub(crate) fn finish(self, part: Part) -> Result<(), InstallError> {
        match self {
            Self::Kept => Ok(()),
            Self::Failed { failure } => Err(failure),
            Self::RolledBack { failure } => Err(InstallError::RolledBack {
                part,
                failure: Box::new(failure),
            }),
            Self::Stuck { failure, rollback } => Err(InstallError::RollbackFailed {
                part,
                failure: Box::new(failure),
                rollback: Box::new(rollback),
            }),
        }
    }

    /// Stops `service`, puts `held` back, reloads the units, and starts it.
    fn restart(service: &str, held: Held) -> Result<(), InstallError> {
        Invocation::new("systemctl")
            .args(["--user", "stop", service])
            .run()?;
        held.put_back()?;
        Invocation::new("systemctl")
            .args(["--user", "daemon-reload"])
            .run()?;
        Invocation::new("systemctl")
            .args(["--user", "reset-failed", service])
            .run()?;
        Invocation::new("systemctl")
            .args(["--user", "start", service])
            .run()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("ren-ui");
        let aside = dir.path().join("ren-ui.rollback");
        let unit = dir.path().join("ren-ui.service");
        (dir, installed, aside, unit)
    }

    #[test]
    fn nothing_installed_is_nothing_to_hold() {
        let (_dir, installed, aside, unit) = place();
        assert!(Held::hold(&installed, &aside, &unit).unwrap().is_none());
        assert!(!aside.exists());
    }

    #[test]
    fn a_kept_upgrade_leaves_nothing_aside() {
        let (_dir, installed, aside, unit) = place();
        fs::create_dir(&installed).unwrap();
        fs::write(installed.join("old"), "old").unwrap();
        let held = Held::hold(&installed, &aside, &unit).unwrap().unwrap();
        assert!(!installed.exists());
        assert!(aside.join("old").exists());
        // The new version goes in, and passes.
        fs::create_dir(&installed).unwrap();
        fs::write(installed.join("new"), "new").unwrap();
        held.discard().unwrap();
        assert!(installed.join("new").exists());
        assert!(!aside.exists());
    }

    #[test]
    fn put_back_restores_the_old_version_and_its_unit() {
        let (_dir, installed, aside, unit) = place();
        fs::create_dir(&installed).unwrap();
        fs::write(installed.join("old"), "old").unwrap();
        fs::write(&unit, "ExecStart=node build\n").unwrap();
        let held = Held::hold(&installed, &aside, &unit).unwrap().unwrap();
        fs::create_dir(&installed).unwrap();
        fs::write(installed.join("new"), "new").unwrap();
        fs::write(&unit, "ExecStart=node apps/web/build\n").unwrap();
        held.put_back().unwrap();
        assert!(installed.join("old").exists());
        assert!(!installed.join("new").exists());
        assert!(!aside.exists());
        assert_eq!(fs::read_to_string(&unit).unwrap(), "ExecStart=node build\n");
    }

    #[test]
    fn put_back_removes_a_unit_that_did_not_exist() {
        let (_dir, installed, aside, unit) = place();
        fs::write(&installed, "old binary").unwrap();
        let held = Held::hold(&installed, &aside, &unit).unwrap().unwrap();
        fs::write(&installed, "new binary").unwrap();
        fs::write(&unit, "new unit").unwrap();
        held.put_back().unwrap();
        assert_eq!(fs::read_to_string(&installed).unwrap(), "old binary");
        assert!(!unit.exists());
    }

    #[test]
    fn a_leftover_from_a_cut_short_install_is_replaced() {
        let (_dir, installed, aside, unit) = place();
        fs::create_dir(&aside).unwrap();
        fs::write(aside.join("stale"), "stale").unwrap();
        fs::create_dir(&installed).unwrap();
        let held = Held::hold(&installed, &aside, &unit).unwrap().unwrap();
        assert!(!aside.join("stale").exists());
        held.discard().unwrap();
    }

    #[test]
    fn a_failure_with_nothing_held_is_reported_as_it_was() {
        let upgrade = Upgrade::attempt(
            None,
            "unused.service",
            || Err(InstallError::Doctor),
            || Ok(()),
        );
        assert!(matches!(
            upgrade,
            Upgrade::Failed {
                failure: InstallError::Doctor
            }
        ));
    }

    #[test]
    fn a_pass_discards_what_was_held() {
        let (_dir, installed, aside, unit) = place();
        fs::create_dir(&installed).unwrap();
        let held = Held::hold(&installed, &aside, &unit).unwrap();
        let upgrade = Upgrade::attempt(
            held,
            "unused.service",
            || Ok(fs::create_dir(&installed).map_err(FileError::at(&installed))?),
            || Ok(()),
        );
        assert!(matches!(upgrade, Upgrade::Kept));
        assert!(installed.exists());
        assert!(!aside.exists());
    }
}
