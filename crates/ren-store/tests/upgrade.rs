//! The store's compatibility contract. A shipped migration never changes:
//! each is pinned by the checksum an existing store holds for it
//! (`fixtures/migrations.txt`). Each migration also has a store fixture: the
//! rows a store held at that migration (`fixtures/stores/<version>.sql`),
//! written once, through the recorder, by `write_the_newest_store_fixture`.
//! Every fixture must open with today's migrations and serve every read, so
//! a new migration is checked against stores every earlier release wrote.
//! `cargo xtask compat` refuses changes to either fixture unless the change
//! says it breaks compatibility.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use ren_harness::{
    Harness,
    attachment::{Attachment, Limits},
    attribution::{Attribution, HintSource, SessionHint},
    drive::{
        CommandIntent, DriveEvent, DriveItem, ItemKind, ItemPhase, ItemRead, ItemStatus, TurnEnd,
    },
    settings::{ApprovalPolicy, Effort, RunSettings},
};
use ren_protocol::{
    direction::Direction,
    encoding::{ResponseEncoding, ResponseFormat},
    label::Stored,
    native::{NativeSessionId, NativeThreadId, NativeTurnId},
    route::{Endpoint, Method, Provider, Transport},
    websocket::Payload,
};
use ren_recorder::{
    AttemptStart, AttemptTap, CallStart, CaptureOptions, Head, Recorder, RecorderSetup, RunSession,
    batch::{Actor, BodyKey, HistoryChange, HistorySubject, SessionFact},
    capture::{
        AttemptPhase, AttemptTerminal, CallRejection, ChannelEnd, ProcessExit, RunEnd,
        TerminalCause, TerminalRecord,
    },
    id::{AttachmentId, ChannelId, ProjectId, RunId, SequentialIds, SessionId},
    retention::RetentionPolicy,
    routing::{ConfigRevision, OperatorRevision, UpstreamId},
    secrets::SecretScanner,
    time::{ManualClock, UnixMillis},
};
use ren_store::{
    Reader, Store,
    query::{CallQuery, PageLimit, Shelf, TimeRange},
    read::{ReadRequest, ReadResponse},
    search::{ExposureQuery, SearchQuery},
};
use sqlx::{
    AssertSqlSafe, Connection as _, SqliteConnection, migrate::Migrator,
    sqlite::SqliteConnectOptions,
};
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// The pinned listing of the migrations: one line per migration, its
/// version, description, and checksum.
const PINNED: &str = include_str!("fixtures/migrations.txt");

/// One line of the pinned listing.
fn pin(version: i64, description: &str, checksum: &[u8]) -> String {
    let mut line = format!("{version:04} {description} ");
    for byte in checksum {
        write!(line, "{byte:02x}").unwrap();
    }
    line
}

fn listing() -> String {
    MIGRATOR
        .iter()
        .map(|migration| {
            pin(
                migration.version,
                &migration.description,
                &migration.checksum,
            ) + "\n"
        })
        .collect()
}

fn fixture(version: i64) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/stores")
        .join(format!("{version:04}.sql"))
}

#[test]
fn every_migration_is_pinned() {
    assert_eq!(
        listing(),
        PINNED,
        "a shipped migration never changes; a new one is pinned with its checksum"
    );
}

#[test]
fn every_migration_has_a_store_fixture() {
    for migration in MIGRATOR.iter() {
        let path = fixture(migration.version);
        assert!(
            path.is_file(),
            "{} is missing: write it with `cargo test -p ren-store --test upgrade -- --ignored`",
            path.display()
        );
    }
}

#[tokio::test]
async fn every_store_fixture_opens_and_serves_every_read() {
    for migration in MIGRATOR.iter() {
        let rows = std::fs::read_to_string(fixture(migration.version)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        StoreAt::write(dir.path(), migration.version, &rows).await;
        let clock = ManualClock::at(UnixMillis::from_millis(START + 3_600_000));
        let (store, _) = Store::open(
            dir.path(),
            &clock,
            Arc::new(SequentialIds::starting_at(1_000_000)),
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "the store at migration {} does not open: {error}",
                migration.version
            )
        });
        store.close().await.unwrap();
        let reader = Reader::open(dir.path(), 1).await.unwrap();
        Reads::every(&reader).await;
        reader.close().await;
    }
}

