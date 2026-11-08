//! The forwarding route table, pinned. Clients are configured with these
//! paths, so a refactor must never change one silently: each line is the
//! ingress path, what it accepts, its method, the upstream entry it goes to,
//! that entry's provider, the native endpoint and upstream path, and the
//! harness whose headers it reads.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use tgv_gateway::{UpstreamTable, UpstreamUrl};
use tgv_protocol::label::Labeled;

#[test]
fn every_route_is_pinned() {
    let url = || UpstreamUrl::parse("https://example.com").unwrap();
    let upstreams = UpstreamTable::new(url(), url(), url(), Vec::new()).unwrap();
    let table: String = upstreams
        .routes()
        .iter()
        .map(|bound| {
            let route = bound.forward;
            format!(
                "{} {:?} {} upstream={} provider={} endpoint={} {} harness={}\n",
                bound.path,
                route.accepts,
                route.accepts.method().label(),
                bound.upstream,
                bound.api.provider().label(),
                route.endpoint.label(),
                route.upstream_path,
                route.harness.map_or("none", |harness| harness.into()),
            )
        })
        .collect();
    let pinned = include_str!("fixtures/routes.txt");
    assert!(
        table == pinned,
        "routes changed; the current table is:\n{table}"
    );
}
