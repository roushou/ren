//! The forwarding route table, pinned, with one fallback-only and two direct
//! configured entries. Clients are configured with these paths, so a
//! refactor must never change one silently: each line is the
//! ingress path, what it accepts, its method, the upstream entry it goes to,
//! that entry's provider, the native endpoint and upstream path, and the
//! harness whose headers it reads.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use tgv_gateway::{
    ConfiguredEntry, EntryIngress, OperatorKey, UpstreamAuth, UpstreamEntry, UpstreamTable,
    UpstreamUrl,
};
use tgv_protocol::label::Labeled;
use tgv_protocol::route::Provider;
use tgv_recorder::routing::UpstreamId;

#[test]
fn every_route_is_pinned() {
    let url = || UpstreamUrl::parse("https://example.com").unwrap();
    // Configured entries mount routes only when they opt in.
    let configured = |id: &str, provider: Provider, ingress| ConfiguredEntry {
        entry: UpstreamEntry {
            id: UpstreamId::parse(id).unwrap(),
            api: provider.native_api(),
            url: url(),
            auth: UpstreamAuth::OperatorKey(OperatorKey::new("fixture").unwrap()),
        },
        ingress,
    };
    let upstreams = UpstreamTable::new(
        url(),
        url(),
        url(),
        vec![
            configured(
                "anthropic-key",
                Provider::Anthropic,
                EntryIngress::FallbackOnly,
            ),
            configured(
                "anthropic-direct",
                Provider::Anthropic,
                EntryIngress::Direct,
            ),
            configured("openai-direct", Provider::OpenAi, EntryIngress::Direct),
        ],
    )
    .unwrap();
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
