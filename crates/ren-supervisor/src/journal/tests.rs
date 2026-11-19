use super::{Command, Entry, Replay, ReplayError, Start};
use crate::process::{Process, ProcessEffect, Rest, Serving};
use crate::run::{RunCommand, RunState};
use crate::workdir::Workdir;
use ren_harness::Harness;
use ren_harness::drive::{Driver, Opening};
use ren_harness::lines::{LineLimit, Lines};
use ren_harness::prompt::{KeptPrompt, Prompt};
use ren_harness::settings::RunSettings;
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::{RunId, SessionId};
use serde_json::{Value, json};

const THREAD: &str = "0190f3a0-0000-7000-8000-000000000001";
const TURN: &str = "0190f3a0-0000-7000-8000-000000000002";

fn uuid(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0190_f3a0_0000_7000_8000_0000_0000_0000 | n)
}

fn start(first: Option<&Prompt>) -> Start {
    Start {
        run: RunId::restore(uuid(10)),
        session: SessionId::restore(uuid(11)),
        harness: Harness::Codex,
        workdir: "/work/acme".into(),
        settings: RunSettings::default(),
        opening: Opening::New {
            cwd: "/work/acme".into(),
            id: NativeThreadId::new(uuid(1)).unwrap(),
            settings: RunSettings::default(),
        },
        version: "0.1.0".into(),
        seed: uuid(2),
        first: first.map(KeptPrompt::from),
    }
}

/// A process as the daemon runs it: what it is told goes into its journal
/// and its client's output into the spool, as the task writes them.
struct Daemon {
    process: Process,
    lines: Lines,
    journal: Vec<u8>,
    spool: Vec<u8>,
    sent: Vec<Value>,
}

impl Daemon {
    fn start(first: Option<&Prompt>) -> Self {
        let start = start(first);
        let (codec, opening) = Driver::open(
            start.harness,
            start.opening.clone(),
            &start.version,
            start.seed,
        );
        let serving = Serving {
            run: start.run,
            session: start.session,
            harness: start.harness,
            workdir: Workdir::journaled(&start.workdir).unwrap(),
            settings: start.settings.clone(),
        };
        let journal = Entry::Started(Box::new(start)).line().unwrap();
        let first = first.cloned();
        let (process, effects) = Process::opening(serving, codec, opening, first);
        let mut daemon = Self {
            process,
            lines: Lines::new(LineLimit::DEFAULT),
            journal,
            spool: Vec::new(),
            sent: Vec::new(),
        };
        daemon.sent(&effects);
        daemon
    }

    fn sent(&mut self, effects: &[ProcessEffect]) {
        for effect in effects {
            if let ProcessEffect::Write(line) = effect {
                self.sent
                    .push(serde_json::from_slice(line.as_bytes()).unwrap());
            }
        }
    }

    /// The client writes `bytes`, in whatever pieces.
    fn output(&mut self, bytes: &[u8]) -> Vec<ProcessEffect> {
        self.spool.extend_from_slice(bytes);
        let effects: Vec<_> = self
            .lines
            .push(bytes)
            .into_iter()
            .flat_map(|frame| self.process.receive(frame))
            .collect();
        let upto = u64::try_from(self.spool.len()).unwrap();
        self.journal.extend(Entry::Read { upto }.line().unwrap());
        self.sent(&effects);
        effects
    }

    fn says(&mut self, message: &Value) -> Vec<ProcessEffect> {
        self.output(format!("{message}\n").as_bytes())
    }

    fn command(&mut self, command: RunCommand) {
        self.journal
            .extend(Entry::Command(Command::from(&command)).line().unwrap());
        let effects = self.process.command(command).unwrap();
        self.sent(&effects);
    }

    /// The client answers every request it was sent and has not answered.
    fn answer(&mut self) {
        for _ in 0..4 {
            let asked = std::mem::take(&mut self.sent);
            if asked.is_empty() {
                break;
            }
            for request in asked {
                let result = match request["method"].as_str() {
                    Some("thread/start") => json!({ "thread": { "id": THREAD } }),
                    Some("turn/start") => json!({ "turn": { "id": TURN, "status": "inProgress" } }),
                    Some(_) | None => json!({}),
                };
                if request.get("id").is_some() {
                    self.says(&json!({ "id": request["id"], "result": result }));
                }
            }
        }
    }