/// Write the store fixture of the newest migration, when it has none. Run it
/// once when adding a migration, and never again after it ships:
/// `cargo test -p ren-store --test upgrade -- --ignored`.
#[tokio::test]
#[ignore = "writes a fixture; run once for a new migration"]
async fn write_the_newest_store_fixture() {
    let newest = MIGRATOR.iter().last().unwrap().version;
    let path = fixture(newest);
    assert!(
        !path.exists(),
        "{} exists; a fixture is written once",
        path.display()
    );
    let dir = tempfile::tempdir().unwrap();
    Scenario::record(dir.path()).await;
    let rows = StoreAt::dump(dir.path(), newest).await;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, rows).unwrap();
}

/// When the scenario starts: 2026-01-05, 09:00 UTC.
const START: u64 = 1_767_603_600_000;

/// A store's rows at one migration, as SQL.
struct StoreAt;

impl StoreAt {
    /// Make a store in `dir` at `version` and load `rows` into it.
    async fn write(dir: &Path, version: i64, rows: &str) {
        let options = SqliteConnectOptions::new()
            .filename(dir.join(Store::METADATA_FILE))
            .create_if_missing(true)
            .foreign_keys(true);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        MIGRATOR.run_to(version, &mut connection).await.unwrap();
        Self::attach_blobs(&mut connection, dir).await;
        sqlx::raw_sql(AssertSqlSafe(rows.to_owned()))
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
    }

    /// The rows of the store in `dir`, written at `version`, as SQL that
    /// [`StoreAt::write`] loads: every table's rows in creation order, and
    /// the blob store's tables whole. The search index is left out, as its
    /// triggers fill it again.
    async fn dump(dir: &Path, version: i64) -> String {
        let options = SqliteConnectOptions::new()
            .filename(dir.join(Store::METADATA_FILE))
            .read_only(true);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        Self::attach_blobs(&mut connection, dir).await;
        let mut out = format!(
            "-- The rows of a Ren store at migration {version:04}, recorded by\n\
             -- `write_the_newest_store_fixture` (tests/upgrade.rs). Frozen once\n\
             -- its migration is on main: today's store must open it.\n"
        );
        let page_size: i64 = sqlx::query_scalar("PRAGMA blobs.page_size")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        writeln!(out, "\nPRAGMA blobs.page_size = {page_size};").unwrap();
        let blob_tables: Vec<String> = sqlx::query_scalar(
            "SELECT sql FROM blobs.sqlite_master WHERE type = 'table' ORDER BY rowid",
        )
        .fetch_all(&mut connection)
        .await
        .unwrap();
        for sql in blob_tables {
            let sql = sql
                .strip_prefix("CREATE TABLE ")
                .expect("a table's definition");
            writeln!(out, "CREATE TABLE blobs.{sql};").unwrap();
        }
        for schema in ["main", "blobs"] {
            let tables: Vec<(String, i64)> = sqlx::query_as(AssertSqlSafe(format!(
                "SELECT m.name, t.wr FROM {schema}.sqlite_master m \
                 JOIN pragma_table_list t ON t.schema = '{schema}' AND t.name = m.name \
                 WHERE m.type = 'table' AND t.type = 'table' \
                 AND m.name NOT LIKE 'sqlite_%' AND m.name <> '_sqlx_migrations' \
                 ORDER BY m.rowid"
            )))
            .fetch_all(&mut connection)
            .await
            .unwrap();
            for (table, without_rowid) in tables {
                Self::table(&mut connection, &mut out, schema, &table, without_rowid).await;
            }
        }
        connection.close().await.unwrap();
        out
    }

