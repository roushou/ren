//! The canary's checks of a build, run against the canary on its copy of
//! production's history: `ren doctor` ([`CheckKind::Doctor`]), every read
//! of the read API ([`CheckKind::Reads`]), and every page of the UI in a
//! headless browser ([`CheckKind::Pages`]). Each becomes one [`Check`]; a
//! check that cannot run fails, so none is left out of the verdict.

use super::instance::CanaryInstance;
use crate::{
    builds::StoredBuild,
    daemon::{CheckStatus, DoctorReport},
    dev::shot::{Camera, Page},
    invocation::Invocation,
    progress::Progress,
};
use ren_harness::host::HostEnv;
use ren_release::{Check, CheckKind, Outcome, Verdict};
use std::net::SocketAddr;

/// Runs the checks against one canary.
pub struct Checks<'a> {
    pub env: &'a HostEnv,
    pub canary: &'a CanaryInstance,
    pub build: &'a StoredBuild,
}

/// The read API, swept: each listing, then each read of one item the
/// listings name. Together they make every `ReadRequest`.
pub struct ReadSweep {
    gateway: SocketAddr,
}

/// What a sweep found.
#[derive(Debug, Default, PartialEq, Eq)]
struct Swept {
    read: usize,
    failed: Vec<String>,
}