    /// A daemon restarted now: the process rebuilt from the journal and
    /// the spool.
    fn restarted(&self) -> Result<Daemon, ReplayError> {
        let entries = Entry::read(&self.journal)?;
        let Some(Entry::Started(start)) = entries.first() else {
            return Err(ReplayError::NoStart);
        };
        let mut replay = Replay::begin(start, LineLimit::DEFAULT)?;
        for entry in entries.iter().skip(1) {
            if let Entry::Read { upto } = entry {
                let from = usize::try_from(replay.read()).unwrap();
                let to = usize::try_from(*upto).unwrap().min(self.spool.len());
                // The spool is read back in other pieces than it came in.
                for piece in self.spool[from..to].chunks(7) {
                    replay.output(piece);
                }
            }
            replay.entry(entry)?;
        }
        let replayed = replay.finish();
        assert_eq!(replayed.read, u64::try_from(self.spool.len()).unwrap());
        Ok(Daemon {
            process: replayed.process,
            lines: replayed.lines,
            journal: self.journal.clone(),
            spool: self.spool.clone(),
            sent: Vec::new(),
        })
    }
}

/// `text` with a 2×1 PNG: a prompt is journaled with its images' bytes.
fn with_image(text: &str) -> Prompt {
    use ren_harness::attachment::{Attachment, Limits};
    use ren_harness::prompt::PromptImage;
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(2u32.to_be_bytes());
    bytes.extend(1u32.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    let read = Attachment::parse("shot.png", &bytes, &Limits::default()).unwrap();
    let image = PromptImage::new(read, "/data/attachments/s/a/shot.png".into(), bytes.into());
    Prompt::with_images(text.into(), vec![image]).unwrap()
}

/// The two processes are the same: their state, their codec, and what
/// they hold.
fn same(left: &Process, right: &Process) {
    assert_eq!(format!("{left:?}"), format!("{right:?}"));
}

#[test]
fn a_replayed_process_is_the_one_the_daemon_left() {
    let mut daemon = Daemon::start(None);
    daemon.answer();
    assert!(matches!(daemon.process.state(), RunState::Idle { .. }));
    daemon.command(RunCommand::Prompt(with_image("fix the order list")));
    daemon.answer();
    assert!(matches!(daemon.process.state(), RunState::Working { .. }));
    // Half a message is in flight when the daemon stops.
    let item = json!({ "method": "item/completed", "params": { "threadId": THREAD, "turnId": TURN,
        "item": { "type": "agentMessage", "id": "msg_1", "text": "Sorted by date now." } } })
    .to_string();
    let (head, tail) = item.split_at(20);
    daemon.output(head.as_bytes());

    let mut restarted = daemon.restarted().unwrap();
    same(&restarted.process, &daemon.process);

    // From here on, both see the rest of it and the turn's end alike.
    let ended = json!({ "method": "turn/completed", "params": { "threadId": THREAD,
        "turn": { "id": TURN, "status": "completed" } } });
    for daemon in [&mut daemon, &mut restarted] {
        daemon.output(format!("{tail}\n").as_bytes());
        daemon.says(&ended);
    }
    same(&restarted.process, &daemon.process);
    assert!(matches!(restarted.process.state(), RunState::Idle { .. }));
}

#[test]
fn a_held_first_prompt_and_a_refused_command_replay_alike() {
    let mut daemon = Daemon::start(Some(&Prompt::try_from("start".to_owned()).unwrap()));
    // Refused while opening: replayed, it is refused again.
    daemon.journal.extend(
        Entry::Command(Command::from(&RunCommand::Interrupt))
            .line()
            .unwrap(),
    );
    let _refused = daemon.process.command(RunCommand::Interrupt);
    daemon.journal.extend(
        Entry::RestOver {
            rest: Rest::default(),
        }
        .line()
        .unwrap(),
    );
    let _stale = daemon.process.rest_over(Rest::default());
    daemon.answer();
    same(&daemon.restarted().unwrap().process, &daemon.process);
}

#[test]
fn a_journal_cut_short_or_out_of_order_is_read_as_far_as_it_holds() {
    let mut daemon = Daemon::start(None);
    daemon.answer();
    // A line the daemon was writing as it stopped is not an entry yet.
    let whole = daemon.journal.len();
    daemon.journal.extend(br#"{"entry":"read","up"#);
    assert_eq!(
        Entry::read(&daemon.journal).unwrap().len(),
        Entry::read(&daemon.journal[..whole]).unwrap().len()
    );
    daemon.restarted().unwrap();

    let mut wrong = Daemon::start(None);
    wrong
        .journal
        .extend(Entry::Read { upto: 99 }.line().unwrap());
    assert!(matches!(
        wrong.restarted(),
        Err(ReplayError::SpoolShort { read: 0, upto: 99 })
    ));
    assert!(matches!(
        Entry::read(b"{\"entry\":\"read\",\"upto\":1}\n"),
        Ok(entries) if entries == [Entry::Read { upto: 1 }]
    ));
    assert!(matches!(
        Entry::read(b"nonsense\n"),
        Err(ReplayError::Unreadable { line: 1, .. })
    ));
}
