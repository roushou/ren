//! The deployer as a service (docs/plans/instances.md, phases 3 and 6): it
//! listens on a Unix socket and takes up what it is asked at once, not at
//! the canary timer's next pass, one pass at a time. Asks that arrive while
//! a pass runs are taken together by the next round, in order: a promotion
//! the operator asked for, a canary pass over `main` (for the project it
//! builds; another's is [`DeployerAnswer::NotBuilt`]), then the previews.
//! A promotion's or the canary's askers are answered as its pass starts
//! ([`DeployerAnswer::Started`]); the previews' once the pass planned which
//! run ([`DeployerAnswer::Planned`]), or that it failed before it did.

use super::{CanaryError, Deployer, PublishArgs, RunArgs, Source, previews::PreviewPass};
use crate::{progress::Progress, replica::ProductionPlace};
use ren_gateway::deployer::{DeployerAnswer, DeployerRequest};
use ren_harness::host::HostEnv;
use std::{
    io::{BufRead as _, BufReader, ErrorKind, Write as _},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

/// The deployer listening for asks.
pub(super) struct Service<'a> {
    pub deployer: &'a Deployer,
    pub production: &'a ProductionPlace,
    pub publish: PublishArgs,
    pub project: &'a str,
}

impl Service<'_> {
    /// Its socket's folder, under the user's runtime folder.
    const FOLDER: &str = "ren-deployer";
    /// How long an asker has to say what it asks.
    const ASK_WAIT: Duration = Duration::from_secs(2);

    /// Where the deployer listens: what `[deployer] socket` names.
    pub fn socket(env: &HostEnv) -> Option<PathBuf> {
        env.absolute_path("XDG_RUNTIME_DIR")
            .map(|runtime| runtime.join(Self::FOLDER).join("deployer.sock"))
    }

    /// Listens on `socket` until stopped.
    pub fn serve(&self, env: &HostEnv, socket: &Path) -> Result<(), CanaryError> {
        let listener = Self::listen(socket)?;
        Progress(&format!("The deployer listens on {}", socket.display())).show();
        loop {
            let first = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) => {
                    println!("warning: an ask was not taken: {error}");
                    continue;
                }
            };
            let askers = Self::askers(&listener, first);
            self.round(env, askers);
        }
    }

    /// The socket, made afresh in a folder only the operator reaches.
    fn listen(socket: &Path) -> Result<UnixListener, CanaryError> {
        use std::os::unix::fs::DirBuilderExt as _;
        if let Some(folder) = socket.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(folder)
                .map_err(CanaryError::Serve)?;
        }
        match std::fs::remove_file(socket) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(CanaryError::Serve(error)),
        }
        UnixListener::bind(socket).map_err(CanaryError::Serve)
    }

    /// `first` and every ask already waiting, each with what it asked; one
    /// that asked nothing the deployer takes is told so.
    fn askers(listener: &UnixListener, first: UnixStream) -> Vec<(UnixStream, DeployerRequest)> {
        let mut streams = vec![first];
        if listener.set_nonblocking(true).is_ok() {
            while let Ok((stream, _)) = listener.accept() {
                streams.push(stream);
            }
            let _ = listener.set_nonblocking(false);
        }
        streams
            .into_iter()
            .filter_map(|stream| match Self::request(&stream) {
                Some(request) => Some((stream, request)),
                None => {
                    Self::answer(
                        vec![stream],
                        &DeployerAnswer::Failed("not a request the deployer takes".into()),
                    );
                    None
                }
            })
            .collect()
    }

    fn request(stream: &UnixStream) -> Option<DeployerRequest> {
        stream.set_nonblocking(false).ok()?;
        stream.set_read_timeout(Some(Self::ASK_WAIT)).ok()?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).ok()?;
        DeployerRequest::read(&line)
    }

    /// One round for `askers`: each pass asked for, once, in order.
    fn round(&self, env: &HostEnv, askers: Vec<(UnixStream, DeployerRequest)>) {
        let mut promote = Vec::new();
        let mut canary = Vec::new();
        let mut previews = Vec::new();
        let mut not_built = Vec::new();
        for (stream, request) in askers {
            match request {
                DeployerRequest::Promote => promote.push(stream),
                DeployerRequest::Canary(project) if project.as_str() == self.project => {
                    canary.push(stream);
                }
                DeployerRequest::Canary(_) => not_built.push(stream),
                DeployerRequest::Previews => previews.push(stream),
            }
        }
        Self::answer(not_built, &DeployerAnswer::NotBuilt);
        if !promote.is_empty() {
            Self::answer(promote, &DeployerAnswer::Started);
            Progress("Carrying out the promotion the operator asked for").show();
            if let Err(error) = self.deployer.requested(env, self.production) {
                println!("warning: the promotion failed: {error}");
            }
        }
        if !canary.is_empty() {
            Self::answer(canary, &DeployerAnswer::Started);
            Progress("main moved: a canary pass").show();
            let pass = RunArgs {
                source: Source::Main,
                publish: self.publish,
            };
            if let Err(error) = self.deployer.delegate(env, &pass) {
                println!("warning: the canary pass failed: {error}");
            }
        }
        if !previews.is_empty() {
            self.pass(env, previews);
        }
    }

    /// One previews pass for `askers`, each answered once it has planned.
    fn pass(&self, env: &HostEnv, askers: Vec<UnixStream>) {
        let mut waiting = Some(askers);
        let ran = PreviewPass {
            deployer: self.deployer,
            production: self.production,
            publish: self.publish,
            project: self.project,
        }
        .run_planned(env, || {
            if let Some(askers) = waiting.take() {
                Self::answer(askers, &DeployerAnswer::Planned);
            }
        });
        if let Err(error) = ran {
            println!("warning: the previews pass failed: {error}");
            if let Some(askers) = waiting.take() {
                Self::answer(askers, &DeployerAnswer::Failed(error.to_string()));
            }
        }
    }

    /// Tells each asker `answer`; one that left is not told.
    fn answer(askers: Vec<UnixStream>, answer: &DeployerAnswer) {
        for mut asker in askers {
            let _ = asker.write_all(answer.line().as_bytes());
        }
    }

    /// Asks the deployer listening on `socket` for `request`, without
    /// waiting for it: whether one listens.
    pub fn nudge(socket: &Path, request: &DeployerRequest) -> bool {
        match UnixStream::connect(socket) {
            Ok(mut stream) => stream.write_all(request.line().as_bytes()).is_ok(),
            Err(_) => false,
        }
    }
}
