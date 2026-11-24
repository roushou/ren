//! The factory on the host, as the daemon asks it (docs/plans/instances.md,
//! phase 3): one line written to its Unix socket (`[factory] socket`), one
//! line read back. The gateway is handed it as its [`Factory`].

use futures_util::future::BoxFuture;
use ren_gateway::factory::{Factory, FactoryAnswer, FactoryError, FactoryRequest};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// The factory listening on its socket.
#[derive(Debug, Clone)]
pub struct FactorySocket {
    path: PathBuf,
}

impl FactorySocket {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    async fn ask(path: PathBuf, request: FactoryRequest) -> Result<FactoryAnswer, FactoryError> {
        let mut stream = tokio::net::UnixStream::connect(&path)
            .await
            .map_err(FactoryError::Unreachable)?;
        stream
            .write_all(request.line().as_bytes())
            .await
            .map_err(FactoryError::Unreachable)?;
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .await
            .map_err(FactoryError::Unreachable)?;
        FactoryAnswer::read(&line)
    }
}

impl Factory for FactorySocket {
    fn plan(&self) -> BoxFuture<'static, Result<FactoryAnswer, FactoryError>> {
        Box::pin(Self::ask(self.path.clone(), FactoryRequest::Previews))
    }
}

#[cfg(test)]
mod tests {
    use super::FactorySocket;
    use ren_gateway::factory::{Factory, FactoryAnswer, FactoryError};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    #[tokio::test]
    async fn an_ask_writes_its_line_and_reads_the_factorys() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let path = dir.path().join("factory.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let factory = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            stream.get_mut().write_all(b"planned\n").await.unwrap();
            line
        };
        let socket = FactorySocket::new(path.clone());
        let (answer, asked) = tokio::join!(socket.plan(), factory);
        assert_eq!(answer.unwrap(), FactoryAnswer::Planned);
        assert_eq!(asked, "previews\n");
        drop(dir);
        assert!(matches!(
            socket.plan().await,
            Err(FactoryError::Unreachable(_))
        ));
    }
}
