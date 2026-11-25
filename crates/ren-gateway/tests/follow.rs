//! What the GitHub watch's sightings ask of the factory: previews when the
//! pull requests that want one move, the canary when `main` moves. With
//! invented repositories and commits.
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use ren_gateway::factory::{FactoryRequest, Follower};
use ren_github::{
    Listing, Observation, PullRequest, Readiness, Reported, RepositorySlug, Sighting, Snapshot,
};
use std::time::SystemTime;

fn open(number: u64, head: char, readiness: Readiness) -> PullRequest {
    PullRequest {
        number: number.to_string().parse().unwrap(),
        title: format!("feat: change {number}"),
        body: String::new(),
        url: format!("https://github.com/acme/ren/pull/{number}"),
        author: None,
        readiness,
        head_branch: format!("change-{number}"),
        head: head.to_string().repeat(40).parse().unwrap(),
        base_branch: "main".into(),
        merge: Reported::Unknown("UNKNOWN".into()),
        review: Reported::Unknown("UNKNOWN".into()),
        checks: Reported::Unknown("UNKNOWN".into()),
        check_list: Vec::new(),
    }
}

fn seen(main: char, pulls: Vec<PullRequest>) -> Observation {
    Observation::Waiting.seen(Sighting {
        snapshot: Snapshot {
            default_branch: Some(("main".into(), main.to_string().repeat(40).parse().unwrap())),
            pull_requests: pulls,
            listing: Listing::Complete,
            manifest: None,
        },
        at: SystemTime::UNIX_EPOCH,
    })
}

fn ren() -> RepositorySlug {
    "acme/ren".parse().unwrap()
}

fn follower() -> Follower {
    Follower::watching([
        ("ren".parse().unwrap(), ren()),
        ("acme-web".parse().unwrap(), "acme/web".parse().unwrap()),
    ])
}

const PREVIEWS: FactoryRequest = FactoryRequest::Previews;

#[test]
fn previews_follow_pull_requests_opening_moving_and_closing() {
    let mut follower = follower();
    let ready = |number, head| open(number, head, Readiness::Ready);
    // Nothing seen yet asks nothing; the first answer asks for previews.
    assert_eq!(follower.see(&ren(), &Observation::Waiting), []);
    let one = seen('0', vec![ready(7, 'a')]);
    assert_eq!(follower.see(&ren(), &one), [PREVIEWS]);
    assert_eq!(follower.see(&ren(), &one), []);
    // Opened, a head moved, closed.
    let two = seen('0', vec![ready(8, 'b'), ready(7, 'a')]);
    assert_eq!(follower.see(&ren(), &two), [PREVIEWS]);
    let moved = seen('0', vec![ready(8, 'c'), ready(7, 'a')]);
    assert_eq!(follower.see(&ren(), &moved), [PREVIEWS]);
    let closed = seen('0', vec![ready(8, 'c')]);
    assert_eq!(follower.see(&ren(), &closed), [PREVIEWS]);
    // A draft opened is nothing a preview follows; one marked ready is.
    let draft = seen('0', vec![ready(8, 'c'), open(9, 'd', Readiness::Draft)]);
    assert_eq!(follower.see(&ren(), &draft), []);
    let marked = seen('0', vec![ready(8, 'c'), ready(9, 'd')]);
    assert_eq!(follower.see(&ren(), &marked), [PREVIEWS]);
    // A failed read keeps its last answer apart: not a move, not a close.
    let failed = marked.failed("refused".into(), SystemTime::UNIX_EPOCH);
    assert_eq!(follower.see(&ren(), &failed), []);
}

#[test]
fn the_canary_follows_main_once_it_was_seen() {
    let mut follower = follower();
    // The first sighting is no move: a restarted daemon starts no pass.
    assert_eq!(follower.see(&ren(), &seen('a', vec![])), [PREVIEWS]);
    assert_eq!(follower.see(&ren(), &seen('a', vec![])), []);
    assert_eq!(
        follower.see(&ren(), &seen('b', vec![])),
        [FactoryRequest::Canary("ren".parse().unwrap())]
    );
    // Each project watched on the repository that moved, and no other.
    let web: RepositorySlug = "acme/web".parse().unwrap();
    assert_eq!(follower.see(&web, &seen('a', vec![])), [PREVIEWS]);
    assert_eq!(
        follower.see(&web, &seen('c', vec![open(1, 'e', Readiness::Ready)])),
        [
            PREVIEWS,
            FactoryRequest::Canary("acme-web".parse().unwrap())
        ]
    );
}