    async fn table(
        connection: &mut SqliteConnection,
        out: &mut String,
        schema: &str,
        table: &str,
        without_rowid: i64,
    ) {
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?1, ?2) ORDER BY cid")
                .bind(table)
                .bind(schema)
                .fetch_all(&mut *connection)
                .await
                .unwrap();
        let names = columns
            .iter()
            .map(|column| format!("\"{column}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let values = columns
            .iter()
            .map(|column| format!("quote(\"{column}\")"))
            .collect::<Vec<_>>()
            .join(" || ', ' || ");
        let order = if without_rowid == 0 { "rowid" } else { "1" };
        let rows: Vec<String> = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT {values} FROM {schema}.\"{table}\" ORDER BY {order}"
        )))
        .fetch_all(&mut *connection)
        .await
        .unwrap();
        if rows.is_empty() {
            return;
        }
        let target = match schema {
            "main" => format!("\"{table}\""),
            other => format!("{other}.\"{table}\""),
        };
        writeln!(out, "\n-- {schema}.{table}").unwrap();
        for row in rows {
            writeln!(out, "INSERT INTO {target} ({names}) VALUES ({row});").unwrap();
        }
    }

    async fn attach_blobs(connection: &mut SqliteConnection, dir: &Path) {
        let path = dir.join("blobs.sqlite3");
        sqlx::query("ATTACH DATABASE ? AS blobs")
            .bind(path.to_str().unwrap())
            .execute(&mut *connection)
            .await
            .unwrap();
    }
}

/// Every read the API serves, over a store, each expected to answer and to
/// find what the scenario recorded.
struct Reads;

impl Reads {
    async fn every(reader: &Reader) {
        let serve = |request: ReadRequest| async move {
            reader
                .serve(&request)
                .await
                .unwrap_or_else(|error| panic!("{request:?}: {error}"))
        };
        let ReadResponse::Calls(page) = serve(ReadRequest::Calls(CallQuery::default())).await
        else {
            panic!("calls");
        };
        assert!(!page.calls.is_empty(), "calls");
        for call in &page.calls {
            serve(ReadRequest::Call(call.id)).await;
            serve(ReadRequest::CallActivity(call.id)).await;
            let mut body = Vec::new();
            reader
                .export(BodyKey::Request(call.id), |bytes| {
                    body.extend_from_slice(bytes);
                    Ok(())
                })
                .await
                .unwrap();
        }
        let mut recordings = Vec::new();
        for shelf in [Shelf::Current, Shelf::Archived] {
            let ReadResponse::Recordings { recordings: listed } = serve(ReadRequest::Recordings {
                limit: PageLimit::DEFAULT,
                shelf,
            })
            .await
            else {
                panic!("recordings");
            };
            recordings.extend(listed);
        }
        assert!(!recordings.is_empty(), "recordings");
        for recording in &recordings {
            serve(ReadRequest::RecordingCalls {
                recording: recording.id,
                limit: PageLimit::DEFAULT,
                after: None,
            })
            .await;
            serve(ReadRequest::Report(recording.id)).await;
            let ReadResponse::Turns { turns } = serve(ReadRequest::Turns {
                recording: recording.id,
                limit: PageLimit::DEFAULT,
            })
            .await
            else {
                panic!("turns");
            };
            for turn in turns {
                serve(ReadRequest::Turn(turn.turn)).await;
            }
        }
        serve(ReadRequest::Usage(TimeRange::default())).await;
        let ReadResponse::Search(hits) = serve(ReadRequest::Search(SearchQuery {
            terms: "migration".parse().unwrap(),
            recording: None,
            client: None,
            field: None,
            time: TimeRange::default(),
            limit: PageLimit::DEFAULT,
        }))
        .await
        else {
            panic!("search");
        };
        assert!(!hits.hits.is_empty(), "search");
        let ReadResponse::Exposures { exposures } = serve(ReadRequest::Exposures(ExposureQuery {
            recording: None,
            time: TimeRange::default(),
            limit: PageLimit::DEFAULT,
        }))
        .await
        else {
            panic!("exposures");
        };
        assert!(!exposures.is_empty(), "exposures");
        let mut runs = Vec::new();
        for shelf in [Shelf::Current, Shelf::Archived] {
            let ReadResponse::Runs { runs: listed } = serve(ReadRequest::Runs {
                limit: PageLimit::DEFAULT,
                shelf,
                project: None,
            })
            .await
            else {
                panic!("runs");
            };
            runs.extend(listed);
        }
        assert!(!runs.is_empty(), "runs");
        for run in &runs {
            serve(ReadRequest::Run(run.id)).await;
            serve(ReadRequest::Conversation(run.id)).await;
        }
        let mut sessions = Vec::new();
        for shelf in [Shelf::Current, Shelf::Archived] {
            sessions.extend(
                reader
                    .sessions(PageLimit::DEFAULT, shelf, None)
                    .await
                    .unwrap(),
            );
        }
        assert!(!sessions.is_empty(), "sessions");
        let mut attachments = 0;
        for session in &sessions {
            reader.session(session.id).await.unwrap().unwrap();
            attachments += reader.attachments(session.id).await.unwrap().len();
        }
        assert!(attachments > 0, "attachments");
        // Stores before work items hold none.
        for shelf in [Shelf::Current, Shelf::Archived] {
            let ReadResponse::WorkItems { work } = serve(ReadRequest::WorkItems {
                limit: PageLimit::DEFAULT,
                shelf,
                project: None,
            })
            .await
            else {
                panic!("work items");
            };
            for item in &work {
                serve(ReadRequest::WorkItem(item.id)).await;
                assert!(item.state().is_some(), "work item state");
            }
        }
        // Stores before projects hold none.
        for project in reader.projects().await.unwrap() {
            let named = reader.project_named(&project.name).await.unwrap();
            assert_eq!(named.map(|named| named.id), Some(project.id));
            assert!(project.factory.known().is_some(), "project factory");
        }
        reader
            .observations_after(0, PageLimit::DEFAULT)
            .await
            .unwrap();
        reader.storage().await.unwrap();
    }
}

