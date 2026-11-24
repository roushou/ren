//! The factory as a service (docs/plans/instances.md, phase 3): it listens
//! on a Unix socket and runs a previews pass for each ask, one at a time,
//! so a preview asked for starts now, not at the canary timer's next pass.
//! Asks that arrive while a pass runs are taken together by the next one.
//! Each asker is answered once the pass planned which previews run
//! ([`FactoryAnswer::Planned`]), or that it failed before it did; the pass
//! then goes on building and starting them.

use super::{CanaryError, Factory, PublishArgs, previews::PreviewPass};
use crate::{progress::Progress, replica::ProductionPlace};
use ren_gateway::factory::{FactoryAnswer, FactoryRequest};
use ren_harness::host::HostEnv;
use std::{
    io::{BufRead as _, BufReader, ErrorKind, Write as _},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

/// The factory listening for asks.
pub(super) struct FactoryService<'a> {
    pub factory: &'a Factory,
    pub production: &'a ProductionPlace,
    pub publish: PublishArgs,
    pub project: &'a str,
}

impl FactoryService<'_> {
    /// Its socket's folder, under the user's runtime folder.
    const FOLDER: &str = "ren-factory";
    /// How long an asker has to say what it asks.
    const ASK_WAIT: Duration = Duration::from_secs(2);

    /// Where the factory listens: what `[factory] socket` names.
    pub fn socket(env: &HostEnv) -> Option<PathBuf> {
        env.absolute_path("XDG_RUNTIME_DIR")
            .map(|runtime| runtime.join(Self::FOLDER).join("factory.sock"))
    }

    /// Listens on `socket` until stopped.
    pub fn serve(&self, env: &HostEnv, socket: &Path) -> Result<(), CanaryError> {
        let listener = Self::listen(socket)?;
        Progress(&format!("The factory listens on {}", socket.display())).show();
        loop {
            let first = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) => {
                    println!("warning: an ask was not taken: {error}");
                    continue;
                }
            };
            let askers = Self::askers(&listener, first);
            if askers.is_empty() {
                continue;
            }
            self.pass(env, askers);
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

    /// `first` and every ask already waiting, each that asked for the
    /// previews; one that asked for something else is told so.
    fn askers(listener: &UnixListener, first: UnixStream) -> Vec<UnixStream> {
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
                Some(FactoryRequest::Previews) => Some(stream),
                None => {
                    Self::answer(
                        vec![stream],
                        &FactoryAnswer::Failed("not a request the factory takes".into()),
                    );
                    None
                }
            })
            .collect()
    }

    fn request(stream: &UnixStream) -> Option<FactoryRequest> {
        stream.set_nonblocking(false).ok()?;
        stream.set_read_timeout(Some(Self::ASK_WAIT)).ok()?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).ok()?;
        FactoryRequest::read(&line)
    }

    /// One previews pass for `askers`, each answered once it has planned.
    fn pass(&self, env: &HostEnv, askers: Vec<UnixStream>) {
        let mut waiting = Some(askers);
        let ran = PreviewPass {
            factory: self.factory,
            production: self.production,
            publish: self.publish,
            project: self.project,
        }
        .run_planned(env, || {
            if let Some(askers) = waiting.take() {
                Self::answer(askers, &FactoryAnswer::Planned);
            }
        });
        if let Err(error) = ran {
            println!("warning: the previews pass failed: {error}");
            if let Some(askers) = waiting.take() {
                Self::answer(askers, &FactoryAnswer::Failed(error.to_string()));
            }
        }
    }

    /// Tells each asker `answer`; one that left is not told.
    fn answer(askers: Vec<UnixStream>, answer: &FactoryAnswer) {
        for mut asker in askers {
            let _ = asker.write_all(answer.line().as_bytes());
        }
    }

    /// Asks the factory listening on `socket` for a pass, without waiting
    /// for it: whether one listens.
    pub fn nudge(socket: &Path) -> bool {
        match UnixStream::connect(socket) {
            Ok(mut stream) => stream
                .write_all(FactoryRequest::Previews.line().as_bytes())
                .is_ok(),
            Err(_) => false,
        }
    }
}
