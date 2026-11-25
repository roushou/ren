//! The deployer on the host, as the daemon asks it (docs/plans/instances.md,
//! phase 3): one line written to its Unix socket (`[deployer] socket`), one
//! line read back. The gateway is handed it as its [`Deployer`].

use futures_util::future::BoxFuture;
use ren_gateway::deployer::{Deployer, DeployerAnswer, DeployerError, DeployerRequest};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// The deployer listening on its socket.
#[derive(Debug, Clone)]
pub struct DeployerSocket {
    path: PathBuf,
}

impl DeployerSocket {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    async fn send(
        path: PathBuf,
        request: DeployerRequest,
    ) -> Result<DeployerAnswer, DeployerError> {
        let mut stream = tokio::net::UnixStream::connect(&path)
            .await
            .map_err(DeployerError::Unreachable)?;
        stream
            .write_all(request.line().as_bytes())
            .await
            .map_err(DeployerError::Unreachable)?;
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .await
            .map_err(DeployerError::Unreachable)?;
        DeployerAnswer::read(&line)
    }
}

impl Deployer for DeployerSocket {
    fn ask(
        &self,
        request: DeployerRequest,
    ) -> BoxFuture<'static, Result<DeployerAnswer, DeployerError>> {
        Box::pin(Self::send(self.path.clone(), request))
    }
}

#[cfg(test)]
mod tests {
    use super::DeployerSocket;
    use ren_gateway::deployer::{Deployer, DeployerAnswer, DeployerError, DeployerRequest};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    #[tokio::test]
    async fn an_ask_writes_its_line_and_reads_the_deployers() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let path = dir.path().join("deployer.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let deployer = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            stream.get_mut().write_all(b"planned\n").await.unwrap();
            line
        };
        let socket = DeployerSocket::new(path.clone());
        let (answer, asked) = tokio::join!(socket.ask(DeployerRequest::Previews), deployer);
        assert_eq!(answer.unwrap(), DeployerAnswer::Planned);
        assert_eq!(asked, "previews\n");
        drop(dir);
        assert!(matches!(
            socket.ask(DeployerRequest::Promote).await,
            Err(DeployerError::Unreachable(_))
        ));
    }
}