/// What the fixture holds: a session whose run is driven through a turn,
/// with the turn's model calls (one carrying a secret), a WebSocket call, a
/// rate-limited call, an interrupted one, a refused one, an image attached,
/// an operator action, a second run archived, work items, and two
/// projects, one archived.
struct Scenario {
    clock: Arc<ManualClock>,
    ids: Arc<SequentialIds>,
    recorder: Recorder,
}

impl Scenario {
    async fn record(dir: &Path) {
        let clock = Arc::new(ManualClock::at(UnixMillis::from_millis(START)));
        let ids = Arc::new(SequentialIds::default());
        let (store, recovery) = Store::open(dir, clock.as_ref(), ids.clone()).await.unwrap();
        let setup = RecorderSetup {
            options: CaptureOptions::default(),
            retention: RetentionPolicy::default(),
            max_calls: 64,
            live_buffer: 64,
            scanner: Arc::new(SecretScanner::new().unwrap()),
        };
        let (recorder, task) =
            Recorder::new(store, recovery.calls, setup, clock.clone(), ids.clone()).unwrap();
        let scenario = Self {
            clock,
            ids,
            recorder,
        };
        scenario.play();
        drop(scenario);
        task.run().await.unwrap();
    }

    fn later(&self, millis: u64) {
        self.clock.advance(Duration::from_millis(millis));
    }

