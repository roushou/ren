//! The relay's shell: the passes it holds, and the listener that reads each
//! request's head, decides it, and passes an admitted tunnel's bytes
//! through unread. Every request is driven inside [`Relay::serve`]'s own
//! future, so the relay spawns no task: its caller runs `serve`.

use crate::{
    head::{Answer, Ask, Head, Reading},
    passage::{Course, Passage, TunnelEnd, Unreachable},
    policy::{Reachable, RelayPolicy, Verdict},
    ticket::Ticket,
};
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use ren_destination::{Host, Target};
use std::{
    collections::{HashMap, hash_map::Entry},
    future::Future,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, PoisonError, Weak},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::mpsc::UnboundedSender,
    time::timeout,
};

/// One holder's pass: who it is, and where its tunnels may go.
#[derive(Debug)]
struct Pass<H> {
    holder: H,
    policy: RelayPolicy,
}

type Passes<H> = Mutex<HashMap<Ticket, Pass<H>>>;

/// A forward proxy for holders it has issued a pass to. `H` names a holder
/// (a run, to Ren); each [`Passage`] carries it.
#[derive(Debug)]
pub struct Relay<H> {
    passes: Arc<Passes<H>>,
}

/// A pass the relay holds until this handle is dropped.
#[derive(Debug)]
#[must_use = "the pass is revoked once its handle is dropped"]
pub struct PassHandle<H> {
    passes: Weak<Passes<H>>,
    ticket: Ticket,
}

/// Why a pass was not issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IssueError {
    #[error("the relay holds a pass with this ticket already")]
    TicketHeld,
}

/// Why a request's head was not read.
enum HeadFailure {
    Invalid,
    /// The client closed or failed before a whole head.
    Gone,
}

impl<H> Default for Relay<H> {
    fn default() -> Self {
        Self {
            passes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<H> Drop for PassHandle<H> {
    fn drop(&mut self) {
        if let Some(passes) = self.passes.upgrade() {
            passes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&self.ticket);
        }
    }
}

impl<H: Clone> Relay<H> {
    /// How long a client has to send its request's head.
    const HEAD_WAIT: Duration = Duration::from_secs(10);
    /// How long a name has to resolve, and an address to connect.
    const REACH_WAIT: Duration = Duration::from_secs(10);

    pub fn new() -> Self {
        Self::default()
    }

    /// Lets the holder that presents `ticket` reach what `policy` lists,
    /// until the handle is dropped.
    pub fn issue(
        &self,
        ticket: Ticket,
        holder: H,
        policy: RelayPolicy,
    ) -> Result<PassHandle<H>, IssueError> {
        let mut passes = self.passes.lock().unwrap_or_else(PoisonError::into_inner);
        match passes.entry(ticket.clone()) {
            Entry::Occupied(_) => Err(IssueError::TicketHeld),
            Entry::Vacant(vacant) => {
                vacant.insert(Pass { holder, policy });
                Ok(PassHandle {
                    passes: Arc::downgrade(&self.passes),
                    ticket,
                })
            }
        }
    }

    /// The holder and policy of the pass `ticket` is, if the relay holds it.
    fn pass(&self, ticket: &Ticket) -> Option<(H, RelayPolicy)> {
        self.passes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(ticket)
            .map(|pass| (pass.holder.clone(), pass.policy.clone()))
    }

    /// Takes requests on `listener` until `shutdown` resolves, sending each
    /// one's [`Passage`] to `passages` as it ends. A request without a
    /// ticket the relay holds is answered and reported to no one: it has no
    /// holder. Tunnels still open at shutdown are closed unreported.
    pub async fn serve(
        &self,
        listener: TcpListener,
        passages: UnboundedSender<Passage<H>>,
        shutdown: impl Future<Output = ()>,
    ) {
        let mut requests = FuturesUnordered::new();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                () = &mut shutdown => return,
                accepted = listener.accept() => {
                    // A failed accept (too many open files, a client gone
                    // before it was taken) ends that client only.
                    if let Ok((stream, _)) = accepted {
                        requests.push(self.take(stream, &passages));
                    }
                }
                Some(()) = requests.next(), if !requests.is_empty() => {}
            }
        }
    }

