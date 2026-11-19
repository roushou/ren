//! What the task does for the clients themselves: which are installed
//! and signed in, their sign-ins, and the commands and models their
//! listing processes report.

use super::listing::{CommandKey, ListingError, ListingProbe, ListingReply, Listings};
use super::login::{SignInError, SignIns, StatusCheck};
use super::{Live, Served, SupervisorTask};
use crate::{
    CommandsAsked, CommandsError, SignInRequest, StartError, StartPlace,
    client::{Client, ClientStatus},
    host::AgentHost,
    workdir::{RequestedDir, Workdir},
};
use futures_util::{FutureExt, future::BoxFuture};
use ren_harness::sign_in::Console;
use ren_harness::{
    Harness,
    commands::{ClientCommand, CommandListing},
    models::{Model, ModelListing},
    settings::{ApprovalPolicy, Midway},
};
use ren_protocol::label::Labeled;

impl<H: AgentHost> SupervisorTask<H> {
    /// Whether `harness`'s program is on this host's path.
    pub(super) fn installed(&self, harness: Harness) -> bool {
        self.host.finds(harness.driven_program())
    }

    /// Every client as this host has it, with its sign-in as last read.
    pub(super) fn clients(&self, sign_ins: &SignIns) -> Vec<Client> {
        Harness::ALL
            .iter()
            .map(|harness| Client {
                harness: *harness,
                status: match self.installed(*harness) {
                    true => ClientStatus::Ready,
                    false => ClientStatus::NotInstalled,
                },
                approvals: ApprovalPolicy::offered(*harness).to_vec(),
                midway: Midway::offered(*harness).to_vec(),
                sign_in: sign_ins.state(*harness),
                sign_in_methods: harness.sign_in_methods().to_vec(),
            })
            .collect()
    }

    /// Check the sign-in of each installed client that offers a status
    /// command and has not been checked.
    pub(super) fn check_unchecked(&self, sign_ins: &mut SignIns) {
        for harness in Harness::ALL {
            let unchecked = matches!(
                sign_ins.state(*harness),
                crate::sign_in::SignInState::NotChecked
            );
            if unchecked && !harness.sign_in_status().is_empty() && self.installed(*harness) {
                sign_ins.check(*harness, None, || self.status_check(*harness));
            }
        }
    }

    /// The first root, where a client's own short commands run.
    pub(super) fn root_workdir(&self) -> Result<Workdir, SignInError> {
        let root = self
            .bounds
            .roots()
            .iter()
            .next()
            .ok_or(SignInError::NoRoot)?;
        Workdir::confirm(&self.bounds, root.to_path_buf()).map_err(|_| SignInError::NoRoot)
    }

