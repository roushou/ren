//! Operator controls: pause, session blocks, and cancelling a running
//! call. The daemon's control API exposes them.
//!
//! [`Controls`] holds the current [`OperatorState`] snapshot and the calls an
//! operator can cancel. The gateway plans every call against one snapshot and
//! registers each running call; the control API changes the state only after
//! the recorder has reserved room to record the change, so every operator
//! decision is recorded.
//!
//! Contract:
//! - capabilities: a [`Recorder`];
//! - inputs: operator commands and call registrations; outputs: the operator
//!   state and the running calls;
//! - errors: [`ControlError`].

use crate::{
    channel::CallSlot,
    operator::{OperatorCommand, OperatorState},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError, RwLock},
};
use tgv_protocol::{native::NativeSessionId, route::Transport};
use tgv_recorder::{Recorder, batch::Actor, id::CallId};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Why a control action was not carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    /// The recorder has no room to record the action, so it is not applied.
    #[error("the recorder cannot record this action now; try again")]
    Unrecorded,
    /// The action could not be serialized for its record, so it is not applied.
    #[error("this action cannot be recorded")]
    Unrecordable,
    #[error("no running call has this id")]
    NotRunning,
}

/// How to stop one running call.
#[derive(Debug, Clone)]
pub enum Cancel {
    /// An HTTP call: its relay watches this token.
    Http(CancellationToken),
    /// A call on a WebSocket channel: its relay is sent the call's slot.
    WebSocket {
        slot: CallSlot,
        relay: mpsc::UnboundedSender<CallSlot>,
    },
}

impl Cancel {
    fn request(&self) {
        match self {
            Self::Http(token) => token.cancel(),
            // A closed relay already ended the call.
            Self::WebSocket { slot, relay } => drop(relay.send(*slot)),
        }
    }
}

/// A call an operator can cancel.
#[derive(Debug)]
struct Running {
    transport: Transport,
    session: Option<NativeSessionId>,
    cancel: Cancel,
}

/// A running call, as the control API lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RunningCall {
    pub call: CallId,
    pub transport: Transport,
}

/// The operator state and the running calls.
#[derive(Debug, Default)]
pub struct Controls {
    state: RwLock<Arc<OperatorState>>,
    running: Mutex<HashMap<CallId, Running>>,
}

impl Controls {
    /// Controls starting from `state`: the snapshot the store restored, or
    /// the empty state in a new data directory.
    pub fn restore(state: OperatorState) -> Self {
        Self {
            state: RwLock::new(Arc::new(state)),
            running: Mutex::default(),
        }
    }

    /// The current snapshot. A call planned against it keeps it.
    pub fn snapshot(&self) -> Arc<OperatorState> {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Record and apply `command`. The state changes only if the recorder
    /// reserved room for the record; changes are recorded in revision order.
    pub fn apply(
        &self,
        recorder: &Recorder,
        actor: Actor,
        command: OperatorCommand,
    ) -> Result<Arc<OperatorState>, ControlError> {
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        let slot = recorder.operator_slot().ok_or(ControlError::Unrecorded)?;
        let next = Arc::new(state.apply(command));
        slot.record(next.revision(), actor, &command, next.as_ref())
            .map_err(|_| ControlError::Unrecordable)?;
        *state = next.clone();
        Ok(next)
    }

    /// Make a call cancellable until the returned guard drops.
    pub fn register(
        self: &Arc<Self>,
        call: CallId,
        transport: Transport,
        session: Option<NativeSessionId>,
        cancel: Cancel,
    ) -> Registered {
        self.running_calls().insert(
            call,
            Running {
                transport,
                session,
                cancel,
            },
        );
        Registered {
            controls: self.clone(),
            call,
        }
    }

    /// Whether `call` is running.
    pub fn running_call(&self, call: CallId) -> Result<(), ControlError> {
        if self.running_calls().contains_key(&call) {
            Ok(())
        } else {
            Err(ControlError::NotRunning)
        }
    }

    /// Ask a running call to stop. Returns the call's session, for an
    /// optional block.
    pub fn cancel(&self, call: CallId) -> Result<Option<NativeSessionId>, ControlError> {
        let running = self.running_calls();
        let running = running.get(&call).ok_or(ControlError::NotRunning)?;
        running.cancel.request();
        Ok(running.session)
    }

    pub fn running(&self) -> Vec<RunningCall> {
        let mut calls: Vec<RunningCall> = self
            .running_calls()
            .iter()
            .map(|(call, running)| RunningCall {
                call: *call,
                transport: running.transport,
            })
            .collect();
        calls.sort_by_key(|running| running.call);
        calls
    }

    fn running_calls(&self) -> std::sync::MutexGuard<'_, HashMap<CallId, Running>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Keeps a call cancellable; dropping it unregisters the call.
#[must_use = "the call is cancellable only while this guard lives"]
#[derive(Debug)]
pub struct Registered {
    controls: Arc<Controls>,
    call: CallId,
}

impl Drop for Registered {
    fn drop(&mut self) {
        self.controls.running_calls().remove(&self.call);
    }
}

#[cfg(test)]
mod tests {
    use super::{Cancel, ControlError, Controls};
    use std::sync::Arc;
    use tgv_protocol::route::Transport;
    use tgv_recorder::id::CallId;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    #[test]
    fn a_registered_call_is_cancellable_until_its_guard_drops() {
        let controls = Arc::new(Controls::default());
        let call = CallId::restore(Uuid::from_u128(1));
        let token = CancellationToken::new();
        let guard = controls.register(call, Transport::Http, None, Cancel::Http(token.clone()));
        assert_eq!(controls.running().len(), 1);
        assert_eq!(controls.cancel(call), Ok(None));
        assert!(token.is_cancelled());
        drop(guard);
        assert_eq!(controls.cancel(call), Err(ControlError::NotRunning));
        assert!(controls.running().is_empty());
    }
}
