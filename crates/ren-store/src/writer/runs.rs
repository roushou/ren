//! How a batch's operator, session, run, and history records are written.

use super::MetadataTx;
use crate::StoreError;
use crate::millis::SqlMillis;
use ren_harness::{
    attachment::AttachmentKind,
    drive::{DriveEvent, DriveEventKind, ItemRead},
    models::ModelId,
};
use ren_protocol::label::Labeled;
use ren_recorder::Observation;
use ren_recorder::batch::{
    HistoryChange, HistoryEdit, HistorySubject, ObservationKind, OperatorAction, RunEntry, RunFact,
    RunRecord, SessionFact, SessionRecord,
};
use ren_recorder::{
    capture::{CallState, ExitParts},
    id::ChannelId,
};
use sqlx::{AssertSqlSafe, Row};

impl MetadataTx<'_> {
    pub(super) async fn operator(
        &mut self,
        action: &OperatorAction,
    ) -> Result<Observation, StoreError> {
        sqlx::query(
            "INSERT INTO operator_actions(at_wall, revision, actor, command) VALUES (?, ?, ?, ?)",
        )
        .bind(SqlMillis::at(action.at.wall))
        .bind(i64::try_from(action.revision.get()).unwrap_or(i64::MAX))
        .bind(serde_json::to_string(&action.actor)?)
        .bind(action.command.as_str())
        .execute(&mut *self.tx)
        .await?;
        // Actions commit in revision order, so the last one is the current state.
        sqlx::query(
            "INSERT INTO operator_state(id, revision, state) VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET revision = ?1, state = ?2",
        )
        .bind(i64::try_from(action.revision.get()).unwrap_or(i64::MAX))
        .bind(action.state.as_str())
        .execute(&mut *self.tx)
        .await?;
        // The log needs a channel; operator actions share the nil channel,
        // which has no row.
        self.log(
            action.at,
            ChannelId::restore(uuid::Uuid::nil()),
            None,
            None,
            ObservationKind::OperatorAction,
            action,
        )
        .await
    }

    pub(super) async fn session(
        &mut self,
        record: &SessionRecord,
    ) -> Result<Observation, StoreError> {
        let at = SqlMillis::at(record.at.wall);
        match &record.fact {
            SessionFact::Created {
                harness,
                workdir,
                settings,
            } => {
                sqlx::query(
                    "INSERT INTO operator_sessions(id, harness, workdir, created_at, model, effort, approvals) VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(record.session.to_string())
                .bind(harness.label())
                .bind(workdir)
                .bind(at)
                .bind(settings.model.as_ref().map(ModelId::as_str))
                .bind(settings.effort.map(Labeled::label))
                .bind(settings.approvals.map(Labeled::label))
                .execute(&mut *self.tx)
                .await?;
            }
            SessionFact::Configured { settings } => {
                sqlx::query(
                    "UPDATE operator_sessions SET model = ?, effort = ?, approvals = ? WHERE id = ?",
                )
                .bind(settings.model.as_ref().map(ModelId::as_str))
                .bind(settings.effort.map(Labeled::label))
                .bind(settings.approvals.map(Labeled::label))
                .bind(record.session.to_string())
                .execute(&mut *self.tx)
                .await?;
            }
            SessionFact::ClientChosen { harness } => {
                sqlx::query("UPDATE operator_sessions SET harness = ? WHERE id = ?")
                    .bind(harness.label())
                    .bind(record.session.to_string())
                    .execute(&mut *self.tx)
                    .await?;
            }
            SessionFact::FolderChosen { workdir } => {
                sqlx::query("UPDATE operator_sessions SET workdir = ? WHERE id = ?")
                    .bind(workdir)
                    .bind(record.session.to_string())
                    .execute(&mut *self.tx)
                    .await?;
            }
            SessionFact::Assigned { project } => {
                sqlx::query("UPDATE operator_sessions SET project = ? WHERE id = ?")
                    .bind(project.map(|project| project.to_string()))
                    .bind(record.session.to_string())
                    .execute(&mut *self.tx)
                    .await?;
            }
            SessionFact::Archived | SessionFact::Unarchived => {
                let archived = match record.fact {
                    SessionFact::Archived => Some(at),
                    SessionFact::Unarchived
                    | SessionFact::Created { .. }
                    | SessionFact::Configured { .. }
                    | SessionFact::ClientChosen { .. }
                    | SessionFact::FolderChosen { .. }
                    | SessionFact::Attached { .. }
                    | SessionFact::Detached { .. }
                    | SessionFact::AttachmentsSent { .. }
                    | SessionFact::Assigned { .. } => None,
                };
                sqlx::query("UPDATE operator_sessions SET archived_at = ? WHERE id = ?")
                    .bind(archived)
                    .bind(record.session.to_string())
                    .execute(&mut *self.tx)
                    .await?;
            }
            SessionFact::Attached { attachment, read } => {
                let AttachmentKind::Image(image) = read.kind();
                sqlx::query(
                    "INSERT INTO attachments(id, operator_session, name, format, width, height, size, sha256, added_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(attachment.to_string())
                .bind(record.session.to_string())
                .bind(read.name().as_str())
                .bind(image.format().label())
                .bind(image.dimensions().width())
                .bind(image.dimensions().height())
                .bind(i64::try_from(read.size().get()).unwrap_or(i64::MAX))
                .bind(read.sha256().to_string())
                .bind(at)
                .execute(&mut *self.tx)
                .await?;
            }
            SessionFact::AttachmentsSent { attachments } => {
                for attachment in attachments {
                    sqlx::query(
                        "UPDATE attachments SET sent_at = ? WHERE id = ? AND operator_session = ?",
                    )
                    .bind(at)
                    .bind(attachment.to_string())
                    .bind(record.session.to_string())
                    .execute(&mut *self.tx)
                    .await?;
                }
            }
            SessionFact::Detached { attachment } => {
                sqlx::query("DELETE FROM attachments WHERE id = ? AND operator_session = ?")
                    .bind(attachment.to_string())
                    .bind(record.session.to_string())
                    .execute(&mut *self.tx)
                    .await?;
            }
        }
        // Sessions share the nil channel, which has no row, as runs do.
        self.log(
            record.at,
            ChannelId::restore(uuid::Uuid::nil()),
            None,
            None,
            record.fact.kind(),
            record,
        )
        .await
    }

    pub(super) async fn run(&mut self, record: &RunRecord) -> Result<Observation, StoreError> {
        let id = record.run.to_string();
        let at = SqlMillis::at(record.at.wall);
        match &record.fact {
            RunFact::Started {
                session,
                harness,
                workdir,
                settings,
                toolset,
            } => {
                sqlx::query(
                    "INSERT INTO runs(id, operator_session, harness, workdir, started_at, model, effort, approvals, toolset_generation) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(session.to_string())
                .bind(harness.label())
                .bind(workdir)
                .bind(at)
                .bind(settings.model.as_ref().map(ModelId::as_str))
                .bind(settings.effort.map(Labeled::label))
                .bind(settings.approvals.map(Labeled::label))
                .bind(toolset.and_then(|generation| i64::try_from(generation.number()).ok()))
                .execute(&mut *self.tx)
                .await?;
            }
            RunFact::Configured { settings } => {
                // The session's settings are its newest, as its next turn runs.
                for table in ["runs", "operator_sessions"] {
                    let target = match table {
                        "runs" => "id = ?",
                        _ => "id = (SELECT operator_session FROM runs WHERE id = ?)",
                    };
                    sqlx::query(AssertSqlSafe(format!(
                        "UPDATE {table} SET model = ?, effort = ?, approvals = ? WHERE {target}"
                    )))
                    .bind(settings.model.as_ref().map(ModelId::as_str))
                    .bind(settings.effort.map(Labeled::label))
                    .bind(settings.approvals.map(Labeled::label))
                    .bind(&id)
                    .execute(&mut *self.tx)
                    .await?;
                }
            }
            // Observations alone: the gap between them is the time no daemon
            // watched the run.
            RunFact::Detached | RunFact::Rejoined => {}
            RunFact::Passed { passage } => {
                let bytes = passage.bytes();
                sqlx::query(
                    "INSERT INTO run_passages(run, seq, at, asked, end, through, sent, received) VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM run_passages WHERE run = ?1), ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .bind(&id)
                .bind(at)
                .bind(passage.asked())
                .bind(passage.end().label())
                .bind(passage.through())
                .bind(bytes.map(|bytes| i64::try_from(bytes.sent).unwrap_or(i64::MAX)))
                .bind(bytes.map(|bytes| i64::try_from(bytes.received).unwrap_or(i64::MAX)))
                .execute(&mut *self.tx)
                .await?;
            }
            RunFact::Opened { thread } => {
                sqlx::query("UPDATE runs SET thread = ? WHERE id = ?")
                    .bind(thread.to_string())
                    .bind(&id)
                    .execute(&mut *self.tx)
                    .await?;
            }
            RunFact::Ended { end, exit } => {
                let parts = exit.map(ExitParts::from);
                let kind = parts.map(|parts| parts.kind.label());
                let value = parts.and_then(|parts| parts.value);
                sqlx::query(
                    "UPDATE runs SET ended_at = ?, run_end = ?, exit_kind = ?, exit_value = ? WHERE id = ?",
                )
                .bind(at)
                .bind(end.label())
                .bind(kind)
                .bind(value)
                .bind(&id)
                .execute(&mut *self.tx)
                .await?;
            }
        }
        // Runs share the nil channel, which has no row, as operator actions do.
        self.log(
            record.at,
            ChannelId::restore(uuid::Uuid::nil()),
            None,
            None,
            record.fact.kind(),
            record,
        )
        .await
    }

    /// Store one entry of a run's conversation: appended, or, for an item
    /// already stored, in its place.
    pub(super) async fn run_entry(&mut self, entry: &RunEntry) -> Result<(), StoreError> {
        let run = entry.run.to_string();
        let item = match &entry.event {
            DriveEvent::Item { item, .. } => Some(item.id.as_str().to_owned()),
            DriveEvent::Ready { .. }
            | DriveEvent::TurnStarted { .. }
            | DriveEvent::Delta { .. }
            | DriveEvent::Approval { .. }
            | DriveEvent::ApprovalResolved { .. }
            | DriveEvent::TurnEnded { .. }
            | DriveEvent::Error { .. }
            | DriveEvent::Refused { .. }
            | DriveEvent::Unsupported { .. }
            | DriveEvent::Unknown { .. }
            | DriveEvent::Unreadable { .. } => None,
        };
        // A prompt's text, kept beside the event for the sessions list.
        let prompt = if let DriveEvent::Item { item, .. } = &entry.event
            && let ItemRead::Prompt { text, .. } = &item.read
        {
            Some(text.as_str())
        } else {
            None
        };
        let kind = DriveEventKind::from(&entry.event);
        let event = serde_json::to_string(&entry.event)?;
        sqlx::query(
            "INSERT INTO run_entries(run, seq, at, started_at, turn, item, kind, event, prompt) VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM run_entries WHERE run = ?1), ?2, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(run, item) WHERE item IS NOT NULL DO UPDATE SET at = excluded.at, kind = excluded.kind, event = excluded.event, prompt = excluded.prompt",
        )
        .bind(&run)
        .bind(SqlMillis::at(entry.at.wall))
        .bind(entry.turn.map(|turn| turn.to_string()))
        .bind(item)
        .bind(kind.label())
        .bind(event)
        .bind(prompt)
        .execute(&mut *self.tx)
        .await?;
        Ok(())
    }

    /// Archive, unarchive, or delete a recording, a run, or an operator
    /// session. A run's edit applies to the recordings its client made under
    /// its thread too, and a recording's to the runs that worked in it, so
    /// the two never disagree.
    pub(super) async fn history(&mut self, edit: &HistoryEdit) -> Result<Observation, StoreError> {
        let at = SqlMillis::at(edit.at.wall);
        let (recordings, runs) = self.subjects(edit.subject).await?;
        match edit.change {
            HistoryChange::Archive | HistoryChange::Unarchive => {
                let at = match edit.change {
                    HistoryChange::Archive => Some(at),
                    HistoryChange::Unarchive | HistoryChange::Delete => None,
                };
                for (table, ids) in [("recordings", &recordings), ("runs", &runs)] {
                    for id in ids {
                        sqlx::query(AssertSqlSafe(format!(
                            "UPDATE {table} SET archived_at = ? WHERE id = ?"
                        )))
                        .bind(at)
                        .bind(id)
                        .execute(&mut *self.tx)
                        .await?;
                    }
                }
                // An operator session's own shelf; otherwise, while each
                // operator session has one run, it is archived with it.
                if let HistorySubject::OperatorSession(session) = edit.subject {
                    sqlx::query("UPDATE operator_sessions SET archived_at = ? WHERE id = ?")
                        .bind(at)
                        .bind(session.to_string())
                        .execute(&mut *self.tx)
                        .await?;
                }
                for run in &runs {
                    sqlx::query("UPDATE operator_sessions SET archived_at = ? WHERE id = (SELECT operator_session FROM runs WHERE id = ?)")
                        .bind(at)
                        .bind(run)
                        .execute(&mut *self.tx)
                        .await?;
                }
            }
            HistoryChange::Delete => {
                for recording in &recordings {
                    let calls = sqlx::query(
                        "SELECT id, channel FROM calls WHERE recording = ? AND state <> ?",
                    )
                    .bind(recording)
                    .bind(CallState::Open.label())
                    .fetch_all(&mut *self.tx)
                    .await?;
                    for row in calls {
                        let call: String = row.try_get("id")?;
                        let channel: String = row.try_get("channel")?;
                        self.retire(&call, &channel, Some(recording)).await?;
                    }
                }
                for run in &runs {
                    let session: Option<String> =
                        sqlx::query_scalar("SELECT operator_session FROM runs WHERE id = ?")
                            .bind(run)
                            .fetch_optional(&mut *self.tx)
                            .await?
                            .flatten();
                    // A running run stays, with its log rows.
                    let ended =
                        sqlx::query("DELETE FROM runs WHERE id = ? AND ended_at IS NOT NULL")
                            .bind(run)
                            .execute(&mut *self.tx)
                            .await?
                            .rows_affected();
                    if ended > 0 {
                        for table in ["run_entries", "run_passages"] {
                            sqlx::query(AssertSqlSafe(format!(
                                "DELETE FROM {table} WHERE run = ?"
                            )))
                            .bind(run)
                            .execute(&mut *self.tx)
                            .await?;
                        }
                        sqlx::query("DELETE FROM observations WHERE call IS NULL AND kind IN (?, ?, ?, ?, ?, ?) AND json_extract(payload, '$.run') = ?")
                            .bind(ObservationKind::RunStarted.label())
                            .bind(ObservationKind::RunOpened.label())
                            .bind(ObservationKind::RunConfigured.label())
                            .bind(ObservationKind::RunEnded.label())
                            .bind(ObservationKind::RunRecovered.label())
                            .bind(ObservationKind::RunPassed.label())
                            .bind(run)
                            .execute(&mut *self.tx)
                            .await?;
                        // An operator session goes with its last run.
                        if let Some(session) = &session {
                            self.drop_session(session).await?;
                        }
                    }
                }
                // An operator session goes even with no run to take it.
                if let HistorySubject::OperatorSession(session) = edit.subject {
                    self.drop_session(&session.to_string()).await?;
                }
            }
        }
        // History edits share the nil channel, which has no row.
        self.log(
            edit.at,
            ChannelId::restore(uuid::Uuid::nil()),
            None,
            None,
            ObservationKind::HistoryEdited,
            edit,
        )
        .await
    }

    /// Delete operator session `session` and its own facts, once no run
    /// of it is left.
    pub(super) async fn drop_session(&mut self, session: &str) -> Result<(), StoreError> {
        let gone = sqlx::query("DELETE FROM operator_sessions WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM runs WHERE operator_session = ?1)")
            .bind(session)
            .execute(&mut *self.tx)
            .await?
            .rows_affected();
        if gone > 0 {
            sqlx::query("DELETE FROM observations WHERE call IS NULL AND kind IN (?, ?, ?, ?, ?, ?, ?, ?, ?) AND json_extract(payload, '$.session') = ?")
                .bind(ObservationKind::SessionCreated.label())
                .bind(ObservationKind::SessionConfigured.label())
                .bind(ObservationKind::SessionClientChosen.label())
                .bind(ObservationKind::SessionFolderChosen.label())
                .bind(ObservationKind::SessionArchived.label())
                .bind(ObservationKind::SessionUnarchived.label())
                .bind(ObservationKind::SessionAttached.label())
                .bind(ObservationKind::SessionDetached.label())
                .bind(ObservationKind::SessionAttachmentsSent.label())
                .bind(session)
                .execute(&mut *self.tx)
                .await?;
            sqlx::query("DELETE FROM attachments WHERE operator_session = ?")
                .bind(session)
                .execute(&mut *self.tx)
                .await?;
        }
        Ok(())
    }

    /// The recordings and runs an edit of `subject` applies to: a recording
    /// and the runs whose thread it is, a run and the recordings its thread
    /// names, or an operator session's runs and the recordings they name.
    pub(super) async fn subjects(
        &mut self,
        subject: HistorySubject,
    ) -> Result<(Vec<String>, Vec<String>), StoreError> {
        let (id, recordings, runs) = match subject {
            HistorySubject::Recording(recording) => (
                recording.to_string(),
                "SELECT id FROM recordings WHERE id = ?1",
                "SELECT id FROM runs WHERE thread = (SELECT native_id FROM recordings WHERE id = ?1)",
            ),
            HistorySubject::Run(run) => (
                run.to_string(),
                "SELECT id FROM recordings WHERE native_id = (SELECT thread FROM runs WHERE id = ?1)",
                "SELECT id FROM runs WHERE id = ?1",
            ),
            HistorySubject::OperatorSession(session) => (
                session.to_string(),
                "SELECT id FROM recordings WHERE native_id IN (SELECT thread FROM runs WHERE operator_session = ?1)",
                "SELECT id FROM runs WHERE operator_session = ?1",
            ),
        };
        let recordings = sqlx::query_scalar(recordings)
            .bind(&id)
            .fetch_all(&mut *self.tx)
            .await?;
        let runs = sqlx::query_scalar(runs)
            .bind(&id)
            .fetch_all(&mut *self.tx)
            .await?;
        Ok((recordings, runs))
    }
}
