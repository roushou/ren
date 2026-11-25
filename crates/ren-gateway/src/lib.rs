//! Ren's HTTP interface: native forwarding, management, and control routes.
//! The primary type is [`Gateway`].
//!
//! A [`Gateway`] admits a call, records it through the recorder's tap, and
//! relays the native request and response unchanged except for the header
//! policy. Each attempt's lifecycle is the pure [`HttpAttempt`] machine; this
//! module only feeds it events and runs its actions.
//!
//! A request is buffered whole, planned by the [`Policy`]
//! against one operator snapshot, changed by its transform rules and
//! the upstream entry's credential override, and tried on its
//! candidates in order while the retry rule allows. The plan, each
//! attempt's changes, and any rejection are recorded with the call.
//!
//! The forwarding path itself is the `forward` module: the route table, the
//! header policy, request buffering, the upstream seam, the HTTP and
//! WebSocket relays, and the ingress. The crate also owns what forwarding
//! decides with: the pure lifecycles in [`lifecycle`] (an HTTP attempt, a
//! call's candidates, a WebSocket channel, and retry eligibility), operator
//! state (`operator`) and the [`Controls`] that hold it (`control`),
//! who is calling (`user`), call planning (`policy`), and which previews of
//! Ren run (`preview`).
//!
//! Management handlers read history through the store's `Reader`, stream live
//! observations from the recorder, and run authenticated controls and backups.
//! [`ManagementSetup`] supplies their resources. All routes share one router
//! and listener; the host owns resource construction and task lifecycles.
//!
//! Contract:
//! - configuration: [`GatewayConfig`] (admission limit, header deadline,
//!   configuration revision, buffer, policy, upstream entries);
//! - capabilities: an [`Upstream`], a [`Recorder`], the shared [`Controls`],
//!   a [`Clock`], and the `Reader` and `Backup` in [`ManagementSetup`],
//!   which also names the running [`crate::release::Build`] that `/health` reports;
//! - inputs: inference requests on the upstream table's routes, management reads, and controls;
//! - outputs: provider responses, management data, and HTTP error responses;
//! - lifecycle: [`Shutdown`] drains, then aborts, through [`Ingress`].
//!
//! A WebSocket handshake on a [`Accepts::PostOrWebSocket`] route is itself an
//! HTTP attempt (recorded with method `GET`): a refusal or a failed connect is
//! relayed and recorded like any response. Once both legs upgrade, the
//! channel's calls are relayed by the `forward::websocket` module.
//!
//! [`HttpAttempt`]: crate::lifecycle::attempt::HttpAttempt

pub mod api;
pub mod build;
pub mod control;
pub mod deployment;
pub mod factory;
mod forward;
pub mod github;
pub mod lifecycle;
mod limits;
pub mod operator;
pub mod packages;
pub mod policy;
#[cfg(test)]
mod purity;
pub mod release;
pub mod reply;
mod timeout;
pub mod user;

pub use api::{
    ApiRead, ControlOptions, ControlRefusal, ControlToken, ManagementSetup, ReleaseFolder,
};
pub use build::{Environment, InstanceOptions};
pub use forward::GatewayError;
pub use forward::buffer::{
    BufferOptions, BufferOptionsError, BufferRefusal, Buffered, RequestBuffer, RequestLimits,
};
pub use forward::headers::HeaderPolicy;
pub use forward::ingress::{Ingress, IngressError, Shutdown, Stopped};
pub use forward::route::{Accepts, BoundRoute, Forward, Route};
pub use forward::upstream::{
    AuthOverride, ConfiguredEntry, Connected, EntryIngress, NativeUrls, OperatorKey,
    ReqwestUpstream, Socket, TransportTimeouts, Upstream, UpstreamAuth, UpstreamBody,
    UpstreamConfigError, UpstreamEntry, UpstreamHandshake, UpstreamRequest, UpstreamSocket,
    UpstreamTable, UpstreamTableError, UpstreamUrl,
};
pub use forward::websocket::{WebSocketMode, WebSocketOptions};
pub use limits::{ConcurrencyLimit, ConcurrencyOutOfRange};
pub use timeout::{TimeoutOutOfRange, TimeoutSecs};

