//! Work items' facts, as rows of `work_items`.

use super::MetadataTx;
use crate::{StoreError, millis::SqlMillis};
use ren_protocol::label::Labeled;
use ren_recorder::{
    Observation,
    batch::{WorkFact, WorkRecord},
    id::ChannelId,
    work::WorkStateKind,
};

impl MetadataTx<'_> {
    pub(super) async fn work(&mut self, record: &WorkRecord) -> Result<Observation, StoreError> {
        let id = record.item.to_string();
        let at = SqlMillis::at(record.at.wall);
        let update = |state: WorkStateKind, column: Option<&'static str>| {
            let set = column.map_or_else(String::new, |column| format!(", {column} = ?"));
            (
                state,
                format!("UPDATE work_items SET state = ?, updated_at = ?{set} WHERE id = ?"),
            )
        };
        match &record.fact {
            WorkFact::Queued { order } => {
                let json = serde_json::to_string(order)?;
                sqlx::query(
                    "INSERT INTO work_items(id, created_at, updated_at, title, work_order, state) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(at)
                .bind(at)
                .bind(order.title())
                .bind(json)
                .bind(WorkStateKind::Queued.label())
                .execute(&mut *self.tx)
                .await?;
            }
            WorkFact::Sent { session } => {
                let (state, sql) = update(WorkStateKind::Sent, Some("operator_session"));
                sqlx::query(sqlx::AssertSqlSafe(sql))
                    .bind(state.label())
                    .bind(at)
                    .bind(session.to_string())
                    .bind(&id)
                    .execute(&mut *self.tx)
                    .await?;
            }
            WorkFact::Landed { commit } => {
                let (state, sql) = update(WorkStateKind::Landed, Some("landed_commit"));
                sqlx::query(sqlx::AssertSqlSafe(sql))
                    .bind(state.label())
                    .bind(at)
                    .bind(commit.to_string())
                    .bind(&id)
                    .execute(&mut *self.tx)
                    .await?;
            }
            WorkFact::Shipped { revision } => {
                let (state, sql) = update(WorkStateKind::Shipped, Some("shipped_revision"));
                sqlx::query(sqlx::AssertSqlSafe(sql))
                    .bind(state.label())
                    .bind(at)
                    .bind(revision.to_string())
                    .bind(&id)
                    .execute(&mut *self.tx)
                    .await?;
            }
            WorkFact::Dismissed => {
                let (state, sql) = update(WorkStateKind::Dismissed, None);
                sqlx::query(sqlx::AssertSqlSafe(sql))
                    .bind(state.label())
                    .bind(at)
                    .bind(&id)
                    .execute(&mut *self.tx)
                    .await?;
            }
        }
        // Work items share the nil channel, which has no row, as sessions do.
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
}