    /// Start `harness`'s status commands, read when they end.
    pub(super) fn status_check(
        &self,
        harness: Harness,
    ) -> Result<BoxFuture<'static, crate::sign_in::SignInState>, SignInError> {
        let plans = harness.sign_in_status();
        if !self.installed(harness) {
            return Err(SignInError::NotInstalled(harness));
        }
        let workdir = self.root_workdir()?;
        let tools = self.host.tools();
        let processes = plans
            .iter()
            .map(|plan| self.host.spawn_with_stderr(plan, &workdir, &tools))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                tracing::debug!(%harness, %error, "cannot start a client's status command");
                SignInError::Spawn
            })?;
        Ok(StatusCheck::run(harness, processes).boxed())
    }

    /// Carry out a sign-in request.
    pub(super) fn sign_in(&self, request: SignInRequest, sign_ins: &mut SignIns) {
        match request {
            SignInRequest::Check { harness, reply } => {
                sign_ins.check(harness, Some(reply), || self.status_check(harness));
            }
            SignInRequest::Start {
                harness,
                method,
                reply,
            } => {
                let started = harness
                    .sign_in(method)
                    .map_err(SignInError::from)
                    .and_then(|plan| {
                        if !self.installed(harness) {
                            return Err(SignInError::NotInstalled(harness));
                        }
                        let workdir = self.root_workdir()?;
                        let tools = self.host.tools();
                        let process = match plan.console {
                            Console::Pipes => {
                                self.host.spawn_with_stderr(&plan.launch, &workdir, &tools)
                            }
                            Console::Terminal => {
                                self.host.spawn_in_terminal(&plan.launch, &workdir, &tools)
                            }
                        };
                        process
                            .map(|process| (process, plan.console, plan.flow))
                            .map_err(|error| {
                                tracing::debug!(%harness, %error, "cannot start a client's login");
                                SignInError::Spawn
                            })
                    })
                    .map(|(process, console, flow)| {
                        sign_ins.login(harness, process, console, flow, self.line_limit)
                    });
                let _ = reply.send(started);
            }
            SignInRequest::Answer {
                harness,
                answer,
                reply,
            } => sign_ins.answer(
                harness,
                answer,
                |address| self.host.open_local(address),
                reply,
            ),
            SignInRequest::Cancel { harness, reply } => {
                let _ = reply.send(sign_ins.cancel(harness));
            }
            SignInRequest::Wait { harness, reply } => {
                let _ = reply.send(sign_ins.wait(harness));
            }
        }
    }

    /// Keep what a running client lists, when it lists something new.
    pub(super) fn book(
        live: &mut Live,
        commands: &mut Listings<CommandKey, Vec<ClientCommand>, CommandsError>,
    ) {
        let listed = live.process.commands();
        if listed.is_empty() || listed == live.booked.as_slice() {
            return;
        }
        if let Some(key) = &live.book {
            live.booked = listed.to_vec();
            commands.keep(key.clone(), live.booked.clone());
        }
    }

    /// Answer what a session's client, or a client in a folder, offers a
    /// prompt: a running client's own list, what the installed client last
    /// listed in that folder, or a listing started now.
    pub(super) fn commands(
        &self,
        asked: CommandsAsked,
        reply: ListingReply<Vec<ClientCommand>, CommandsError>,
        served: &mut Served,
    ) {
        let (harness, folder) = match asked {
            CommandsAsked::Session(session) => {
                let Some(serving) = served.sessions.get(&session) else {
                    let _ = reply.send(Err(CommandsError::NoSuchSession(session)));
                    return;
                };
                let running = served
                    .process(session)
                    .and_then(|run| served.runs.get(&run))
                    .map(|live| live.process.commands())
                    .filter(|listed| !listed.is_empty());
                if let Some(listed) = running {
                    let _ = reply.send(Ok(listed.to_vec()));
                    return;
                }
                (serving.harness(), serving.workdir().clone())
            }
            CommandsAsked::Client { harness, place } => match self.listing_folder(place) {
                Ok(folder) => (harness, folder),
                Err(error) => {
                    let _ = reply.send(Err(CommandsError::Place(error)));
                    return;
                }
            },
        };
        let Some(stamp) = self
            .host
            .client_stamp(harness.profile().program, &self.host.tools())
        else {
            let _ = reply.send(Err(CommandsError::NotInstalled(harness)));
            return;
        };
        let key = CommandKey {
            harness,
            stamp,
            folder: folder.as_path().to_path_buf(),
        };
        let Some(reply) = served.commands.ask(&key, reply) else {
            return;
        };
        match self.command_probe(harness, &folder) {
            Ok(listing) => served.commands.start(key, reply, listing),
            Err(error) => {
                let _ = reply.send(Err(error.into()));
            }
        }
    }

    /// The folder a listing runs in for `place`: a folder in bounds, or the
    /// scratch folder itself, never a new one.
    pub(super) fn listing_folder(&self, place: StartPlace) -> Result<Workdir, StartError> {
        match place {
            // A workspace's client lists what the repository's does.
            StartPlace::Folder(dir) | StartPlace::Worktree(dir) => {
                self.confirm(&dir.parse::<RequestedDir>().map_err(StartError::Workdir)?)
            }
            StartPlace::Scratch => {
                let scratch = self.bounds.scratch().ok_or(StartError::NoScratch)?;
                Workdir::confirm(&self.bounds, scratch.to_path_buf()).map_err(StartError::Workdir)
            }
        }
    }

    /// A listing process of `harness` in `folder`, asked for its commands.
    pub(super) fn command_probe(
        &self,
        harness: Harness,
        folder: &Workdir,
    ) -> Result<BoxFuture<'static, Result<Vec<ClientCommand>, ListingError>>, ListingError> {
        let plan = harness.list_models(&self.gateway);
        let (mut listing, first) =
            CommandListing::open(harness, &self.version, &folder.as_path().to_string_lossy());
        let process = self
            .host
            .spawn(&plan, folder, &self.host.tools())
            .map_err(|error| {
                tracing::debug!(%harness, %error, "cannot start a client to list its commands");
                ListingError::Spawn
            })?;
        Ok(ListingProbe::run(
            move |frame| listing.receive(frame),
            first,
            process,
            self.line_limit,
        )
        .boxed())
    }

    /// List the models of every client installed here, with no one waiting.
    pub(super) fn warm(&self, listings: &mut Listings<Harness, Vec<Model>>) {
        for harness in Harness::ALL {
            if !self.installed(*harness) {
                continue;
            }
            match self.probe(*harness) {
                Ok(listing) => listings.warm(*harness, listing),
                Err(error) => tracing::debug!(%harness, %error, "cannot list a client's models"),
            }
        }
    }

    /// Answer from the listings, or start the client briefly in the first
    /// root to list its models.
    pub(super) fn models(
        &self,
        harness: Harness,
        reply: ListingReply<Vec<Model>>,
        listings: &mut Listings<Harness, Vec<Model>>,
    ) {
        let Some(reply) = listings.ask(&harness, reply) else {
            return;
        };
        match self.probe(harness) {
            Ok(listing) => listings.start(harness, reply, listing),
            Err(error) => {
                let _ = reply.send(Err(error));
            }
        }
    }

    pub(super) fn probe(
        &self,
        harness: Harness,
    ) -> Result<BoxFuture<'static, Result<Vec<Model>, ListingError>>, ListingError> {
        let plan = harness.list_models(&self.gateway);
        let root = self
            .bounds
            .roots()
            .iter()
            .next()
            .ok_or(ListingError::NoRoot)?;
        let workdir =
            Workdir::confirm(&self.bounds, root.to_path_buf()).map_err(|_| ListingError::NoRoot)?;
        let (mut listing, first) =
            ModelListing::open(harness, &self.version, &workdir.as_path().to_string_lossy());
        let process = self
            .host
            .spawn(&plan, &workdir, &self.host.tools())
            .map_err(|error| {
                tracing::debug!(%harness, %error, "cannot start a client to list its models");
                ListingError::Spawn
            })?;
        Ok(ListingProbe::run(
            move |frame| listing.receive(frame),
            first,
            process,
            self.line_limit,
        )
        .boxed())
    }
}