    fn play(&self) {
        let session = SessionId::generate(self.ids.as_ref());
        let run = RunId::generate(self.ids.as_ref());
        let tap = self
            .recorder
            .run(
                run,
                RunSession::New(session),
                Harness::ClaudeCode,
                "/work/acme-api".into(),
                RunSettings {
                    model: Some("claude-sonnet-4-5".parse().unwrap()),
                    effort: Some(Effort::High),
                    approvals: Some(ApprovalPolicy::Edits),
                },
                Some(ren_depot::toolset::Generation::from(1)),
            )
            .unwrap();
        self.attach(session);
        let thread = NativeThreadId::new(uuid::Uuid::from_u128(0x7a11)).unwrap();
        let turn = NativeTurnId::new(uuid::Uuid::from_u128(0x7a12)).unwrap();
        tap.opened(thread);
        tap.entry(None, DriveEvent::Ready { thread });
        self.later(1_000);
        tap.entry(Some(turn), DriveEvent::TurnStarted { turn });
        tap.entry(
            Some(turn),
            Self::item(
                "prompt_1",
                ItemKind::UserMessage,
                ItemRead::Prompt {
                    text: "Why does the orders migration fail on an empty table?".into(),
                    images: Vec::new(),
                },
            ),
        );
        let hint = SessionHint {
            source: HintSource::ClaudeCodeHeaders,
            session: NativeSessionId::new(thread.as_uuid()).unwrap(),
            turn: Some(turn),
            thread: Some(thread),
        };
        self.later(500);
        self.messages(Some(hint), 0);
        self.later(2_000);
        tap.entry(
            Some(turn),
            Self::item(
                "command_1",
                ItemKind::CommandExecution,
                ItemRead::Command {
                    command: "cargo test -p orders migration".into(),
                    intent: CommandIntent::default(),
                    status: Stored::Known(ItemStatus::Completed),
                    output: Some("test migration_on_empty_table ... ok".into()),
                    exit_code: Some(0),
                },
            ),
        );
        self.later(500);
        self.messages(Some(hint), 1);
        tap.entry(
            Some(turn),
            Self::item(
                "reply_1",
                ItemKind::AgentMessage,
                ItemRead::Reply {
                    text:
                        "The migration read the first row unconditionally; it now checks for one."
                            .into(),
                },
            ),
        );
        tap.entry(
            Some(turn),
            DriveEvent::TurnEnded {
                turn,
                end: TurnEnd::Completed,
            },
        );
        tap.configured(RunSettings {
            model: Some("claude-opus-4-1".parse().unwrap()),
            effort: Some(Effort::Medium),
            approvals: Some(ApprovalPolicy::Edits),
        });
        self.later(1_000);
        self.websocket();
        self.later(1_000);
        self.rate_limited();
        self.later(1_000);
        self.interrupted();
        self.later(1_000);
        self.refused();
        self.later(1_000);
        self.operator();
        self.later(60_000);
        tap.end(RunEnd::Stopped, Some(ProcessExit::Code(0)));
        self.archived_run(session);
        self.later(1_000);
        let ren = self.projects();
        self.later(1_000);
        self.work(session, ren);
    }

    /// Two projects registered, one of them archived; returns the one
    /// that is not.
    fn projects(&self) -> ProjectId {
        use ren_recorder::{
            batch::ProjectFact,
            project::{Factory, ProjectRegistration},
        };
        let register = |name: &str, repository: &str, factory: Factory| {
            self.recorder
                .project_slot()
                .unwrap()
                .register(ProjectRegistration {
                    name: name.parse().unwrap(),
                    repository: repository.parse().unwrap(),
                    factory,
                })
        };
        let ren = register("ren", "/home/op/ren", Factory::Ren);
        self.later(1_000);
        let api = register("acme-api", "/home/op/dev/acme-api", Factory::Contract);
        self.later(1_000);
        self.recorder
            .project_slot()
            .unwrap()
            .record(api, ProjectFact::Archived);
        ren
    }

