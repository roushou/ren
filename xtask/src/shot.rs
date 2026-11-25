//! `cargo xtask dev shot`: screenshots of the dev UI's pages with a headless
//! Chromium, for checking a UI change without a browser, and the canary's
//! page check. Each page is first asked for, and fails unless the UI answers
//! it (after its redirects) with a success: an error page is a failure, not
//! a capture. Then it loads for a few seconds and is captured; the live feed
//! never holds it open. The pages under a project are those of a project in
//! the history being shown: Ren's, else the first.

use crate::file::FileError;
use crate::invocation::{Echo, Invocation, InvocationError};
use clap::ValueEnum;
use ren_harness::host::HostEnv;
use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

/// A page of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Page {
    /// The desk: what needs the operator, what is under way, what happened.
    Desk,
    /// Every project at a glance.
    Projects,
    Release,
    Overview,
    Live,
    Traffic,
    Recordings,
    /// The newest recording, with its turns.
    Recording,
    Usage,
    Search,
    /// The newest run's session, with its conversation.
    Sessions,
    /// How the machine stands: recording, packages, GitHub.
    System,
    /// The packages Ren installs for its agents.
    Packages,
}

/// The project, and its newest run and the newest recording, which some
/// pages show.
#[derive(Debug, Default)]
struct Newest {
    recording: Option<String>,
    /// The session of the project's newest run.
    session: Option<String>,
    /// The project the pages under one show, by name.
    project: Option<String>,
}

impl Page {
    pub const ALL: [Self; 13] = [
        Self::Desk,
        Self::Projects,
        Self::Release,
        Self::Overview,
        Self::Live,
        Self::Traffic,
        Self::Recordings,
        Self::Recording,
        Self::Usage,
        Self::Search,
        Self::Sessions,
        Self::System,
        Self::Packages,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Desk => "desk",
            Self::Projects => "projects",
            Self::Release => "release",
            Self::Overview => "overview",
            Self::Live => "live",
            Self::Traffic => "traffic",
            Self::Recordings => "recordings",
            Self::Recording => "recording",
            Self::Usage => "usage",
            Self::Search => "search",
            Self::Sessions => "sessions",
            Self::System => "system",
            Self::Packages => "packages",
        }
    }

    /// The page's path: the newest recording for `Recording`; the project's
    /// pages for `Release` and `Sessions` (its newest run's session open, if
    /// any). None when there is no such recording or
    /// project, except Sessions, which is then Scratch.
    fn path(self, newest: &Newest) -> Option<String> {
        let project = newest
            .project
            .as_deref()
            .map(|name| format!("/projects/{name}"));
        match self {
            Self::Desk => Some("/".into()),
            Self::Projects => Some("/projects".into()),
            Self::Release => project.map(|base| format!("{base}/release")),
            Self::Overview => Some("/monitor".into()),
            Self::Live => Some("/monitor/live".into()),
            Self::Traffic => Some("/monitor/traffic".into()),
            Self::Recordings => Some("/monitor/sessions".into()),
            Self::Recording => newest
                .recording
                .as_ref()
                .map(|id| format!("/monitor/sessions/{id}")),
            Self::Usage => Some("/monitor/usage".into()),
            Self::Search => Some("/monitor/search?q=retry".into()),
            Self::Sessions => Some(match (project, newest.session.as_deref()) {
                (Some(base), Some(session)) => format!("{base}/sessions/{session}"),
                (Some(base), None) => format!("{base}/sessions"),
                (None, Some(_) | None) => "/scratch".into(),
            }),
            Self::System => Some("/system".into()),
            Self::Packages => Some("/system/packages".into()),
        }
    }
}

/// What one round of screenshots took.
#[derive(Debug, Default)]
pub struct Shots {
    pub written: Vec<PathBuf>,
    /// Pages the UI did not answer with a success, or that did not capture.
    pub failed: Vec<(Page, PageFailure)>,
    /// Pages with nothing to show yet (no recording, or no project).
    pub skipped: Vec<Page>,
}

/// Why a page failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PageFailure {
    /// The UI did not answer it.
    #[error("no answer")]
    Unanswered,
    /// The UI answered it with an error: its error page.
    #[error("answered {0}")]
    Status(u16),
    /// It answered, but did not load in time or capture.
    #[error("did not capture")]
    NotCaptured,
}

/// Takes screenshots of one UI.
/// Why screenshots could not be taken.
#[derive(Debug, thiserror::Error)]
pub enum ShotError {
    #[error("no headless Chromium found: set REN_CHROME, or install chrome-headless-shell")]
    NoChrome,
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
}

pub struct Camera {
    chrome: PathBuf,
    ui: SocketAddr,
    gateway: SocketAddr,
    out: PathBuf,
}

impl Camera {
    /// How long a page loads before it is captured, in milliseconds.
    const LOAD_MS: &'static str = "4000";

