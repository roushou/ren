use super::*;
use uuid::Uuid;

fn call(
    n: u128,
    session: Option<u128>,
    client: Option<Harness>,
    documents: &[(SearchField, &str)],
) -> CallDocuments {
    CallDocuments {
        call: CallId::restore(Uuid::from_u128(n)),
        session: session.map(|n| SessionId::restore(Uuid::from_u128(n))),
        turn: None,
        client,
        started_at: UnixMillis::from_millis(u64::try_from(n).unwrap() * 1000),
        documents: documents
            .iter()
            .map(|(field, text)| Document {
                field: *field,
                text: (*text).to_owned(),
            })
            .collect(),
        exposures: Vec::new(),
    }
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        terms: SearchTerms::parse(text).unwrap(),
        session: None,
        client: None,
        field: None,
        time: TimeRange::default(),
        limit: PageLimit::new(20).unwrap(),
    }
}

#[test]
fn terms_become_quoted_phrases_so_input_never_injects_syntax() {
    assert_eq!(
        SearchTerms::parse("cargo  test").unwrap().expression(),
        r#""cargo" AND "test""#
    );
    assert_eq!(
        SearchTerms::parse(r#"run "install vps" now"#)
            .unwrap()
            .expression(),
        r#""run" AND "install vps" AND "now""#
    );
    // FTS5 operators and stray quotes are plain text inside a phrase.
    assert_eq!(
        SearchTerms::parse(r#"a OR b NEAR(c) col:x *"#)
            .unwrap()
            .expression(),
        r#""a" AND "OR" AND "b" AND "NEAR(c)" AND "col:x" AND "*""#
    );
    assert_eq!(
        SearchTerms::parse(r#"unclosed "quote"#)
            .unwrap()
            .expression(),
        r#""unclosed" AND "quote""#
    );
    let phrase = SearchTerms::parse(r#" run "install vps" "#).unwrap();
    assert_eq!(SearchTerms::parse(&phrase.to_string()), Ok(phrase));
    assert_eq!(SearchTerms::parse("   "), Err(SearchError::Terms));
    assert_eq!(SearchTerms::parse(r#""  ""#), Err(SearchError::Terms));
    assert_eq!(
        SearchTerms::parse(&"x".repeat(SearchTerms::MAX_BYTES + 1)),
        Err(SearchError::Terms)
    );
}

#[tokio::test]
async fn indexed_documents_are_found_filtered_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let index = SearchIndex::open(dir.path()).await.unwrap();
    index
        .add(&[
            call(
                1,
                Some(10),
                Some(Harness::ClaudeCode),
                &[
                    (SearchField::Prompt, "Why does the migration fail?"),
                    (SearchField::ToolInput, "Bash cargo xtask install-vps {}"),
                ],
            ),
            call(
                2,
                Some(20),
                Some(Harness::Codex),
                &[(SearchField::ToolOutput, "test migration ... FAILED")],
            ),
            // A call with nothing to index is still marked as indexed.
            call(3, None, None, &[]),
        ])
        .await
        .unwrap();
    let ids: Vec<_> = [1_u128, 2, 3, 4]
        .iter()
        .map(|n| CallId::restore(Uuid::from_u128(*n)))
        .collect();
    assert_eq!(index.indexed(&ids).await.unwrap().len(), 3);

    let page = index.search(&query("migration")).await.unwrap();
    assert_eq!(page.indexed_calls, 3);
    let calls: Vec<_> = page.hits.iter().map(|hit| hit.call).collect();
    // Newest call first.
    assert_eq!(calls, [ids[1], ids[0]]);
    assert_eq!(page.hits[1].snippet, "Why does the «migration» fail?");
    assert_eq!(page.hits[1].field, Stored::Known(SearchField::Prompt));
    assert_eq!(
        page.hits[1].client,
        Some(Stored::Known(Harness::ClaudeCode))
    );

    // Punctuation splits words: a hyphenated name is found as a phrase.
    let page = index.search(&query(r#""install-vps""#)).await.unwrap();
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].field, Stored::Known(SearchField::ToolInput));

    let mut filtered = query("migration");
    filtered.client = Some(Harness::Codex);
    assert_eq!(index.search(&filtered).await.unwrap().hits[0].call, ids[1]);
    filtered.client = None;
    filtered.field = Some(SearchField::Prompt);
    assert_eq!(index.search(&filtered).await.unwrap().hits[0].call, ids[0]);
    filtered.field = None;
    filtered.session = Some(SessionId::restore(Uuid::from_u128(20)));
    assert_eq!(index.search(&filtered).await.unwrap().hits.len(), 1);
    filtered.session = None;
    filtered.time = TimeRange::new(Some(UnixMillis::from_millis(1_500)), None).unwrap();
    assert_eq!(index.search(&filtered).await.unwrap().hits.len(), 1);

    // An FTS5 operator in the text is searched for, never executed.
    assert!(index.search(&query("NEAR(")).await.unwrap().hits.is_empty());

    index.remove(&ids[..1]).await.unwrap();
    assert_eq!(
        index.search(&query("migration")).await.unwrap().hits.len(),
        1
    );
    assert_eq!(index.indexed_count().await.unwrap(), 2);
    let page = index.indexed_page(None, 10).await.unwrap();
    assert_eq!(page, [ids[1], ids[2]]);
    assert_eq!(
        index.indexed_page(Some(ids[1]), 10).await.unwrap(),
        [ids[2]]
    );
}

#[tokio::test]
async fn another_index_version_is_rebuilt_and_readers_need_a_built_index() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        SearchIndex::open_read(dir.path()).await,
        Err(StoreError::NotFound)
    ));
    let index = SearchIndex::open(dir.path()).await.unwrap();
    index
        .add(&[call(
            1,
            None,
            None,
            &[(SearchField::Reply, "kept until rebuilt")],
        )])
        .await
        .unwrap();
    assert_eq!(
        SearchIndex::open_read(dir.path())
            .await
            .unwrap()
            .indexed_count()
            .await
            .unwrap(),
        1
    );
    sqlx::query("PRAGMA user_version = 999")
        .execute(&index.pool)
        .await
        .unwrap();
    assert!(matches!(
        SearchIndex::open_read(dir.path()).await,
        Err(StoreError::NotFound)
    ));
    let rebuilt = SearchIndex::open(dir.path()).await.unwrap();
    assert_eq!(rebuilt.indexed_count().await.unwrap(), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(SearchIndex::FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the index holds private text");
    }
}

#[tokio::test]
async fn index_usage_budgets_only_the_share_of_calls_the_store_still_has() {
    use std::sync::Arc;
    use tgv_recorder::{
        id::SequentialIds,
        time::{ManualClock, UnixMillis},
    };
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(SearchIndex::usage(dir.path()).await.unwrap(), None);
    let clock = ManualClock::at(UnixMillis::EPOCH);
    let (mut store, _) = Store::open(dir.path(), &clock, Arc::new(SequentialIds::default()))
        .await
        .unwrap();
    let index = SearchIndex::open(dir.path()).await.unwrap();
    // The store has none of these calls: as if retention just deleted them.
    index
        .add(&[
            call(
                1,
                None,
                None,
                &[(SearchField::Reply, &"words ".repeat(2_000))],
            ),
            call(
                2,
                None,
                None,
                &[(SearchField::Reply, &"more ".repeat(2_000))],
            ),
        ])
        .await
        .unwrap();
    let usage = SearchIndex::usage(dir.path()).await.unwrap().unwrap();
    assert_eq!(usage.indexed_calls, 2);
    assert_eq!(usage.stale_calls, 2);
    assert!(usage.file.live_bytes() > 0);
    assert_eq!(usage.budget_bytes(), 0);
    // The store measures it too, and its total leaves the stale rows out.
    let disk = store.disk_usage().await.unwrap();
    assert_eq!(disk.search, Some(usage));
    assert_eq!(
        disk.live_bytes(),
        disk.metadata.live_bytes() + disk.blobs.live_bytes()
    );
}

#[test]
fn the_index_budget_is_its_live_size_in_proportion_to_kept_calls() {
    let usage = |indexed_calls, stale_calls| crate::view::IndexUsage {
        file: crate::view::FileUsage {
            page_size: 4096,
            page_count: 110,
            freelist_count: 10,
        },
        indexed_calls,
        stale_calls,
    };
    assert_eq!(usage(0, 0).budget_bytes(), 409_600);
    assert_eq!(usage(4, 0).budget_bytes(), 409_600);
    assert_eq!(usage(4, 1).budget_bytes(), 307_200);
    assert_eq!(usage(4, 4).budget_bytes(), 0);
    assert_eq!(usage(4, 9).budget_bytes(), 0);
}

#[tokio::test]
async fn exposures_are_listed_newest_first_and_removed_with_their_call() {
    use crate::secrets::SecretKind;
    let dir = tempfile::tempdir().unwrap();
    let index = SearchIndex::open(dir.path()).await.unwrap();
    let exposure = |field| Exposure {
        field,
        kind: SecretKind::GithubToken,
        hint: "ghp_…wxyz".to_owned(),
        fingerprint: "0123456789abcdef".to_owned(),
        count: 2,
    };
    let mut first = call(1, Some(10), Some(Harness::Codex), &[]);
    first.exposures = vec![exposure(SearchField::ToolOutput)];
    let mut second = call(2, Some(20), None, &[]);
    second.exposures = vec![exposure(SearchField::Prompt)];
    index.add(&[first, second]).await.unwrap();
    let all = ExposureQuery {
        session: None,
        time: TimeRange::default(),
        limit: PageLimit::new(10).unwrap(),
    };
    let listed = index.exposures(&all).await.unwrap();
    let calls: Vec<_> = listed.iter().map(|exposure| exposure.call).collect();
    assert_eq!(
        calls,
        [
            CallId::restore(Uuid::from_u128(2)),
            CallId::restore(Uuid::from_u128(1))
        ]
    );
    assert_eq!(listed[1].kind, Stored::Known(SecretKind::GithubToken));
    assert_eq!(listed[1].count, 2);
    let mut one = all.clone();
    one.session = Some(SessionId::restore(Uuid::from_u128(10)));
    assert_eq!(index.exposures(&one).await.unwrap().len(), 1);
    index
        .remove(&[CallId::restore(Uuid::from_u128(1))])
        .await
        .unwrap();
    assert_eq!(index.exposures(&all).await.unwrap().len(), 1);
}