    /// One client's request, from its head to its tunnel's end.
    async fn take(&self, mut stream: TcpStream, passages: &UnboundedSender<Passage<H>>) {
        let mut read = Vec::with_capacity(1024);
        let head = match timeout(Self::HEAD_WAIT, Self::read_head(&mut stream, &mut read)).await {
            Ok(Ok(head)) => head,
            Ok(Err(HeadFailure::Invalid)) => {
                return Self::answer(&mut stream, Answer::BadRequest).await;
            }
            Ok(Err(HeadFailure::Gone)) | Err(_) => return,
        };
        let Some((holder, policy)) = head.ticket().and_then(|ticket| self.pass(ticket)) else {
            return Self::answer(&mut stream, Answer::TicketNeeded).await;
        };
        let course = match head.ask() {
            Ask::Other { method } => {
                Self::answer(&mut stream, Answer::NotATunnel).await;
                Course::NotATunnel {
                    method: method.clone(),
                }
            }
            Ask::Tunnel(target) => {
                let early = read.get(head.len()..).unwrap_or_default();
                let end = Self::tunnel(&mut stream, early, target, &policy).await;
                Course::Tunnel {
                    target: target.clone(),
                    end,
                }
            }
        };
        // Its receiver gone, the passage has no one to go to.
        let _ = passages.send(Passage::new(holder, course));
    }

    /// Reads until `read` holds a whole head.
    async fn read_head(stream: &mut TcpStream, read: &mut Vec<u8>) -> Result<Head, HeadFailure> {
        let mut chunk = [0_u8; 1024];
        loop {
            match Head::read(read) {
                Reading::Complete(head) => return Ok(head),
                Reading::Invalid(_) => return Err(HeadFailure::Invalid),
                Reading::Incomplete => {}
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return Err(HeadFailure::Gone),
                Ok(n) => read.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            }
        }
    }

    /// Decides the tunnel to `target`, reaches it, and passes its bytes
    /// (`early` first: what the client sent after its head) until either
    /// side closes.
    async fn tunnel(
        stream: &mut TcpStream,
        early: &[u8],
        target: &Target,
        policy: &RelayPolicy,
    ) -> TunnelEnd {
        let through = match policy.decide(target) {
            Verdict::Admitted(destination) => destination,
            Verdict::Refused(refusal) => {
                Self::answer(stream, Answer::Refused(refusal)).await;
                return TunnelEnd::Refused(refusal);
            }
        };
        let resolved = match Self::resolve(target).await {
            Ok(resolved) => resolved,
            Err(why) => {
                Self::answer(stream, Answer::Unreachable).await;
                return TunnelEnd::Unreachable { through, why };
            }
        };
        let reachable = match Reachable::of(&through, target, &resolved) {
            Ok(reachable) => reachable,
            Err(refusal) => {
                Self::answer(stream, Answer::Refused(refusal)).await;
                return TunnelEnd::Refused(refusal);
            }
        };
        let mut upstream = match Self::connect(&reachable, target.port()).await {
            Ok(upstream) => upstream,
            Err(why) => {
                Self::answer(stream, Answer::Unreachable).await;
                return TunnelEnd::Unreachable { through, why };
            }
        };
        if stream
            .write_all(&Answer::Established.bytes())
            .await
            .is_err()
            || upstream.write_all(early).await.is_err()
        {
            return TunnelEnd::Broken { through };
        }
        match tokio::io::copy_bidirectional(stream, &mut upstream).await {
            Ok((sent, received)) => TunnelEnd::Closed {
                through,
                sent: sent.saturating_add(u64::try_from(early.len()).unwrap_or(u64::MAX)),
                received,
            },
            Err(_) => TunnelEnd::Broken { through },
        }
    }

    /// The addresses `target` names: its own, or what its name resolves to.
    async fn resolve(target: &Target) -> Result<Vec<IpAddr>, Unreachable> {
        match target.host() {
            Host::Address(address) => Ok(vec![*address]),
            Host::Name(name) => {
                match timeout(
                    Self::REACH_WAIT,
                    tokio::net::lookup_host((name.as_str(), target.port())),
                )
                .await
                {
                    Ok(Ok(found)) => {
                        let found: Vec<IpAddr> = found.map(|address| address.ip()).collect();
                        if found.is_empty() {
                            Err(Unreachable::Unresolved)
                        } else {
                            Ok(found)
                        }
                    }
                    Ok(Err(_)) => Err(Unreachable::Unresolved),
                    Err(_) => Err(Unreachable::TimedOut),
                }
            }
        }
    }

    /// The first of `reachable` on `port` that takes the connection.
    async fn connect(reachable: &[Reachable], port: u16) -> Result<TcpStream, Unreachable> {
        let mut why = Unreachable::Unresolved;
        for address in reachable {
            let address = SocketAddr::new(address.address(), port);
            match timeout(Self::REACH_WAIT, TcpStream::connect(address)).await {
                Ok(Ok(upstream)) => return Ok(upstream),
                Ok(Err(_)) => why = Unreachable::Refused,
                Err(_) => why = Unreachable::TimedOut,
            }
        }
        Err(why)
    }

    /// Sends `answer`; a client gone before it is answered needs none.
    async fn answer(stream: &mut TcpStream, answer: Answer) {
        let _ = stream.write_all(&answer.bytes()).await;
        let _ = stream.flush().await;
    }
}
