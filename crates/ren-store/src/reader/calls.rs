//! Building the history listing's SQL: a [`CallQuery`]'s filters and
//! cursor, and the [`CallPage`] read back from its rows.

use crate::{
    StoreError,
    query::{CallQuery, KindFilter, PageLimit, StatusFilter, ToolFilter},
    view::{CallPage, CallView},
};
use ren_harness::Harness;
use ren_harness::attribution::HintSource;
use ren_harness::tools::CallPurpose;
use ren_protocol::inspect::{InputAccounting, ModelOutcome};
use ren_protocol::{
    label::Labeled,
    route::{Endpoint, Method},
};
use ren_recorder::capture::{AttemptTerminal, CallState, TerminalClass};
use ren_recorder::project::ProjectName;
use sqlx::{QueryBuilder, Sqlite};

impl CallQuery {
    /// An `AND` clause for calls (aliased `c`) a harness made: known from
    /// the convention that attributed each one.
    pub(crate) fn push_client(sql: &mut QueryBuilder<Sqlite>, client: Harness) {
        sql.push(" AND json_extract(c.attribution, '$.hint.source') IN (");
        let mut sources = sql.separated(", ");
        for source in HintSource::ALL
            .iter()
            .filter(|source| source.harness() == client)
        {
            sources.push_bind(source.label());
        }
        sql.push(")");
    }

    /// Whether `calls c` is a model call: a request for model output, not a
    /// token count, a model listing, a WebSocket handshake (recorded as a
    /// GET on a model endpoint), or a client's own check such as Claude
    /// Code's quota probe. A call whose digest is not yet derived counts as
    /// a model call.
    fn push_model_call(sql: &mut QueryBuilder<Sqlite>) {
        sql.push("(c.method <> ")
            .push_bind(Method::Get.label())
            .push(" AND c.endpoint IN (");
        let mut endpoints = sql.separated(", ");
        for endpoint in Endpoint::ALL.iter().filter(|endpoint| endpoint.generates()) {
            endpoints.push_bind(endpoint.label());
        }
        sql.push(
            ") AND NOT EXISTS (SELECT 1 FROM call_digests d WHERE d.call = c.id AND d.purpose IN (",
        );
        let mut purposes = sql.separated(", ");
        for purpose in CallPurpose::ALL
            .iter()
            .filter(|purpose| **purpose != CallPurpose::Work)
        {
            purposes.push_bind(purpose.label());
        }
        sql.push(")))");
    }

    /// The condition a recording `s` meets when it is the conversation of a
    /// run of a session working for `project`: the run's thread is its
    /// native id.
    pub(crate) fn push_project_recording(sql: &mut QueryBuilder<Sqlite>, project: &ProjectName) {
        sql.push("s.native_id IN (SELECT r.thread FROM runs r JOIN operator_sessions o ON o.id = r.operator_session JOIN projects p ON p.id = o.project WHERE p.name = ")
            .push_bind(project.as_str().to_owned())
            .push(" AND r.thread IS NOT NULL)");
    }

    /// `calls`, `errors`, `input_tokens`, `output_tokens`, and
    /// `cache_read_tokens` aggregated over the rows of `calls c`. Errors are
    /// the model calls the `errors` status filter finds: a client's own
    /// check or a failed token count is not a failure of the recording's
    /// work. Input adds cached
    /// context where the provider reported it separately. A token SUM over
    /// calls that reported no usage is NULL: "not reported", not zero.
    pub(crate) fn push_usage_columns(sql: &mut QueryBuilder<Sqlite>) {
        // The failure test comes first: it is cheap, and false for most
        // calls, so the model-call test's digest lookup runs only for the
        // few that failed.
        sql.push(
            "COUNT(c.id) AS calls, COALESCE(SUM(CASE WHEN (c.status >= 400 OR c.terminal IN (",
        );
        Self::push_terminals(sql, TerminalClass::Failed);
        sql.push(") OR c.outcome = ")
            .push_bind(ModelOutcome::Failed.label())
            .push(") AND ");
        Self::push_model_call(sql);
        sql.push(" THEN 1 ELSE 0 END), 0) AS errors, ")
            .push("SUM(CASE WHEN c.input_accounting IS NULL THEN NULL ELSE COALESCE(c.input_tokens, 0) + CASE WHEN c.input_accounting = ")
            .push_bind(InputAccounting::ExcludesCache.label())
            .push(" THEN COALESCE(c.cache_read_tokens, 0) + COALESCE(c.cache_creation_tokens, 0) ELSE 0 END END) AS input_tokens, ")
            .push("SUM(c.output_tokens) AS output_tokens, ")
            .push("SUM(c.cache_read_tokens) AS cache_read_tokens");
    }

