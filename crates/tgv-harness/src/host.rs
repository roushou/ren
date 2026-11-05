//! The host environment, captured once in `main` and handed on as a value.
//!
//! Components never read the process environment themselves: they receive a
//! `HostEnv` and ask it for paths (`HOME`, the XDG directories, `PATH`) and
//! for operator API keys. Values may be secrets, so `Debug` shows names only.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::PathBuf,
};

/// Environment variables by name. Empty unless captured or given.
#[derive(Clone, Default)]
pub struct HostEnv {
    vars: BTreeMap<OsString, OsString>,
}

impl HostEnv {
    /// This process's environment, as it is now.
    #[expect(
        clippy::disallowed_methods,
        reason = "the host environment capability itself"
    )]
    pub fn capture() -> Self {
        Self {
            vars: std::env::vars_os().collect(),
        }
    }

    /// An environment with exactly these variables, for tests and dry runs.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self {
            vars: pairs
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        }
    }

    fn raw(&self, name: &str) -> Option<&OsStr> {
        self.vars
            .get(OsStr::new(name))
            .map(OsString::as_os_str)
            .filter(|value| !value.is_empty())
    }

    /// A non-blank Unicode value of `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.raw(name)
            .and_then(OsStr::to_str)
            .filter(|value| !value.trim().is_empty())
    }

    /// Whether `name` is set to anything non-empty. The value is not read.
    pub fn is_set(&self, name: &str) -> bool {
        self.raw(name).is_some()
    }

    /// `name` as an absolute path; relative values are ignored, as the XDG
    /// specification requires.
    pub fn absolute_path(&self, name: &str) -> Option<PathBuf> {
        self.raw(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    }

    /// The directories in `PATH`, in order.
    pub fn search_path(&self) -> Vec<PathBuf> {
        self.raw("PATH")
            .map(|path| std::env::split_paths(path).collect())
            .unwrap_or_default()
    }
}

impl std::fmt::Debug for HostEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.vars.keys()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::HostEnv;
    use std::path::PathBuf;

    #[test]
    fn values_never_show_in_debug_and_blank_is_absent() {
        let env = HostEnv::from_pairs([("KEY", "sk-SECRET"), ("BLANK", " "), ("EMPTY", "")]);
        assert_eq!(env.get("KEY"), Some("sk-SECRET"));
        assert_eq!(env.get("BLANK"), None);
        assert!(env.is_set("BLANK") && !env.is_set("EMPTY") && !env.is_set("MISSING"));
        let debug = format!("{env:?}");
        assert!(debug.contains("KEY") && !debug.contains("SECRET"));
    }

    #[test]
    fn paths_must_be_absolute() {
        let env = HostEnv::from_pairs([
            ("HOME", "/home/fixture"),
            ("XDG_STATE_HOME", "relative/state"),
            ("PATH", "/usr/bin:/bin"),
        ]);
        assert_eq!(
            env.absolute_path("HOME"),
            Some(PathBuf::from("/home/fixture"))
        );
        assert_eq!(env.absolute_path("XDG_STATE_HOME"), None);
        assert_eq!(
            env.search_path(),
            [PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
    }
}