    /// A headless Chromium: `REN_CHROME`, else `chrome-headless-shell` or
    /// `chromium` on PATH, else Playwright's downloaded headless shell.
    pub fn new(
        env: &HostEnv,
        ui: SocketAddr,
        gateway: SocketAddr,
        out: PathBuf,
    ) -> Result<Self, ShotError> {
        let chrome = env
            .absolute_path("REN_CHROME")
            .or_else(|| Self::on_path(env))
            .or_else(|| Self::playwright(env))
            .ok_or(ShotError::NoChrome)?;
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

    /// A read of the gateway's API, at `path` (with its query).
    fn read(&self, path: &str) -> Option<serde_json::Value> {
        let answer = Invocation::new("curl")
            .args(["--silent", "--fail", "--max-time", "5"])
            .arg(format!("http://{}{path}", self.gateway))
            .capture()
            .ok()?;
        serde_json::from_str(&answer).ok()
    }

    /// The newest item's id in a listing of the gateway's API, such as
    /// `recordings` or `runs`, with `query` after its limit.
    fn newest(&self, listing: &str, query: &str) -> Option<String> {
        self.newest_field(listing, query, "id")
    }

    /// A field of the newest item in a listing, as [`Self::newest`] reads it.
    fn newest_field(&self, listing: &str, query: &str, field: &str) -> Option<String> {
        let answer = self.read(&format!("/api/{listing}?limit=1{query}"))?;
        answer[listing][0][field].as_str().map(str::to_owned)
    }

    /// The project the pages under one show: Ren's, else the first not
    /// archived, by name.
    fn project(&self) -> Option<String> {
        let answer = self.read("/api/projects")?;
        let projects = answer["projects"].as_array()?;
        let current = projects
            .iter()
            .filter(|project| project["archived_at"].is_null());
        current
            .clone()
            .find(|project| project["factory"] == "ren")
            .or_else(|| current.clone().next())
            .and_then(|project| project["name"].as_str())
            .map(str::to_owned)
    }

    /// The status the UI answers `path` with, after its redirects; None when
    /// it does not answer.
    fn status(&self, path: &str) -> Option<u16> {
        Invocation::new("curl")
            .args(["--silent", "--location", "--max-time", "30"])
            .args(["--output", "/dev/null", "--write-out", "%{http_code}"])
            .arg(format!("http://{}{path}", self.ui))
            .capture()
            .ok()?
            .trim()
            .parse()
            .ok()
            .filter(|code| *code != 0)
    }

    /// Capture `pages` at `width`×`height`.
    pub fn shoot(&self, pages: &[Page], (width, height): (u16, u16)) -> Result<Shots, ShotError> {
        fs::create_dir_all(&self.out).map_err(FileError::at(&self.out))?;
        let project = self.project();
        let session = project.as_deref().and_then(|name| {
            self.newest_field("runs", &format!("&project={name}"), "operator_session")
        });
        let newest = Newest {
            recording: self.newest("recordings", ""),
            session,
            project,
        };
        let mut shots = Shots::default();
        for page in pages {
            let Some(path) = page.path(&newest) else {
                println!("skipped {}: nothing to show yet", page.name());
                shots.skipped.push(*page);
                continue;
            };
            match self.status(&path) {
                Some(code) if (200..300).contains(&code) => {}
                answered => {
                    let failure = answered.map_or(PageFailure::Unanswered, PageFailure::Status);
                    println!("failed {}: {path} {failure}", page.name());
                    shots.failed.push((*page, failure));
                    continue;
                }
            }
            let file = self.out.join(format!("{}.png", page.name()));
            // An earlier round's file would read as this round's capture.
            if file.exists() {
                fs::remove_file(&file).map_err(FileError::at(&file))?;
            }
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
                shots.written.push(file);
            } else {
                println!("failed {}: {path} did not capture", page.name());
                shots.failed.push((*page, PageFailure::NotCaptured));
            }
        }
        Ok(shots)
    }
}

#[cfg(test)]
mod tests {
    use super::{Newest, Page};

    #[test]
    fn a_projects_pages_are_the_historys_own_project() {
        let none = Newest::default();
        let some = Newest {
            recording: Some("s1".into()),
            session: Some("r1".into()),
            project: Some("ren".into()),
        };
        let no_run = Newest {
            project: Some("ren".into()),
            ..Newest::default()
        };
        for page in Page::ALL {
            match page {
                Page::Recording => {
                    assert_eq!(page.path(&none), None);
                    assert_eq!(page.path(&some).as_deref(), Some("/monitor/sessions/s1"));
                }
                Page::Release => {
                    assert_eq!(page.path(&none), None);
                    assert_eq!(page.path(&some).as_deref(), Some("/projects/ren/release"));
                }
                Page::Sessions => {
                    assert_eq!(page.path(&none).as_deref(), Some("/scratch"));
                    assert_eq!(
                        page.path(&some).as_deref(),
                        Some("/projects/ren/sessions/r1")
                    );
                    assert_eq!(
                        page.path(&no_run).as_deref(),
                        Some("/projects/ren/sessions")
                    );
                }
                Page::Desk
                | Page::Projects
                | Page::Overview
                | Page::Live
                | Page::Traffic
                | Page::Recordings
                | Page::Usage
                | Page::Search
                | Page::System
                | Page::Packages => assert!(page.path(&none).is_some_and(|p| p.starts_with('/'))),
            }
        }
    }
}
