//! Work items in the API: `/api/work` and `/api/work/{id}`, and the order a
//! client sends to queue one (`POST /control/v1/work`), which is read back
//! as it was sent.

use ren_git::Commit;
use ren_git::SourceRevision;
use ren_protocol::label::Stored;
use ren_recorder::{
    id::{ProjectId, SessionId, WorkItemId},
    time::UnixMillis,
    work::{
        Evidence, MarkedPage, PageTheme, PreviewUrl, WorkMark, WorkOrder, WorkPreview,
        WorkStateKind,
    },
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Work items, most recently changed first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkItems {
    pub work: Vec<WorkItemView>,
}

/// One work item: what was marked, and where it stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkItemView {
    #[ts(as = "String")]
    pub id: WorkItemId,
    #[ts(as = "u64")]
    pub created_at: UnixMillis,
    #[ts(as = "u64")]
    pub updated_at: UnixMillis,
    /// Its first note's first line.
    pub title: String,
    pub order: WorkOrderBody,
    /// `queued`, `sent`, `landed`, `shipped`, or `dismissed`.
    #[ts(as = "String")]
    pub state: Stored<WorkStateKind>,
    /// The project it is for; `None` when none was named, or from before
    /// projects.
    #[ts(as = "Option<String>")]
    pub project: Option<ProjectId>,
    /// The session that took it up, once sent.
    #[ts(as = "Option<String>")]
    pub session: Option<SessionId>,
    /// The commit its change landed on `main` as.
    #[ts(as = "Option<String>")]
    pub landed: Option<Commit>,
    /// The build production ran with its change.
    #[ts(as = "Option<String>")]
    pub shipped: Option<SourceRevision>,
    /// A preview of its change, while one is up.
    pub preview: Option<WorkPreviewBody>,
}

/// A preview of a work item's change: its session's branch on a copy of
/// production's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct WorkPreviewBody {
    /// The preview's UI, an origin such as `https://ren.example.ts.net:20201`.
    #[ts(as = "String")]
    pub ui: PreviewUrl,
    /// The build it runs: `<commit>`, `<commit>-dirty`, or `unknown`.
    #[ts(as = "String")]
    pub revision: SourceRevision,
}

/// What was marked: the page, its marks, and what else was on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct WorkOrderBody {
    pub page: MarkedPageBody,
    pub marks: Vec<WorkMarkBody>,
    #[serde(default)]
    pub evidence: EvidenceBody,
}

/// The page the marks were made on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct MarkedPageBody {
    /// Its path and query.
    pub path: String,
    /// The UI's route id, when it has one.
    pub route: Option<String>,
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// `light` or `dark`.
    #[ts(as = "String")]
    pub theme: PageTheme,
    /// What the daemon serving the page was built from: `<commit>`,
    /// `<commit>-dirty`, or `unknown`.
    #[ts(as = "Option<String>")]
    pub revision: Option<SourceRevision>,
}

/// One mark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct WorkMarkBody {
    pub selector: String,
    pub text: String,
    pub source: Option<String>,
    pub width: u32,
    pub height: u32,
    pub note: String,
}

/// The errors the page logged and the reads behind it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct EvidenceBody {
    pub console: Vec<String>,
    pub reads: Vec<String>,
}

impl From<WorkOrder> for WorkOrderBody {
    fn from(order: WorkOrder) -> Self {
        let WorkOrder {
            page,
            marks,
            evidence,
        } = order;
        let MarkedPage {
            path,
            route,
            title,
            width,
            height,
            theme,
            revision,
        } = page;
        let Evidence { console, reads } = evidence;
        Self {
            page: MarkedPageBody {
                path,
                route,
                title,
                width,
                height,
                theme,
                revision,
            },
            marks: marks
                .into_iter()
                .map(
                    |WorkMark {
                         selector,
                         text,
                         source,
                         width,
                         height,
                         note,
                     }| WorkMarkBody {
                        selector,
                        text,
                        source,
                        width,
                        height,
                        note,
                    },
                )
                .collect(),
            evidence: EvidenceBody { console, reads },
        }
    }
}

impl From<WorkOrderBody> for WorkOrder {
    fn from(body: WorkOrderBody) -> Self {
        let WorkOrderBody {
            page,
            marks,
            evidence,
        } = body;
        let MarkedPageBody {
            path,
            route,
            title,
            width,
            height,
            theme,
            revision,
        } = page;
        let EvidenceBody { console, reads } = evidence;
        Self {
            page: MarkedPage {
                path,
                route,
                title,
                width,
                height,
                theme,
                revision,
            },
            marks: marks
                .into_iter()
                .map(
                    |WorkMarkBody {
                         selector,
                         text,
                         source,
                         width,
                         height,
                         note,
                     }| WorkMark {
                        selector,
                        text,
                        source,
                        width,
                        height,
                        note,
                    },
                )
                .collect(),
            evidence: Evidence { console, reads },
        }
    }
}

impl From<ren_store::view::WorkItemView> for WorkItemView {
    fn from(item: ren_store::view::WorkItemView) -> Self {
        let ren_store::view::WorkItemView {
            id,
            created_at,
            updated_at,
            title,
            order,
            state,
            project,
            session,
            landed,
            shipped,
            preview,
        } = item;
        Self {
            id,
            created_at,
            updated_at,
            title,
            order: order.into(),
            state,
            project,
            session,
            landed,
            shipped,
            preview: preview.map(|WorkPreview { ui, revision }| WorkPreviewBody { ui, revision }),
        }
    }
}
