//! Projects' facts, as rows of `projects`.

use super::MetadataTx;
use crate::{StoreError, millis::SqlMillis};
use ren_protocol::label::Labeled;
use ren_recorder::{
    Observation,
    batch::{ProjectFact, ProjectRecord},
    id::ChannelId,
};

impl MetadataTx<'_> {
    pub(super) async fn project(
        &mut self,
        record: &ProjectRecord,
    ) -> Result<Observation, StoreError> {
        let id = record.project.to_string();
        let at = SqlMillis::at(record.at.wall);
        match &record.fact {
            // The first project to take a name or a repository keeps it: a
            // later one naming either again is not stored (the gateway
            // refuses it first; this covers two registered at once).
            ProjectFact::Registered { registration } => {
                sqlx::query(
                    "INSERT INTO projects(id, name, repository, factory, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
                )
                .bind(&id)
                .bind(registration.name.as_str())
                .bind(registration.repository.to_string())
                .bind(registration.factory.label())
                .bind(at)
                .execute(&mut *self.tx)
                .await?;
            }
            ProjectFact::Archived => {
                sqlx::query(
                    "UPDATE projects SET archived_at = ? WHERE id = ? AND archived_at IS NULL",
                )
                .bind(at)
                .bind(&id)
                .execute(&mut *self.tx)
                .await?;
            }
            ProjectFact::Unarchived => {
                sqlx::query("UPDATE projects SET archived_at = NULL WHERE id = ?")
                    .bind(&id)
                    .execute(&mut *self.tx)
                    .await?;
            }
        }
        // Projects share the nil channel, which has no row, as work items do.
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
