//! Closing work items from what the factory sees: a sent item lands when a
//! commit on `main` names it (`Work-Item: <id>`, a [`Landing`]), and a
//! landed item ships when production runs a clean build whose history holds
//! that commit. Each change goes through production's own control API, with
//! its control token, as an operator's would; production's daemon checks it
//! against the item's state.

use super::{CanaryError, Factory, instance::ProductionPlace};
use crate::{file::FileError, invocation::Invocation, progress::Progress};
use ren_gateway::ControlToken;
use ren_git::CommitId;
use ren_protocol::label::Stored;
use ren_recorder::{
    id::WorkItemId,
    work::{Landing, WorkStateKind},
};
use ren_release::{Commit, Revision, Tree};
use serde::Deserialize;
use std::{fs, io::Write as _};

/// Production's work items, as far as closing them needs.
pub struct WorkLedger<'a> {
    production: &'a ProductionPlace,
}

/// One work item, as `/api/work` lists it.
#[derive(Debug, Deserialize)]
struct Listed {
    id: WorkItemId,
    state: Stored<WorkStateKind>,
    landed: Option<Commit>,
}

#[derive(Deserialize)]
struct Listing {
    work: Vec<Listed>,
}

/// What a pass did to the work items.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Closed {
    pub landed: usize,
    pub shipped: usize,
}

impl<'a> WorkLedger<'a> {
    /// How far back `main`'s history is read for trailers.
    const HISTORY: u32 = 500;

    pub fn new(production: &'a ProductionPlace) -> Self {
        Self { production }
    }

    /// Lands the sent items `main`'s recent commits name, then ships the
    /// landed items production's build holds. A production whose daemon
    /// keeps no work items yet is left alone.
    pub fn close(&self, factory: &Factory, main: &CommitId) -> Result<Closed, CanaryError> {
        let Some(items) = self.items()? else {
            Progress("Production keeps no work items yet; none to close").show();
            return Ok(Closed::default());
        };
        let mut closed = Closed::default();
        let sent: Vec<WorkItemId> = items
            .iter()
            .filter(|item| item.state == Stored::Known(WorkStateKind::Sent))
            .map(|item| item.id)
            .collect();
        // Oldest first: an item named twice lands as the first commit.
        let mut landed: Vec<(WorkItemId, CommitId)> = items
            .iter()
            .filter(|item| item.state == Stored::Known(WorkStateKind::Landed))
            .filter_map(|item| Some((item.id, Factory::id(item.landed.as_ref()?).ok()?)))
            .collect();
        if !sent.is_empty() {
            let mut log = factory.git.log(&factory.repo, main, Self::HISTORY)?;
            log.reverse();
            for (commit, message) in log {
                let Ok(release_commit) = Factory::commit(&commit) else {
                    continue;
                };
                for item in Landing::read(release_commit, &message).items {
                    if sent.contains(&item) && !landed.iter().any(|(id, _)| *id == item) {
                        self.move_on(
                            item,
                            "land",
                            &serde_json::json!({ "commit": commit.as_str() }),
                        )?;
                        Progress(&format!(
                            "Work item {item} landed as {}",
                            commit.abbreviated(12)
                        ))
                        .show();
                        landed.push((item, commit.clone()));
                        closed.landed += 1;
                    }
                }
            }
        }
        let running = match self.production.revision()? {
            Revision::Commit {
                commit,
                tree: Tree::Clean,
            } => Factory::id(&commit).ok(),
            Revision::Commit {
                tree: Tree::Modified,
                ..
            }
            | Revision::Unknown => None,
        };
        if let Some(running) = running {
            for (item, commit) in &landed {
                if factory.git.contains(&factory.repo, &running, commit)? {
                    let revision = running.as_str();
                    self.move_on(*item, "ship", &serde_json::json!({ "revision": revision }))?;
                    Progress(&format!(
                        "Work item {item} shipped in {}",
                        running.abbreviated(12)
                    ))
                    .show();
                    closed.shipped += 1;
                }
            }
        }
        Ok(closed)
    }

    /// The items under way, or none when production's daemon does not
    /// serve work items.
    fn items(&self) -> Result<Option<Vec<Listed>>, CanaryError> {
        let answer = Invocation::new("curl")
            .args([
                "--silent",
                "--max-time",
                "20",
                "--write-out",
                "\n%{http_code}",
            ])
            .arg(format!(
                "http://{}/api/work?limit=1000",
                self.production.listen()
            ))
            .capture()?;
        let (body, status) = answer.rsplit_once('\n').unwrap_or(("", answer.as_str()));
        match status.trim() {
            "200" => serde_json::from_str::<Listing>(body)
                .map(|listing| Some(listing.work))
                .map_err(|_| CanaryError::WorkAnswer("/api/work".to_owned())),
            "404" => Ok(None),
            other => Err(CanaryError::WorkAnswer(format!(
                "/api/work answered {other}"
            ))),
        }
    }

    /// `POST /control/v1/work/{item}/{change}` on production.
    fn move_on(
        &self,
        item: WorkItemId,
        change: &str,
        body: &serde_json::Value,
    ) -> Result<(), CanaryError> {
        let path = self.production.data_dir.join(ControlToken::FILE);
        let token = fs::read_to_string(&path)
            .ok()
            .and_then(|text| ControlToken::parse(&text))
            .ok_or_else(|| {
                CanaryError::WorkAnswer(format!("no control token in {}", path.display()))
            })?;
        // The token goes in a private file curl reads, never on its command
        // line, where any process could read it.
        let mut header = tempfile::Builder::new()
            .prefix(".ren-work.")
            .tempfile()
            .map_err(FileError::at(&path))?;
        writeln!(header, "authorization: Bearer {}", token.expose())
            .map_err(FileError::at(&path))?;
        let answer = Invocation::new("curl")
            .args([
                "--silent",
                "--max-time",
                "20",
                "--write-out",
                "\n%{http_code}",
            ])
            .args(["-X", "POST", "-H", "content-type: application/json"])
            .arg("-H")
            .arg(format!("@{}", header.path().display()))
            .arg("--data")
            .arg(body.to_string())
            .arg(format!(
                "http://{}/control/v1/work/{item}/{change}",
                self.production.listen()
            ))
            .capture()?;
        let status = answer
            .rsplit_once('\n')
            .map_or(answer.as_str(), |(_, status)| status);
        match status.trim() {
            "202" => Ok(()),
            other => Err(CanaryError::WorkAnswer(format!(
                "{change} of work item {item} answered {other}"
            ))),
        }
    }
}
