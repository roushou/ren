//! The ways through a relayed run's container, which has no network but
//! its loopback: a Unix socket the daemon listens on, in a folder the
//! container mounts, for each listener the run may reach (Ren's gateway,
//! the relay). Inside, the run host listens on the run's loopback and
//! passes each connection to its socket ([`Doorway`]); outside, the daemon
//! passes each connection on a socket to its listener ([`Doors`]). Bytes
//! pass through both unread. Each side drives its connections in its own
//! future and spawns no task.

use futures_util::{StreamExt as _, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};

/// One way out of a run's container: a port on its loopback that reaches
/// the socket at `socket`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doorway {
    /// Where the run connects, on its own loopback.
    pub inside: SocketAddr,
    /// The daemon's socket it reaches, in the folder the container mounts.
    pub socket: PathBuf,
}

/// The daemon's side: the folder of sockets relayed runs' containers mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doors {
    folder: PathBuf,
}

impl Doorway {
    /// Listens on the run's loopback, then passes each connection to the
    /// socket until the returned future is dropped. Listening first, so
    /// the client finds its gateway from its first request.
    pub async fn open(&self) -> io::Result<impl Future<Output = ()> + use<>> {
        let listener = TcpListener::bind(self.inside).await?;
        let socket = self.socket.clone();
        Ok(async move {
            let mut passing = FuturesUnordered::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        if let Ok((stream, _)) = accepted {
                            passing.push(Self::pass(stream, socket.clone()));
                        }
                    }
                    Some(()) = passing.next(), if !passing.is_empty() => {}
                }
            }
        })
    }

    async fn pass(mut stream: TcpStream, socket: PathBuf) {
        // The daemon away (restarting), the connection fails as a refused
        // one would, and the client tries again.
        if let Ok(mut door) = UnixStream::connect(&socket).await {
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut door).await;
        }
    }
}

impl Doors {
    /// The folder under the runs' folder that relayed runs mount.
    pub const FOLDER: &str = ".reach";
    /// The gateway's socket.
    pub const GATEWAY: &str = "gateway.sock";
    /// The relay's socket.
    pub const RELAY: &str = "relay.sock";
    /// The grants socket, which a run's git helper asks directly.
    pub const GRANTS: &str = "grants.sock";

    pub fn new(runs: &Path) -> Self {
        Self {
            folder: runs.join(Self::FOLDER),
        }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The socket called `name` in the folder.
    pub fn socket(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }

    /// Listens on the socket called `name`, made afresh, then passes each
    /// connection to `target` until the returned future is dropped.
    pub fn open(
        &self,
        name: &str,
        target: SocketAddr,
    ) -> io::Result<impl Future<Output = ()> + use<>> {
        let listener = self.listen(name)?;
        Ok(async move {
            let mut passing = FuturesUnordered::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        if let Ok((stream, _)) = accepted {
                            passing.push(Self::pass(stream, target));
                        }
                    }
                    Some(()) = passing.next(), if !passing.is_empty() => {}
                }
            }
        })
    }

    /// Listens on the socket called `name`, made afresh.
    pub fn listen(&self, name: &str) -> io::Result<UnixListener> {
        use std::os::unix::fs::DirBuilderExt as _;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.folder)?;
        let socket = self.socket(name);
        match std::fs::remove_file(&socket) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Ok(()) | Err(_) => {}
        }
        UnixListener::bind(&socket)
    }

    async fn pass(mut stream: UnixStream, target: SocketAddr) {
        if let Ok(mut listener) = TcpStream::connect(target).await {
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut listener).await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "the test spawns what the daemon and the run host drive"
    )]
    use super::{Doors, Doorway};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// A connection on the run's loopback reaches the daemon's listener
    /// through the socket, both ways.
    #[tokio::test]
    async fn a_run_reaches_a_listener_through_its_doorway() {
        let dir = tempfile::tempdir().unwrap();
        let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = echo.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = echo.accept().await.unwrap();
            let (mut read, mut write) = stream.split();
            tokio::io::copy(&mut read, &mut write).await.unwrap();
        });
        let doors = Doors::new(dir.path());
        tokio::spawn(doors.open(Doors::GATEWAY, target).unwrap());
        let inside = {
            let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            free.local_addr().unwrap()
        };
        let doorway = Doorway {
            inside,
            socket: doors.socket(Doors::GATEWAY),
        };
        tokio::spawn(doorway.open().await.unwrap());

        let mut run = tokio::net::TcpStream::connect(inside).await.unwrap();
        run.write_all(b"POST /v1/messages").await.unwrap();
        let mut echoed = [0_u8; 17];
        run.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"POST /v1/messages");
        assert!(doors.folder().ends_with(".reach"));
    }
}