    /// Two work items: one carried from queued to shipped by the session,
    /// one dismissed.
    fn work(&self, session: SessionId, ren: ProjectId) {
        use ren_recorder::batch::WorkFact;
        use ren_recorder::id::WorkItemId;
        use ren_recorder::work::{Evidence, MarkedPage, PageTheme, WorkMark, WorkOrder};
        let order = |note: &str| WorkOrder {
            page: MarkedPage {
                path: "/recordings".into(),
                route: Some("/recordings".into()),
                title: "Recordings".into(),
                width: 1440,
                height: 900,
                theme: PageTheme::Light,
                revision: Some("62e9e9e0000000000000000000000000000000ff".parse().unwrap()),
            },
            marks: vec![WorkMark {
                selector: "[aria-label=\"Recordings\"] table > tr".into(),
                text: "acme-api".into(),
                source: Some("src/routes/recordings/+page.svelte:42".into()),
                width: 600,
                height: 24,
                note: note.into(),
            }],
            evidence: Evidence {
                console: vec!["TypeError: turns is undefined".into()],
                reads: vec!["/api/recordings?limit=100".into()],
            },
        };
        let record = |item: WorkItemId, fact: WorkFact| {
            self.recorder.work_slot().unwrap().record(item, fact);
        };
        // The session works for ren; the dismissed item names no project.
        self.recorder
            .session_slot()
            .unwrap()
            .record(session, SessionFact::Assigned { project: Some(ren) });
        let shipped = WorkItemId::generate(self.ids.as_ref());
        record(
            shipped,
            WorkFact::Queued {
                order: Box::new(order("Show each recording's turn count")),
                project: Some(ren),
            },
        );
        self.later(1_000);
        record(shipped, WorkFact::Sent { session });
        self.later(1_000);
        record(
            shipped,
            WorkFact::Previewed {
                preview: Some(ren_recorder::work::WorkPreview {
                    ui: "https://ren.example.ts.net:20201".parse().unwrap(),
                    revision: "07f4bb7c1e2a3b4c5d6e7f8091a2b3c4d5e6f708-dirty"
                        .parse()
                        .unwrap(),
                }),
            },
        );
        self.later(1_000);
        let commit = "07f4bb7c1e2a3b4c5d6e7f8091a2b3c4d5e6f708";
        record(
            shipped,
            WorkFact::Landed {
                commit: commit.parse().unwrap(),
            },
        );
        self.later(1_000);
        record(
            shipped,
            WorkFact::Shipped {
                revision: commit.parse().unwrap(),
            },
        );
        let dismissed = WorkItemId::generate(self.ids.as_ref());
        record(
            dismissed,
            WorkFact::Queued {
                order: Box::new(order("Move the archive button")),
                project: None,
            },
        );
        self.later(1_000);
        record(dismissed, WorkFact::Dismissed);
    }

    fn item(id: &str, kind: ItemKind, read: ItemRead) -> DriveEvent {
        DriveEvent::Item {
            phase: ItemPhase::Completed,
            item: DriveItem {
                id: serde_json::from_value(serde_json::json!(id)).unwrap(),
                kind: kind.into(),
                read,
                body: serde_json::json!({ "id": id }),
            },
        }
    }

