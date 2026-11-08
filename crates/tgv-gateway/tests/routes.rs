//! The forwarding route table, pinned. Clients are configured with these
//! paths, so a refactor must never change one silently: each line is the
//! ingress path, what it accepts, its method, the upstream entry it goes to,
//! that entry's provider, the native endpoint and upstream path, and the
//! harness whose headers it reads.

use tgv_gateway::Route;
use tgv_protocol::label::Labeled;
use tgv_recorder::routing::UpstreamId;

#[test]
fn every_route_is_pinned() {
    let table: String = Route::ALL
        .iter()
        .map(|route| {
            format!(
                "{} {:?} {} upstream={} provider={} endpoint={} {} harness={}\n",
                route.path,
                route.accepts,
                route.accepts.method().label(),
                UpstreamId::default_for(route.provider),
                route.provider.label(),
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