use crate::{
    api::{ApiState, ControlApi, ReadSources},
    control::Controls,
    limits::UserLimits,
    policy::Policy,
};
use axum::Router;
use ren_recorder::{Recorder, routing::ConfigRevision, time::Clock};
use std::{sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};

/// Declarative gateway settings, fixed for the daemon's lifetime.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Calls forwarded at once; more are rejected with `503`.
    pub max_concurrent_requests: ConcurrencyLimit,
    /// Longest wait for a response head after dispatch, and on WebSocket for
    /// a call's first upstream message.
    pub response_header_timeout: Duration,
    /// Longest quiet gap in a WebSocket call's upstream messages. HTTP bodies
    /// get theirs from the transport (`TransportTimeouts::idle`).
    pub stream_idle_timeout: Duration,
    /// Recorded with every call.
    pub config_revision: ConfigRevision,
    pub websocket: WebSocketOptions,
    pub buffer: BufferOptions,
    pub policy: Policy,
    pub upstreams: UpstreamTable,
    /// The named users forwarded requests are matched to.
    pub users: user::Users,
    /// Calls every caller that is not a named user may have running at
    /// once, together; unlimited beyond the global limit when absent.
    pub max_concurrent_unnamed: Option<ConcurrencyLimit>,
}

/// Serves Ren's HTTP interface, forwarding native requests to one [`Upstream`].
pub struct Gateway<U> {
    config: GatewayConfig,
    upstream: U,
    recorder: Recorder,
    controls: Arc<Controls>,
    clock: Arc<dyn Clock>,
    buffer: RequestBuffer,
    admission: Arc<Semaphore>,
    /// Upgraded WebSocket channels; separate from call admission.
    channels: Arc<Semaphore>,
    /// Each caller's own limit, checked before call admission (so a caller
    /// over it gets 429, not 503).
    user_limits: UserLimits,
    shutdown: Shutdown,
}

impl<U: Upstream> Gateway<U> {
    pub fn new(
        config: GatewayConfig,
        upstream: U,
        recorder: Recorder,
        controls: Arc<Controls>,
        clock: Arc<dyn Clock>,
        shutdown: Shutdown,
    ) -> Self {
        Self {
            admission: config.max_concurrent_requests.semaphore(),
            channels: config.websocket.max_channels.semaphore(),
            user_limits: UserLimits::new(&config.users, config.max_concurrent_unnamed),
            buffer: RequestBuffer::new(config.buffer),
            config,
            upstream,
            recorder,
            controls,
            clock,
            shutdown,
        }
    }

    /// Assemble forwarding, management, and control routes on one listener,
    /// and, with `runs`, model forwarding alone on that one: what sandboxed
    /// runs reach. Transfers ownership of the gateway and management
    /// resources to the server.
    #[must_use]
    pub fn into_ingress(
        self,
        listener: TcpListener,
        runs: Option<TcpListener>,
        management: ManagementSetup,
    ) -> Ingress {
        let gateway = Arc::new(self);
        let ingress = Ingress::new(listener, gateway.clone().router(management));
        match runs {
            Some(runs) => ingress.with_runs(runs, gateway.forwarding_router()),
            None => ingress,
        }
    }

    /// All HTTP routes. Tests can drive this same router without a connection.
    pub fn router(self: Arc<Self>, management: ManagementSetup) -> Router {
        let reader = management.reader.clone();
        let release = management.release.clone();
        let api = ApiState::new(
            ReadSources {
                reader: management.reader,
                retention: management.retention,
                build: management.build,
                instance: management.instance,
                release: management.release,
                pulls: management.pulls,
            },
            self.recorder.clone(),
            self.controls.clone(),
            self.clock.clone(),
            self.shutdown.drain_token(),
            self.config.users.clone(),
        );
        let control = ControlApi::new(
            self.controls.clone(),
            self.recorder.clone(),
            self.clock.clone(),
            management.token,
            management.control.tailscale_logins,
            self.config.users.clone(),
            management.backup,
        );
        let feeds_end = self.shutdown.drain_token();
        self.forwarding_router()
            .merge(api.router())
            .merge(control.router(
                reader,
                management.runs,
                management.packages,
                release,
                management.factory,
                feeds_end,
            ))
    }
}