    fn attach(&self, session: SessionId) {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(640u32.to_be_bytes());
        png.extend(480u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        let read = Attachment::parse("orders-error.png", &png, &Limits::default()).unwrap();
        self.recorder.session_slot().unwrap().record(
            session,
            SessionFact::Attached {
                attachment: AttachmentId::generate(self.ids.as_ref()),
                read,
            },
        );
    }

    fn start(&self, api: Provider, hint: Option<SessionHint>) -> AttemptTap {
        self.recorder.start(
            CallStart {
                transport: Transport::Http,
                api: api.native_api(),
                endpoint: Endpoint::Messages,
                method: Method::Post,
                config_revision: ConfigRevision::new("cfg-3f2a"),
                attribution: Attribution {
                    hint,
                    issues: Vec::new(),
                },
                caller: ren_recorder::caller::Caller::Unidentified,
                operator_revision: None,
                plan: None,
                attempts: 1,
            },
            AttemptStart::unchanged(UpstreamId::default_for(api), api),
        )
    }

    /// One of the turn's model calls: the first carries a secret and asks
    /// for a tool; the second answers.
    fn messages(&self, hint: Option<SessionHint>, step: usize) {
        let key = format!("sk-ant-api03-{}", "Fx7Qm2Lp".repeat(12));
        let request = serde_json::json!({
            "model": "claude-sonnet-4-5",
            "stream": true,
            "messages": [
                { "role": "user", "content": format!(
                    "Why does the orders migration fail on an empty table? The staging key is {key}."
                ) },
            ],
        });
        let event =
            |name: &str, data: serde_json::Value| format!("event: {name}\ndata: {data}\n\n");
        let block = match step {
            0 => vec![
                event(
                    "content_block_start",
                    serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "tool_use", "id": "toolu_01", "name": "Bash", "input": {} } }),
                ),
                event(
                    "content_block_delta",
                    serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "input_json_delta", "partial_json": "{\"command\":\"cargo test -p orders migration\"}" } }),
                ),
                event(
                    "content_block_stop",
                    serde_json::json!({ "type": "content_block_stop", "index": 0 }),
                ),
                event(
                    "message_delta",
                    serde_json::json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 41 } }),
                ),
            ],
            _ => vec![
                event(
                    "content_block_start",
                    serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
                ),
                event(
                    "content_block_delta",
                    serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "The migration read the first row unconditionally; it now checks for one." } }),
                ),
                event(
                    "content_block_stop",
                    serde_json::json!({ "type": "content_block_stop", "index": 0 }),
                ),
                event(
                    "message_delta",
                    serde_json::json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 23 } }),
                ),
            ],
        };
        let response = [
            vec![event("message_start", serde_json::json!({ "type": "message_start", "message": { "id": format!("msg_0{step}"), "model": "claude-sonnet-4-5", "usage": { "input_tokens": 1830, "cache_read_input_tokens": 1200, "output_tokens": 1 } } }))],
            block,
            vec![event("message_stop", serde_json::json!({ "type": "message_stop" }))],
        ]
        .concat()
        .concat();
        let mut tap = self.start(Provider::Anthropic, hint);
        tap.request_data(request.to_string().as_bytes());
        tap.request_ended();
        let mut tap = tap.head(Head {
            status: 200,
            format: ResponseFormat::EventStream,
            encoding: ResponseEncoding::Identity,
        });
        tap.data(Direction::Response, response.as_bytes());
        tap.ended(Direction::Response);
        tap.finish(TerminalRecord {
            terminal: AttemptTerminal::TerminalEvent,
            cause: None,
            phase: Some(AttemptPhase::Response),
        });
    }

    /// A Codex call over a WebSocket: one message each way.
    fn websocket(&self) {
        let channel = self
            .recorder
            .channel(ChannelId::generate(self.ids.as_ref()));
        let thread = NativeThreadId::new(uuid::Uuid::from_u128(0xc0de)).unwrap();
        let mut tap = self.recorder.start_in(
            channel.channel(),
            CallStart {
                transport: Transport::WebSocket,
                api: Provider::OpenAi.native_api(),
                endpoint: Endpoint::Responses,
                method: Method::Message,
                config_revision: ConfigRevision::new("cfg-3f2a"),
                attribution: Attribution {
                    hint: Some(SessionHint {
                        source: HintSource::CodexHeaders,
                        session: NativeSessionId::new(thread.as_uuid()).unwrap(),
                        turn: None,
                        thread: Some(thread),
                    }),
                    issues: Vec::new(),
                },
                caller: ren_recorder::caller::Caller::Unidentified,
                operator_revision: None,
                plan: None,
                attempts: 1,
            },
            AttemptStart::unchanged(UpstreamId::default_for(Provider::OpenAi), Provider::OpenAi),
        );
        let create = serde_json::json!({ "type": "response.create", "model": "gpt-5-codex", "input": [
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Add a test for the orders migration" }] },
        ] });
        tap.request_message(Payload::Text, create.to_string().as_bytes());
        tap.request_ended();
        let mut tap = tap.head(Head {
            status: 101,
            format: ResponseFormat::WebSocketMessages,
            encoding: ResponseEncoding::Identity,
        });
        let item = serde_json::json!({ "type": "function_call", "call_id": "call_01", "name": "shell", "arguments": "{\"command\":[\"cargo\",\"test\"]}" });
        for event in [
            serde_json::json!({ "type": "response.created", "response": { "id": "resp_01", "model": "gpt-5-codex" } }),
            serde_json::json!({ "type": "response.output_item.done", "item": item }),
            serde_json::json!({ "type": "response.completed", "response": { "id": "resp_01", "model": "gpt-5-codex", "output": [item], "usage": { "input_tokens": 2400, "output_tokens": 57 } } }),
        ] {
            tap.message(
                Direction::Response,
                Payload::Text,
                event.to_string().as_bytes(),
            );
        }
        tap.ended(Direction::Response);
        tap.finish(TerminalRecord {
            terminal: AttemptTerminal::TerminalEvent,
            cause: None,
            phase: Some(AttemptPhase::Response),
        });
        channel.close(ChannelEnd::ClientClosed, Some(1000));
    }

    /// A call the provider refused with `429`.
    fn rate_limited(&self) {
        let mut tap = self.start(Provider::Anthropic, None);
        tap.request_data(br#"{"model":"claude-sonnet-4-5","messages":[{"role":"user","content":"Summarise the migration log"}]}"#);
        tap.request_ended();
        let mut tap = tap.head(Head {
            status: 429,
            format: ResponseFormat::Json,
            encoding: ResponseEncoding::Identity,
        });
        tap.data(
            Direction::Response,
            br#"{"type":"error","error":{"type":"rate_limit_error","message":"Number of request tokens has exceeded your per-minute rate limit"}}"#,
        );
        tap.ended(Direction::Response);
        tap.finish(TerminalRecord {
            terminal: AttemptTerminal::BodyComplete,
            cause: None,
            phase: Some(AttemptPhase::Response),
        });
    }

    /// A call whose stream the client cut off.
    fn interrupted(&self) {
        let mut tap = self.start(Provider::Anthropic, None);
        tap.request_data(br#"{"model":"claude-sonnet-4-5","stream":true,"messages":[{"role":"user","content":"Explain the migration plan"}]}"#);
        tap.request_ended();
        let mut tap = tap.head(Head {
            status: 200,
            format: ResponseFormat::EventStream,
            encoding: ResponseEncoding::Identity,
        });
        tap.data(
            Direction::Response,
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-5\"}}\n\nevent: content_",
        );
        tap.finish(TerminalRecord {
            terminal: AttemptTerminal::Cancelled,
            cause: Some(TerminalCause::Client),
            phase: Some(AttemptPhase::Response),
        });
    }

    /// A call Ren answered itself while the operator had paused new calls.
    fn refused(&self) {
        self.recorder.reject(
            None,
            CallStart {
                transport: Transport::Http,
                api: Provider::Anthropic.native_api(),
                endpoint: Endpoint::Messages,
                method: Method::Post,
                config_revision: ConfigRevision::new("cfg-3f2a"),
                attribution: Attribution::default(),
                caller: ren_recorder::caller::Caller::Unidentified,
                operator_revision: Some(OperatorRevision::restore(1)),
                plan: None,
                attempts: 1,
            },
            Some(&bytes::Bytes::from_static(
                br#"{"model":"claude-sonnet-4-5","messages":[{"role":"user","content":"Retry the migration"}]}"#,
            )),
            CallRejection::Paused,
        );
    }

    /// The operator paused new calls.
    fn operator(&self) {
        self.recorder
            .operator_slot()
            .unwrap()
            .record(
                OperatorRevision::restore(1),
                Actor::Loopback,
                &serde_json::json!({ "command": "pause" }),
                &serde_json::json!({ "admission": "paused" }),
            )
            .unwrap();
    }

    /// A second run of the session, ended at once and archived.
    fn archived_run(&self, session: SessionId) {
        let run = RunId::generate(self.ids.as_ref());
        let tap = self
            .recorder
            .run(
                run,
                RunSession::Existing(session),
                Harness::ClaudeCode,
                "/work/acme-api".into(),
                RunSettings::default(),
                Some(ren_depot::toolset::Generation::from(1)),
            )
            .unwrap();
        self.later(5_000);
        tap.end(RunEnd::Exited, Some(ProcessExit::Code(1)));
        self.recorder.history_slot().unwrap().record(
            Actor::Loopback,
            HistorySubject::Run(run),
            HistoryChange::Archive,
        );
    }
}
