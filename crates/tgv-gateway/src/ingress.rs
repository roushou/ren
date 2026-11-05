//! Serving the router, and the shutdown drain.
//!
//! Shutdown has two steps. Draining stops admitting calls and lets HTTP finish
//! on its own; when the deadline passes, aborting cancels what remains, which
//! records each attempt as cancelled by shutdown instead of cutting it off.

use axum::Router;
use std::{future::IntoFuture, io, time::Duration};
use tokio::net::TcpListener;
use tokio_util::{
    sync::{CancellationToken, WaitForCancellationFutureOwned},
    task::{TaskTracker, task_tracker::TaskTrackerToken},
};

/// The daemon's shutdown signal, shared by the server, the gateway, and the
/// management API. Aborting implies draining. It also tracks upgraded
/// WebSocket channels, which outlive their HTTP connection, so the drain
/// waits for them too.
#[derive(Debug, Clone)]
pub struct Shutdown {
    drain: CancellationToken,
    abort: CancellationToken,
    channels: TaskTracker,
}

impl Default for Shutdown {
    fn default() -> Self {
        let abort = CancellationToken::new();
        Self {
            drain: abort.child_token(),
            abort,
            channels: TaskTracker::new(),
        }
    }
}

impl Shutdown {
    /// Stop admitting new calls and close long-lived feeds. In-flight calls
    /// continue until they end or shutdown aborts them.
    pub fn drain(&self) {
        self.drain.cancel();
    }

    /// Cancel every call still running.
    pub fn abort(&self) {
        self.abort.cancel();
    }

    pub fn is_draining(&self) -> bool {
        self.drain.is_cancelled()
    }

    pub fn draining(&self) -> WaitForCancellationFutureOwned {
        self.drain.clone().cancelled_owned()
    }

    /// A token cancelled when draining starts, for components that only need
    /// to stop (the live feed).
    pub fn drain_token(&self) -> CancellationToken {
        self.drain.clone()
    }

    pub(super) fn abort_token(&self) -> CancellationToken {
        self.abort.clone()
    }

    /// Held by a WebSocket channel until it ends.
    pub(super) fn track_channel(&self) -> TaskTrackerToken {
        self.channels.token()
    }

    /// Every tracked channel has ended. Only meaningful once draining, when
    /// the gateway admits no new channels.
    async fn channels_closed(&self) {
        self.channels.close();
        self.channels.wait().await;
    }
}

/// How the server stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Stopped {
    /// Every connection finished before the deadline.
    Drained,
    /// The deadline passed; remaining calls were cancelled by shutdown.
    Aborted,
}

#[derive(Debug, thiserror::Error)]
pub enum IngressError {
    #[error("HTTP server failed")]
    Server(#[source] io::Error),
    #[error("HTTP connections did not close after shutdown cancelled them")]
    Stalled,
}

/// A bound listener and the router it serves.
pub struct Ingress {
    listener: TcpListener,
    router: Router,
}

impl Ingress {
    /// After an abort, connections close as soon as their bodies report the
    /// cancellation; this bounds a connection that does not.
    const ABORT_GRACE: Duration = Duration::from_secs(2);

    pub fn new(listener: TcpListener, router: Router) -> Self {
        Self { listener, router }
    }

    /// Serve until shutdown drains, then wait up to `deadline` for connections
    /// and WebSocket channels to finish before aborting what remains.
    pub async fn serve(
        self,
        shutdown: Shutdown,
        deadline: Duration,
    ) -> Result<Stopped, IngressError> {
        let server = axum::serve(self.listener, self.router)
            .with_graceful_shutdown(shutdown.draining())
            .into_future();
        tokio::pin!(server);
        tokio::select! {
            result = &mut server => return result.map(|()| Stopped::Drained).map_err(IngressError::Server),
            () = shutdown.draining() => {}
        }
        // Channels are waited for on their own: an upgraded socket has left
        // the server's connection tracking.
        let finished = async {
            let (served, ()) = tokio::join!(&mut server, shutdown.channels_closed());
            served
        };
        tokio::pin!(finished);
        if let Ok(result) = tokio::time::timeout(deadline, &mut finished).await {
            return result
                .map(|()| Stopped::Drained)
                .map_err(IngressError::Server);
        }
        shutdown.abort();
        match tokio::time::timeout(Self::ABORT_GRACE, &mut finished).await {
            Ok(result) => result
                .map(|()| Stopped::Aborted)
                .map_err(IngressError::Server),
            Err(_) => Err(IngressError::Stalled),
        }
    }
}
