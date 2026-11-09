//! `cargo xtask dev shot`: screenshots of the dev UI's pages with a headless
//! Chromium, for checking a UI change without a browser. Each page loads
//! for a few seconds, then is captured; the live feed never holds it open.

use super::DevError;
use crate::{
    install::InstallError,
    invocation::{Echo, Invocation},
};
use clap::ValueEnum;
use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tgv_harness::host::HostEnv;

/// A page of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Page {
    Overview,
    Live,
    Traffic,
    Sessions,
    /// The newest session, with its turns.
    Session,
    Usage,
    Search,
}

impl Page {
    pub const ALL: [Self; 7] = [
        Self::Overview,
        Self::Live,
        Self::Traffic,
        Self::Sessions,
        Self::Session,
        Self::Usage,
        Self::Search,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Live => "live",
            Self::Traffic => "traffic",
            Self::Sessions => "sessions",
            Self::Session => "session",
            Self::Usage => "usage",
            Self::Search => "search",
        }
    }

    /// The page's path; `newest` is the newest session, for `Session`.
    fn path(self, newest: Option<&str>) -> Option<String> {
        match self {
            Self::Overview => Some("/".into()),
            Self::Live => Some("/live".into()),
            Self::Traffic => Some("/traffic".into()),
            Self::Sessions => Some("/sessions".into()),
            Self::Session => newest.map(|id| format!("/sessions/{id}")),
            Self::Usage => Some("/usage".into()),
            Self::Search => Some("/search?q=retry".into()),
        }
    }
}

/// Takes screenshots of one UI.
pub struct Camera {
    chrome: PathBuf,
    ui: SocketAddr,
    gateway: SocketAddr,
    out: PathBuf,
}

impl Camera {
    /// How long a page loads before it is captured, in milliseconds.
    const LOAD_MS: &'static str = "4000";

    /// A headless Chromium: `TGV_CHROME`, else `chrome-headless-shell` or
    /// `chromium` on PATH, else Playwright's downloaded headless shell.
    pub fn new(
        env: &HostEnv,
        ui: SocketAddr,
        gateway: SocketAddr,
        out: PathBuf,
    ) -> Result<Self, DevError> {
        let chrome = env
            .absolute_path("TGV_CHROME")
            .or_else(|| Self::on_path(env))
            .or_else(|| Self::playwright(env))
            .ok_or(DevError::NoChrome)?;
        Ok(Self {
            chrome,
            ui,
            gateway,
            out,
        })
    }

    fn on_path(env: &HostEnv) -> Option<PathBuf> {
        let path = env.get("PATH")?;
        ["chrome-headless-shell", "chromium", "chromium-browser"]
            .iter()
            .flat_map(|name| path.split(':').map(move |dir| Path::new(dir).join(name)))
            .find(|candidate| candidate.is_absolute() && candidate.is_file())
    }

    fn playwright(env: &HostEnv) -> Option<PathBuf> {
        let cache = env.absolute_path("HOME")?.join(".cache/ms-playwright");
        let mut shells: Vec<PathBuf> = fs::read_dir(&cache)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|dir| {
                dir.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("chromium_headless_shell-"))
            })
            .filter_map(|dir| fs::read_dir(dir).ok())
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("chrome-headless-shell"))
            .filter(|binary| binary.is_file())
            .collect();
        shells.sort();
        shells.pop()
    }

    /// The newest session's id, from the dev gateway's API.
    fn newest_session(&self) -> Option<String> {
        let answer = Invocation::new("curl")
            .args(["--silent", "--fail", "--max-time", "5"])
            .arg(format!("http://{}/api/sessions?limit=1", self.gateway))
            .capture()
            .ok()?;
        let answer: serde_json::Value = serde_json::from_str(&answer).ok()?;
        answer["sessions"][0]["id"].as_str().map(str::to_owned)
    }

    /// Capture `pages` at `width`×`height`; returns the files written.
    pub fn shoot(
        &self,
        pages: &[Page],
        (width, height): (u16, u16),
    ) -> Result<Vec<PathBuf>, DevError> {
        fs::create_dir_all(&self.out).map_err(InstallError::file(&self.out))?;
        let newest = self.newest_session();
        let mut written = Vec::new();
        for page in pages {
            let Some(path) = page.path(newest.as_deref()) else {
                println!("skipped {}: no session yet", page.name());
                continue;
            };
            let file = self.out.join(format!("{}.png", page.name()));
            // `timeout` bounds a page that would never settle.
            let taken = Invocation::new("timeout")
                .arg("60")
                .arg(&self.chrome)
                .args([
                    "--no-sandbox",
                    "--hide-scrollbars",
                    &format!("--window-size={width},{height}"),
                    &format!("--timeout={}", Self::LOAD_MS),
                ])
                .arg(format!("--screenshot={}", file.display()))
                .arg(format!("http://{}{path}", self.ui))
                .succeeds(Echo::Hide)?;
            if taken && file.is_file() {
                println!("{}", file.display());
                written.push(file);
            } else {
                println!("failed {}", page.name());
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::Page;

    #[test]
    fn every_page_has_a_path_and_the_session_needs_one() {
        for page in Page::ALL {
            match page {
                Page::Session => {
                    assert_eq!(page.path(None), None);
                    assert_eq!(page.path(Some("s1")).as_deref(), Some("/sessions/s1"));
                }
                Page::Overview
                | Page::Live
                | Page::Traffic
                | Page::Sessions
                | Page::Usage
                | Page::Search => assert!(page.path(None).is_some_and(|p| p.starts_with('/'))),
            }
        }
    }
}