impl Checks<'_> {
    /// Every check, in the order they run.
    pub fn run(&self) -> Verdict {
        let checks = [CheckKind::Doctor, CheckKind::Reads, CheckKind::Pages]
            .into_iter()
            .map(|kind| {
                Progress(&format!("Checking the canary: {kind}")).show();
                let outcome = match kind {
                    CheckKind::Doctor => self.doctor(),
                    CheckKind::Reads => ReadSweep {
                        gateway: CanaryInstance::DAEMON,
                    }
                    .run(),
                    CheckKind::Pages => self.pages(),
                };
                match &outcome {
                    Outcome::Passed => println!("{kind}: passed"),
                    Outcome::Failed { detail } => println!("{kind}: failed: {detail}"),
                }
                Check::new(kind, outcome)
            })
            .collect();
        Verdict::new(checks)
    }

    /// `ren doctor` of the build: nothing failed, and the daemon runs the
    /// build its CLI is.
    fn doctor(&self) -> Outcome {
        let report = self
            .canary
            .ren(self.build)
            .args(["doctor", "--json"])
            .json("ren doctor --json", DoctorReport::parse);
        let report = match report {
            Ok(report) => report,
            Err(error) => {
                return Outcome::Failed {
                    detail: error.to_string(),
                };
            }
        };
        let mut failed: Vec<String> = report
            .checks()
            .iter()
            .filter(|check| match check.status() {
                CheckStatus::Ok | CheckStatus::Warning | CheckStatus::Skipped => false,
                CheckStatus::Failed | CheckStatus::Other(_) => true,
            })
            .map(|check| format!("{}: {}", check.name(), check.message()))
            .collect();
        match report.build() {
            Some(build) if *build.status() == CheckStatus::Ok => {}
            Some(build) => failed.push(format!("build: {}", build.message())),
            None => failed.push("build: not reported".to_owned()),
        }
        if failed.is_empty() {
            Outcome::Passed
        } else {
            Outcome::Failed {
                detail: failed.join("; "),
            }
        }
    }

    /// Every page of the canary's UI, captured.
    fn pages(&self) -> Outcome {
        let camera = Camera::new(
            self.env,
            CanaryInstance::UI,
            CanaryInstance::DAEMON,
            self.canary.shots.clone(),
        );
        let shots = camera.and_then(|camera| camera.shoot(&Page::ALL, (1440, 900)));
        match shots {
            Ok(shots) if shots.failed.is_empty() => Outcome::Passed,
            Ok(shots) => Outcome::Failed {
                detail: format!(
                    "did not load: {}",
                    shots
                        .failed
                        .iter()
                        .map(|page| page.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            },
            Err(error) => Outcome::Failed {
                detail: error.to_string(),
            },
        }
    }
}

impl ReadSweep {
    /// The listings, each read as it is.
    const LISTINGS: [&str; 11] = [
        "/api/calls?limit=5",
        "/api/recordings?limit=5",
        "/api/recordings?limit=5&shelf=archived",
        "/api/runs?limit=5",
        "/api/runs?limit=5&shelf=archived",
        "/api/usage",
        "/api/search?q=error",
        "/api/exposures",
        "/api/storage",
        "/api/metrics",
        "/api/release",
    ];

    fn run(&self) -> Outcome {
        let swept = self.sweep();
        println!("reads: {} answered", swept.read);
        if swept.failed.is_empty() {
            Outcome::Passed
        } else {
            Outcome::Failed {
                detail: swept.failed.join("; "),
            }
        }
    }

    fn sweep(&self) -> Swept {
        let mut swept = Swept::default();
        let mut read = |path: String| -> Option<serde_json::Value> {
            match self.get(&path) {
                Ok(body) => {
                    swept.read += 1;
                    Some(body)
                }
                Err(failure) => {
                    swept.failed.push(format!("GET {path} {failure}"));
                    None
                }
            }
        };
        let listings: Vec<Option<serde_json::Value>> = Self::LISTINGS
            .iter()
            .map(|path| read((*path).to_owned()))
            .collect();
        let first = |listing: &Option<serde_json::Value>, field: &str, key: &str| {
            listing
                .as_ref()
                .and_then(|body| body[field][0][key].as_str())
                .map(str::to_owned)
        };
        let (calls, recordings, runs) = (&listings[0], &listings[1], &listings[3]);
        if let Some(call) = first(calls, "calls", "id") {
            read(format!("/api/calls/{call}"));
            read(format!("/api/calls/{call}/activity"));
        }
        if let Some(recording) = first(recordings, "recordings", "id") {
            read(format!("/api/recordings/{recording}/calls?limit=5"));
            let turns = read(format!("/api/recordings/{recording}/turns?limit=5"));
            read(format!("/api/recordings/{recording}/report"));
            if let Some(turn) = first(&turns, "turns", "turn") {
                read(format!("/api/turns/{turn}"));
            }
        }
        if let Some(run) = first(runs, "runs", "id") {
            read(format!("/api/runs/{run}"));
            read(format!("/api/runs/{run}/conversation"));
        }
        swept
    }

    /// `GET path`'s JSON body, or what was wrong with its answer.
    fn get(&self, path: &str) -> Result<serde_json::Value, String> {
        let answer = Invocation::new("curl")
            .args([
                "--silent",
                "--max-time",
                "30",
                "--write-out",
                "\n%{http_code}",
            ])
            .arg(format!("http://{}{path}", self.gateway))
            .capture()
            .map_err(|error| error.to_string())?;
        let (body, status) = answer.rsplit_once('\n').unwrap_or(("", answer.as_str()));
        if status.trim() != "200" {
            return Err(format!("answered {}", status.trim()));
        }
        serde_json::from_str(body).map_err(|_| "answered no JSON".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::ReadSweep;
    use ren_store::read::ReadRequest;

    /// The sweep path that makes `request`: adding a read without sweeping
    /// it stops this from compiling.
    fn swept_by(request: &ReadRequest) -> &'static str {
        match request {
            ReadRequest::Calls(_) => "/api/calls?",
            ReadRequest::Call(_) => "/api/calls/{call}",
            ReadRequest::Recordings { .. } => "/api/recordings?",
            ReadRequest::RecordingCalls { .. } => "/api/recordings/{recording}/calls",
            ReadRequest::Turns { .. } => "/api/recordings/{recording}/turns",
            ReadRequest::Turn(_) => "/api/turns/{turn}",
            ReadRequest::CallActivity(_) => "/api/calls/{call}/activity",
            ReadRequest::Report(_) => "/api/recordings/{recording}/report",
            ReadRequest::Usage(_) => "/api/usage",
            ReadRequest::Search(_) => "/api/search?",
            ReadRequest::Exposures(_) => "/api/exposures",
            ReadRequest::Runs { .. } => "/api/runs?",
            ReadRequest::Run(_) => "/api/runs/{run}",
            ReadRequest::Conversation(_) => "/api/runs/{run}/conversation",
        }
    }

    #[test]
    fn every_read_is_swept() {
        let source = include_str!("checks.rs");
        let tests = source.find("#[cfg(test)]").unwrap();
        let (sweep, tests) = source.split_at(tests);
        let arms = &tests[tests.find("fn swept_by").unwrap()..tests.find("#[test]").unwrap()];
        let paths: Vec<&str> = arms
            .split("=> \"")
            .skip(1)
            .filter_map(|arm| arm.split('"').next())
            .collect();
        assert_eq!(paths.len(), 14, "one path per read");
        for path in paths {
            let listed = ReadSweep::LISTINGS
                .iter()
                .any(|listing| listing.starts_with(path));
            assert!(listed || sweep.contains(path), "{path} is not swept");
        }
        let run = ReadRequest::Run("00000000-0000-4000-8000-000000000001".parse().unwrap());
        assert_eq!(swept_by(&run), "/api/runs/{run}");
    }
}
