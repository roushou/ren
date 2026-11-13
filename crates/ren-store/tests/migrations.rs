//! Migrations applied to rows an earlier version wrote. Each migration's SQL
//! runs in order on an in-memory database, so a migration that fills a new
//! column is checked against the rows that existed before it.
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use sqlx::{Connection, Row, SqliteConnection};

const BEFORE_WIRE: [&str; 3] = [
    include_str!("../migrations/0001_store.sql"),
    include_str!("../migrations/0002_digests.sql"),
    include_str!("../migrations/0003_call_purpose.sql"),
];
const WIRE: &str = include_str!("../migrations/0004_wire.sql");

#[tokio::test]
async fn wire_and_attempt_providers_are_filled_from_each_call() {
    let mut db = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    for migration in BEFORE_WIRE {
        sqlx::raw_sql(sqlx::AssertSqlSafe(migration))
            .execute(&mut db)
            .await
            .unwrap();
    }
    sqlx::raw_sql(
        "INSERT INTO channels(id, transport, opened_at) VALUES ('ch', 'http', 0);
         INSERT INTO calls(id, channel, started_at, provider, endpoint, method, config_revision, attribution, state)
         VALUES ('a', 'ch', 0, 'anthropic', 'messages', 'POST', 'r', '{}', 'finished'),
                ('c', 'ch', 0, 'codex', 'responses', 'POST', 'r', '{}', 'finished'),
                ('o', 'ch', 0, 'openai', 'responses', 'POST', 'r', '{}', 'finished'),
                ('x', 'ch', 0, 'from_a_newer_version', 'messages', 'POST', 'r', '{}', 'finished');
         INSERT INTO attempts(id, call, ordinal, started_at, upstream)
         VALUES ('a0', 'a', 0, 0, 'anthropic'), ('a1', 'a', 1, 0, 'anthropic-key'),
                ('c0', 'c', 0, 0, 'codex'), ('x0', 'x', 0, 0, 'elsewhere');",
    )
    .execute(&mut db)
    .await
    .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(WIRE))
        .execute(&mut db)
        .await
        .unwrap();

    let calls: Vec<(String, Option<String>)> =
        sqlx::query("SELECT id, wire FROM calls ORDER BY id")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .iter()
            .map(|row| (row.get("id"), row.get("wire")))
            .collect();
    let expected = [
        ("a", Some("anthropic_messages")),
        ("c", Some("openai_responses")),
        ("o", Some("openai_responses")),
        ("x", None),
    ];
    assert_eq!(
        calls,
        expected.map(|(id, wire)| (id.to_owned(), wire.map(str::to_owned)))
    );

    let attempts: Vec<(String, Option<String>)> =
        sqlx::query("SELECT id, provider FROM attempts ORDER BY id")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .iter()
            .map(|row| (row.get("id"), row.get("provider")))
            .collect();
    let expected = [
        ("a0", "anthropic"),
        ("a1", "anthropic"),
        ("c0", "codex"),
        ("x0", "from_a_newer_version"),
    ];
    assert_eq!(
        attempts,
        expected.map(|(id, provider)| (id.to_owned(), Some(provider.to_owned())))
    );
}

#[tokio::test]
async fn every_run_gets_an_operator_session_of_its_own() {
    let mut db = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    for migration in [
        include_str!("../migrations/0001_store.sql"),
        include_str!("../migrations/0002_digests.sql"),
        include_str!("../migrations/0003_call_purpose.sql"),
        include_str!("../migrations/0004_wire.sql"),
        include_str!("../migrations/0005_caller.sql"),
        include_str!("../migrations/0006_runs.sql"),
        include_str!("../migrations/0007_archive.sql"),
        include_str!("../migrations/0008_run_entries.sql"),
        include_str!("../migrations/0009_run_settings.sql"),
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(migration))
            .execute(&mut db)
            .await
            .unwrap();
    }
    sqlx::raw_sql(
        "INSERT INTO runs(id, harness, workdir, started_at, model, effort, archived_at)
         VALUES ('r1', 'codex', '/work/acme', 10, 'gpt-deep', 'high', NULL),
                ('r2', 'claude_code', '/work/web', 20, NULL, NULL, 25);",
    )
    .execute(&mut db)
    .await
    .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(include_str!(
        "../migrations/0010_operator_sessions.sql"
    )))
    .execute(&mut db)
    .await
    .unwrap();

    let runs: Vec<(String, Option<String>)> =
        sqlx::query("SELECT id, operator_session FROM runs ORDER BY id")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .iter()
            .map(|row| (row.get("id"), row.get("operator_session")))
            .collect();
    assert_eq!(
        runs,
        [
            ("r1".to_owned(), Some("r1".to_owned())),
            ("r2".to_owned(), Some("r2".to_owned()))
        ]
    );
    type Session = (
        String,
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<i64>,
    );
    let sessions: Vec<Session> = sqlx::query(
        "SELECT id, harness, workdir, created_at, model, effort, archived_at FROM operator_sessions ORDER BY id",
    )
    .fetch_all(&mut db)
    .await
    .unwrap()
    .iter()
    .map(|row| {
        (
            row.get("id"),
            row.get("harness"),
            row.get("workdir"),
            row.get("created_at"),
            row.get("model"),
            row.get("effort"),
            row.get("archived_at"),
        )
    })
    .collect();
    assert_eq!(
        sessions,
        [
            (
                "r1".into(),
                "codex".into(),
                "/work/acme".into(),
                10,
                Some("gpt-deep".into()),
                Some("high".into()),
                None
            ),
            (
                "r2".into(),
                "claude_code".into(),
                "/work/web".into(),
                20,
                None,
                None,
                Some(25)
            ),
        ]
    );
}
