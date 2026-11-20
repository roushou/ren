//! How a batch's call and channel records are written: messages, calls,
//! attempts, bodies and their segments, digests, and the removal of a
//! retired call.

use super::{MetadataTx, StoredChunk};
use crate::StoreError;
use crate::millis::SqlMillis;
use ren_protocol::{direction::Direction, label::Labeled, route::Transport};
use ren_recorder::Observation;
use ren_recorder::caller::{TailscaleLogin, UserId};
use ren_recorder::digest::CallDigest;
use ren_recorder::{
    batch::{
        AttemptBody, BodyEnd, BodyKey, CloseChannel, FinishAttempt, FinishCall, MessageMark,
        ObservationKind, OpenCall, ResponseHead, Segment, StartAttempt,
    },
    inspection::InspectionRecord,
};
use ren_recorder::{
    capture::{BodyEnding, CallState},
    id::{CallId, RecordingId, TurnId},
};

impl MetadataTx<'_> {
    /// Store a call's digest, replacing an earlier one with its exposures
    /// and searchable texts. A call pruned since its digest was derived is
    /// skipped.
    pub(super) async fn digest(
        &mut self,
        call: CallId,
        digest: &CallDigest,
    ) -> Result<(), StoreError> {
        let id = call.to_string();
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM calls WHERE id = ?)")
            .bind(&id)
            .fetch_one(&mut *self.tx)
            .await?;
        if !exists {
            return Ok(());
        }
        for statement in [
            "DELETE FROM call_exposures WHERE call = ?",
            "DELETE FROM search_texts WHERE call = ?",
        ] {
            sqlx::query(statement)
                .bind(&id)
                .execute(&mut *self.tx)
                .await?;
        }
        sqlx::query(
            "INSERT INTO call_digests(call, version, purpose, prompt, prompt_omitted, digest) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(call) DO UPDATE SET version = ?2, purpose = ?3, prompt = ?4, prompt_omitted = ?5, digest = ?6",
        )
        .bind(&id)
        .bind(i64::from(digest.version))
        .bind(digest.purpose.label())
        .bind(digest.prompt.as_ref().map(|prompt| prompt.text.as_str()))
        .bind(
            digest
                .prompt
                .as_ref()
                .map(|prompt| i64::try_from(prompt.omitted_chars).unwrap_or(i64::MAX)),
        )
        .bind(serde_json::to_string(digest)?)
        .execute(&mut *self.tx)
        .await?;
        for exposure in &digest.exposures {
            sqlx::query(
                "INSERT INTO call_exposures(call, field, kind, hint, fingerprint, count) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(exposure.field.label())
            .bind(exposure.kind.label())
            .bind(&exposure.hint)
            .bind(&exposure.fingerprint)
            .bind(i64::try_from(exposure.count).unwrap_or(i64::MAX))
            .execute(&mut *self.tx)
            .await?;
        }
        for text in &digest.search {
            sqlx::query("INSERT INTO search_texts(call, field, text) VALUES (?, ?, ?)")
                .bind(&id)
                .bind(text.field.label())
                .bind(&text.text)
                .execute(&mut *self.tx)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn message(
        &mut self,
        body: BodyKey,
        mark: &MessageMark,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT OR IGNORE INTO messages(owner, direction, byte_offset, len, payload, at_wall) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(body.owner())
            .bind(body.direction().label())
            .bind(i64::try_from(mark.offset).unwrap_or(i64::MAX))
            .bind(i64::try_from(mark.len).unwrap_or(i64::MAX))
            .bind(mark.payload.label())
            .bind(SqlMillis::at(mark.at))
            .execute(&mut *self.tx)
            .await?;
        Ok(())
    }

    pub(super) async fn close_channel(
        &mut self,
        close: &CloseChannel,
    ) -> Result<Observation, StoreError> {
        sqlx::query("UPDATE channels SET closed_at = ?, ending = ?, close_code = ?, unattributed_messages = ?, unattributed_bytes = ? WHERE id = ?")
            .bind(SqlMillis::at(close.at.wall))
            .bind(close.end.label())
            .bind(close.close_code.map(u32::from))
            .bind(i64::try_from(close.unattributed.messages).unwrap_or(i64::MAX))
            .bind(i64::try_from(close.unattributed.bytes).unwrap_or(i64::MAX))
            .bind(close.channel.to_string())
            .execute(&mut *self.tx)
            .await?;
        self.log(
            close.at,
            close.channel,
            None,
            None,
            ObservationKind::ChannelClosed,
            close,
        )
        .await
    }

    pub(super) async fn open_call(&mut self, open: &OpenCall) -> Result<Observation, StoreError> {
        let stamp = &open.stamp;
        let at = SqlMillis::at(stamp.at.wall);
        sqlx::query("INSERT OR IGNORE INTO channels(id, transport, opened_at, provider) VALUES (?, ?, ?, ?)")
            .bind(stamp.channel.to_string())
            .bind(open.transport.label())
            .bind(at)
            .bind(open.provider.label())
            .execute(&mut *self.tx)
            .await?;
        let (recording, turn) = match &open.attribution.hint {
            None => (None, None),
            Some(hint) => {
                let recording: String = sqlx::query_scalar("INSERT INTO recordings(id, provider, source, native_id, first_seen, last_seen) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(provider, source, native_id) DO UPDATE SET first_seen = MIN(first_seen, excluded.first_seen), last_seen = MAX(last_seen, excluded.last_seen) RETURNING id")
                    .bind(RecordingId::generate(self.ids).to_string())
                    .bind(open.provider.label())
                    .bind(hint.source.label())
                    .bind(hint.session.to_string())
                    .bind(at)
                    .bind(at)
                    .fetch_one(&mut *self.tx)
                    .await?;
                let turn = match hint.turn {
                    None => None,
                    Some(native) => Some(
                        sqlx::query_scalar::<_, String>("INSERT INTO turns(id, recording, native_id) VALUES (?, ?, ?) ON CONFLICT(recording, native_id) DO UPDATE SET native_id = excluded.native_id RETURNING id")
                            .bind(TurnId::generate(self.ids).to_string())
                            .bind(&recording)
                            .bind(native.to_string())
                            .fetch_one(&mut *self.tx)
                            .await?,
                    ),
                };
                (Some(recording), turn)
            }
        };
        sqlx::query("INSERT INTO calls(id, channel, started_at, provider, wire, endpoint, method, config_revision, recording, turn, attribution, state, operator_revision, plan, caller, user, tailscale_login) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(stamp.call.to_string())
            .bind(stamp.channel.to_string())
            .bind(at)
            .bind(open.provider.label())
            .bind(open.wire.label())
            .bind(open.endpoint.label())
            .bind(open.method.label())
            .bind(open.config_revision.as_str())
            .bind(recording)
            .bind(turn)
            .bind(serde_json::to_string(&open.attribution)?)
            .bind(CallState::Open.label())
            .bind(
                open.operator_revision
                    .map(|revision| i64::try_from(revision.get()).unwrap_or(i64::MAX)),
            )
            .bind(open.plan.as_ref().map(|plan| plan.as_str()))
            .bind(open.caller.kind().label())
            .bind(open.caller.user().map(UserId::as_str))
            .bind(open.caller.login().map(TailscaleLogin::as_str))
            .execute(&mut *self.tx)
            .await?;
        self.open_body(BodyKey::Request(stamp.call)).await?;
        self.append(stamp, None, ObservationKind::CallOpened, open)
            .await
    }

    pub(super) async fn open_body(&mut self, body: BodyKey) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO bodies(owner, direction, ending) VALUES (?, ?, ?)")
            .bind(body.owner())
            .bind(body.direction().label())
            .bind(BodyEnding::Open.label())
            .execute(&mut *self.tx)
            .await?;
        Ok(())
    }

    pub(super) async fn start_attempt(
        &mut self,
        start: &StartAttempt,
    ) -> Result<Observation, StoreError> {
        let stamp = &start.stamp;
        sqlx::query(
            "INSERT INTO attempts(id, call, ordinal, started_at, upstream, provider, changes) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(start.attempt.to_string())
        .bind(stamp.call.to_string())
        .bind(start.ordinal)
        .bind(SqlMillis::at(stamp.at.wall))
        .bind(start.upstream.as_str())
        .bind(start.provider.label())
        .bind(start.changes.as_ref().map(|changes| changes.as_str()))
        .execute(&mut *self.tx)
        .await?;
        self.open_body(BodyKey::Response(start.attempt)).await?;
        match start.body {
            AttemptBody::Original => {}
            AttemptBody::Changed => {
                self.open_body(BodyKey::AttemptRequest(start.attempt))
                    .await?
            }
        }
        sqlx::query("UPDATE calls SET final_attempt = ? WHERE id = ?")
            .bind(start.attempt.to_string())
            .bind(stamp.call.to_string())
            .execute(&mut *self.tx)
            .await?;
        self.append(
            stamp,
            Some(start.attempt),
            ObservationKind::AttemptStarted,
            start,
        )
        .await
    }

    pub(super) async fn response_head(
        &mut self,
        head: &ResponseHead,
    ) -> Result<Observation, StoreError> {
        let stamp = &head.stamp;
        let limits = (!head.limits.is_empty())
            .then(|| serde_json::to_string(&head.limits))
            .transpose()?;
        sqlx::query("UPDATE attempts SET status = ?, response_format = ?, response_encoding = ?, limit_headers = ? WHERE id = ?")
            .bind(u32::from(head.status))
            .bind(head.format.label())
            .bind(head.encoding.label())
            .bind(limits)
            .bind(head.attempt.to_string())
            .execute(&mut *self.tx)
            .await?;
        sqlx::query("UPDATE calls SET status = ? WHERE id = ?")
            .bind(u32::from(head.status))
            .bind(stamp.call.to_string())
            .execute(&mut *self.tx)
            .await?;
        self.append(
            stamp,
            Some(head.attempt),
            ObservationKind::ResponseHead,
            head,
        )
        .await
    }

    pub(super) async fn segments(
        &mut self,
        body: BodyKey,
        segments: &[Segment],
        chunks: &[StoredChunk],
    ) -> Result<(), StoreError> {
        let mut stored = 0_u64;
        for segment in segments {
            let StoredChunk { hash, len } = *chunks
                .get(segment.chunk.index())
                .ok_or(StoreError::Corrupt("segment chunk reference"))?;
            sqlx::query("INSERT INTO segments(owner, direction, byte_offset, len, chunk) VALUES (?, ?, ?, ?, ?)")
                .bind(body.owner())
                .bind(body.direction().label())
                .bind(i64::try_from(segment.offset).unwrap_or(i64::MAX))
                .bind(i64::try_from(len).unwrap_or(i64::MAX))
                .bind(hash.as_bytes())
                .execute(&mut *self.tx)
                .await?;
            stored = stored.saturating_add(len);
        }
        sqlx::query(
            "UPDATE bodies SET stored_bytes = stored_bytes + ? WHERE owner = ? AND direction = ?",
        )
        .bind(i64::try_from(stored).unwrap_or(i64::MAX))
        .bind(body.owner())
        .bind(body.direction().label())
        .execute(&mut *self.tx)
        .await?;
        Ok(())
    }

    pub(super) async fn end_body(&mut self, body: BodyKey, end: BodyEnd) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE bodies SET observed_bytes = ?, ending = ? WHERE owner = ? AND direction = ?",
        )
        .bind(i64::try_from(end.observed_bytes).unwrap_or(i64::MAX))
        .bind(end.ending.label())
        .bind(body.owner())
        .bind(body.direction().label())
        .execute(&mut *self.tx)
        .await?;
        Ok(())
    }

    pub(super) async fn finish_attempt(
        &mut self,
        finish: &FinishAttempt,
    ) -> Result<Observation, StoreError> {
        let stamp = &finish.stamp;
        let ended = SqlMillis::at(stamp.at.wall);
        let inspection = finish.inspection.as_deref();
        sqlx::query("UPDATE attempts SET terminal = ?, terminal_cause = ?, terminal_phase = ?, ended_at = ?, inspection = ?, inspection_version = ? WHERE id = ?")
            .bind(finish.end.terminal.label())
            .bind(finish.end.cause.map(Labeled::label))
            .bind(finish.end.phase.map(Labeled::label))
            .bind(ended)
            .bind(inspection.map(serde_json::to_string).transpose()?)
            .bind(inspection.map(|_| InspectionRecord::VERSION))
            .bind(finish.attempt.to_string())
            .execute(&mut *self.tx)
            .await?;
        self.end_body(BodyKey::Response(finish.attempt), finish.response)
            .await?;
        if let Some(request) = finish.request {
            self.end_body(BodyKey::AttemptRequest(finish.attempt), request)
                .await?;
        }
        let usage = inspection.and_then(|record| record.usage.as_ref());
        let count =
            |count: Option<u64>| count.map(|count| i64::try_from(count).unwrap_or(i64::MAX));
        sqlx::query("UPDATE calls SET terminal = ?, model = ?, outcome = ?, stop_reason = ?, tool_count = ?, input_accounting = ?, input_tokens = ?, output_tokens = ?, cache_read_tokens = ?, cache_creation_tokens = ?, reasoning_tokens = ? WHERE id = ?")
            .bind(finish.end.terminal.label())
            .bind(inspection.and_then(|record| record.model.clone()))
            .bind(inspection.and_then(|record| record.outcome.as_ref().map(|outcome| outcome.label().to_owned())))
            .bind(inspection.and_then(|record| record.stop_reason.clone()))
            .bind(inspection.map_or(0, |record| i64::try_from(record.tool_calls.len()).unwrap_or(i64::MAX)))
            .bind(usage.map(|usage| usage.accounting.label().to_owned()))
            .bind(usage.and_then(|usage| count(usage.input_tokens)))
            .bind(usage.and_then(|usage| count(usage.output_tokens)))
            .bind(usage.and_then(|usage| count(usage.cache_read_tokens)))
            .bind(usage.and_then(|usage| count(usage.cache_creation_tokens)))
            .bind(usage.and_then(|usage| count(usage.reasoning_tokens)))
            .bind(stamp.call.to_string())
            .execute(&mut *self.tx)
            .await?;
        // The final attempt's tools replace an earlier attempt's.
        sqlx::query("DELETE FROM call_tools WHERE call = ?")
            .bind(stamp.call.to_string())
            .execute(&mut *self.tx)
            .await?;
        let tools = inspection
            .map(InspectionRecord::tool_names)
            .unwrap_or_default();
        for (position, name) in tools.into_iter().enumerate() {
            sqlx::query("INSERT INTO call_tools(call, position, name) VALUES (?, ?, ?)")
                .bind(stamp.call.to_string())
                .bind(i64::try_from(position).unwrap_or(i64::MAX))
                .bind(name)
                .execute(&mut *self.tx)
                .await?;
        }
        self.append(
            stamp,
            Some(finish.attempt),
            ObservationKind::AttemptFinished,
            finish,
        )
        .await
    }

    pub(super) async fn finish_call(
        &mut self,
        finish: &FinishCall,
    ) -> Result<Observation, StoreError> {
        let stamp = &finish.stamp;
        self.end_body(BodyKey::Request(stamp.call), finish.request)
            .await?;
        // Complete only when every body of the call ended and was fully stored.
        sqlx::query("UPDATE calls SET state = ?, ended_at = ?, rejection = ?, capture_complete = (SELECT COALESCE(MIN(b.ending = ? AND b.stored_bytes = b.observed_bytes), 0) FROM bodies b WHERE (b.owner = calls.id AND b.direction = ?) OR b.owner IN (SELECT id FROM attempts WHERE call = calls.id)) WHERE id = ?")
            .bind(CallState::Finished.label())
            .bind(SqlMillis::at(stamp.at.wall))
            .bind(finish.rejection.map(Labeled::label))
            .bind(BodyEnding::Ended.label())
            .bind(Direction::Request.label())
            .bind(stamp.call.to_string())
            .execute(&mut *self.tx)
            .await?;
        self.append(stamp, None, ObservationKind::CallFinished, finish)
            .await
    }

    /// Remove a call and everything only it references.
    pub(super) async fn retire(
        &mut self,
        call: &str,
        channel: &str,
        recording: Option<&str>,
    ) -> Result<(), StoreError> {
        let request = Direction::Request.label();
        for statement in [
            "DELETE FROM segments WHERE (owner = ?1 AND direction = ?2) OR owner IN (SELECT id FROM attempts WHERE call = ?1)",
            "DELETE FROM messages WHERE (owner = ?1 AND direction = ?2) OR owner IN (SELECT id FROM attempts WHERE call = ?1)",
            "DELETE FROM bodies WHERE (owner = ?1 AND direction = ?2) OR owner IN (SELECT id FROM attempts WHERE call = ?1)",
        ] {
            sqlx::query(statement)
                .bind(call)
                .bind(request)
                .execute(&mut *self.tx)
                .await?;
        }
        for statement in [
            "DELETE FROM observations WHERE call = ?",
            "DELETE FROM attempts WHERE call = ?",
            "DELETE FROM calls WHERE id = ?",
        ] {
            sqlx::query(statement)
                .bind(call)
                .execute(&mut *self.tx)
                .await?;
        }
        // A channel's own log rows go with its last call; an open channel
        // keeps its row, so its close can still be recorded.
        sqlx::query("DELETE FROM observations WHERE channel = ?1 AND call IS NULL AND NOT EXISTS (SELECT 1 FROM calls WHERE channel = ?1) AND EXISTS (SELECT 1 FROM channels WHERE id = ?1 AND (transport <> ?2 OR ending IS NOT NULL))")
            .bind(channel)
            .bind(Transport::WebSocket.label())
            .execute(&mut *self.tx)
            .await?;
        sqlx::query("DELETE FROM channels WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM calls WHERE channel = ?1) AND (transport <> ?2 OR ending IS NOT NULL)")
            .bind(channel)
            .bind(Transport::WebSocket.label())
            .execute(&mut *self.tx)
            .await?;
        if let Some(recording) = recording {
            sqlx::query("DELETE FROM turns WHERE recording = ? AND NOT EXISTS (SELECT 1 FROM calls WHERE turn = turns.id)")
                .bind(recording)
                .execute(&mut *self.tx)
                .await?;
            sqlx::query("DELETE FROM recordings WHERE id = ? AND NOT EXISTS (SELECT 1 FROM calls WHERE recording = recordings.id)")
                .bind(recording)
                .execute(&mut *self.tx)
                .await?;
        }
        Ok(())
    }
}