    /// Append this query's filters as WHERE clauses.
    /// Bind every terminal label of one class, comma-separated.
    fn push_terminals(sql: &mut QueryBuilder<Sqlite>, class: TerminalClass) {
        let mut labels = sql.separated(", ");
        for terminal in AttemptTerminal::ALL
            .iter()
            .filter(|terminal| terminal.class() == class)
        {
            labels.push_bind(terminal.label());
        }
    }

    pub(super) fn push_filters(&self, sql: &mut QueryBuilder<Sqlite>) {
        let filter = &self.filter;
        filter.time.push_started(sql);
        if let Some(provider) = filter.provider {
            sql.push(" AND c.provider = ").push_bind(provider.label());
        }
        if let Some(model) = &filter.model {
            sql.push(" AND instr(lower(COALESCE(c.model, '')), lower(")
                .push_bind(model.as_str().to_owned())
                .push(")) > 0");
        }
        if let Some(tool) = &filter.tool {
            // One scan of the tool names, not a lookup per call.
            sql.push(
                " AND c.id IN (SELECT t.call FROM call_tools t WHERE instr(lower(t.name), lower(",
            )
            .push_bind(tool.as_str().to_owned())
            .push(")) > 0)");
        }
        if let Some(text) = &filter.text {
            let text = text.as_str().to_owned();
            sql.push(" AND (instr(lower(c.id), lower(")
                .push_bind(text.clone())
                .push(")) > 0 OR instr(lower(COALESCE(c.model, '')), lower(")
                .push_bind(text.clone())
                .push(")) > 0 OR c.id IN (SELECT t.call FROM call_tools t WHERE instr(lower(t.name), lower(")
                .push_bind(text)
                .push(")) > 0))");
        }
        if let Some(recording) = filter.recording {
            sql.push(" AND c.recording = ")
                .push_bind(recording.to_string());
        }
        if let Some(project) = &filter.project {
            sql.push(" AND c.recording IN (SELECT s.id FROM recordings s WHERE ");
            Self::push_project_recording(sql, project);
            sql.push(")");
        }
        if let Some(client) = filter.client {
            Self::push_client(sql, client);
        }
        if let Some(user) = &filter.user {
            sql.push(" AND c.user = ")
                .push_bind(user.as_str().to_owned());
        }
        if let Some(kind) = filter.kind {
            sql.push(match kind {
                KindFilter::Model => " AND ",
                KindFilter::Other => " AND NOT ",
            });
            Self::push_model_call(sql);
        }
        if let Some(tools) = filter.tools {
            sql.push(match tools {
                ToolFilter::WithTools => " AND c.tool_count > 0",
                ToolFilter::WithoutTools => " AND c.tool_count = 0 AND c.state <> 'open'",
            });
        }
        if let Some(status) = filter.status {
            match status {
                StatusFilter::Active => {
                    sql.push(" AND c.state = ")
                        .push_bind(CallState::Open.label());
                }
                StatusFilter::Finished => {
                    sql.push(" AND c.state = ")
                        .push_bind(CallState::Finished.label());
                }
                StatusFilter::Interrupted => {
                    sql.push(" AND (c.state = ")
                        .push_bind(CallState::Recovered.label())
                        .push(" OR c.terminal IN (");
                    Self::push_terminals(sql, TerminalClass::Stopped);
                    sql.push("))");
                }
                StatusFilter::Errors => {
                    sql.push(" AND (c.status >= 400 OR c.terminal IN (");
                    Self::push_terminals(sql, TerminalClass::Failed);
                    sql.push(") OR c.outcome = ")
                        .push_bind(ModelOutcome::Failed.label())
                        .push(")");
                }
            }
        }
    }
}

impl CallPage {
    pub(super) fn from_rows(
        rows: &[sqlx::sqlite::SqliteRow],
        limit: PageLimit,
    ) -> Result<Self, StoreError> {
        let mut calls = rows
            .iter()
            .take(limit.rows())
            .map(CallView::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next = (rows.len() > limit.rows())
            .then(|| calls.last().map(CallView::cursor))
            .flatten();
        calls.shrink_to_fit();
        Ok(Self { calls, next })
    }
}
