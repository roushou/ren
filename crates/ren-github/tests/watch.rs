//! A watch reads GitHub's answer into a snapshot, and the client and the
//! poller read it from a loopback stand-in for GitHub's endpoint:
//! synthetic answers, in the shape the query asks for.
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use ren_github::{
    CheckState, ChecksState, Listing, MergeState, ReadError, Readiness, Reported, ReviewDecision,
    Watch,
};

fn watch() -> Watch {
    Watch::new("acme/web".parse().unwrap())
}

#[test]
fn the_query_names_the_repository() {
    let query: serde_json::Value = serde_json::from_slice(&watch().query()).unwrap();
    assert_eq!(query["variables"]["owner"], "acme");
    assert_eq!(query["variables"]["name"], "web");
    assert_eq!(query["variables"]["manifest"], "HEAD:.ren/project.toml");
    assert!(
        query["query"]
            .as_str()
            .unwrap()
            .contains("pullRequests(states: OPEN")
    );
}

#[test]
fn an_answer_reads_into_a_snapshot() {
    let snapshot = watch()
        .read(include_bytes!("fixtures/answer.json"))
        .unwrap();
    let (branch, head) = snapshot.default_branch.unwrap();
    assert_eq!((branch.as_str(), head.short()), ("main", "111111111111"));
    assert_eq!(snapshot.listing, Listing::Complete);
    assert_eq!(
        snapshot.manifest.as_deref(),
        Some("reach = [\"registry.npmjs.org:443\"]\n")
    );

    let [landing, layout] = snapshot.pull_requests.as_slice() else {
        panic!("two pull requests: {:?}", snapshot.pull_requests);
    };
    assert_eq!(landing.number.get(), 12);
    assert_eq!(landing.author.as_ref().unwrap().as_str(), "acme-bot");
    assert_eq!(landing.readiness, Readiness::Ready);
    assert_eq!(landing.head_branch, "landing");
    assert_eq!(landing.merge, Reported::Known(MergeState::Behind));
    assert_eq!(landing.review, Reported::Known(ReviewDecision::NotRequired));
    assert_eq!(landing.checks, Reported::Known(ChecksState::Failing));
    let states: Vec<_> = landing
        .check_list
        .iter()
        .map(|check| (check.name.as_str(), check.state.known().unwrap()))
        .collect();
    assert_eq!(
        states,
        [
            ("CI", CheckState::Passed),
            ("Compatibility", CheckState::Failed),
            ("lint", CheckState::Running),
            ("deploy/preview", CheckState::Running),
        ]
    );

    assert_eq!(layout.readiness, Readiness::Draft);
    assert_eq!(layout.author, None);
    assert_eq!(layout.merge.label(), "SOMETHING_NEW");
    assert_eq!(
        layout.review,
        Reported::Known(ReviewDecision::ReviewRequired)
    );
    assert_eq!(layout.checks, Reported::Known(ChecksState::None));
    assert!(layout.check_list.is_empty());
}

#[test]
fn a_manifest_cut_short_or_missing_is_none() {
    let answer = include_str!("fixtures/answer.json");
    for changed in [
        answer.replacen("\"isTruncated\": false", "\"isTruncated\": true", 1),
        answer.replacen(
            "\"manifest\": { \"text\": \"reach = [\\\"registry.npmjs.org:443\\\"]\\n\", \"isTruncated\": false },",
            "\"manifest\": null,",
            1,
        ),
        answer.replacen(
            "\"manifest\": { \"text\": \"reach = [\\\"registry.npmjs.org:443\\\"]\\n\", \"isTruncated\": false },",
            "",
            1,
        ),
    ] {
        assert_ne!(changed, answer);
        assert_eq!(watch().read(changed.as_bytes()).unwrap().manifest, None);
    }
}

#[test]
fn a_cut_listing_says_so() {
    let answer = include_str!("fixtures/answer.json").replacen(
        "\"hasNextPage\": false",
        "\"hasNextPage\": true",
        1,
    );
    let snapshot = watch().read(answer.as_bytes()).unwrap();
    assert_eq!(snapshot.listing, Listing::Truncated);
}

#[test]
fn refusals_and_missing_repositories_are_errors() {
    let refused = br#"{"data": null, "errors": [{"message": "Bad credentials"}]}"#;
    assert_eq!(
        watch().read(refused),
        Err(ReadError::Refused(vec!["Bad credentials".into()]))
    );
    let missing = br#"{"data": {"repository": null}}"#;
    assert!(matches!(
        watch().read(missing),
        Err(ReadError::NoRepository(_))
    ));
    assert!(matches!(watch().read(b"<html>"), Err(ReadError::Shape(_))));
}

#[test]
fn a_head_that_is_not_a_commit_is_refused() {
    let answer = include_str!("fixtures/answer.json")
        .replace("2222222222222222222222222222222222222222", "not-a-commit");
    assert_eq!(
        watch().read(answer.as_bytes()),
        Err(ReadError::Head("not-a-commit".into()))
    );
}
