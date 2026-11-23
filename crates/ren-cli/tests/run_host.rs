//! The run host, as the built `ren` runs it: it holds a client through the
//! daemon going away and coming back, keeps every byte the client wrote,
//! and leaves the client's exit for a daemon that was not there to see it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and the test spawns the tasks a daemon's supervisor would poll"
)]

use ren_daemon::host::{HeldRuns, Launcher};
use ren_recorder::id::RunId;
use ren_supervisor::{AgentProcess, ProcessExit};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// A daemon attached to a run: it writes `line` and reads the client's
/// answer, then goes away, leaving the exit unwatched.
async fn talk(agent: AgentProcess, line: &str) -> String {
    let AgentProcess {
        mut stdin,
        stdout,
        exit,
        ..
    } = agent;
    // The exit future carries stdin to the host, as the supervisor polls it.
    let pumping = tokio::spawn(exit);
    stdin
        .write_all(format!("{line}\n").as_bytes())
        .await
        .unwrap();
    stdin.flush().await.unwrap();
    let mut answer = String::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        BufReader::new(stdout).read_line(&mut answer),
    )
    .await
    .expect("the client answers")
    .unwrap();
    pumping.abort();
    answer
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_host_holds_its_client_while_no_daemon_watches() {
    // Under /tmp: a socket's path holds at most 104 bytes on macOS, and its
    // own temporary folder (`/var/folders/…/T/`) leaves too few for
    // `runs/<run>/host.sock`.
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    let held = HeldRuns::new(
        dir.path().join("runs"),
        Launcher::Direct,
        env!("CARGO_BIN_EXE_ren").into(),
    );
    let run = RunId::restore(uuid::Uuid::from_u128(7));
    let mut command = std::process::Command::new("sh");
    command
        .args(["-c", "while read line; do echo \"got $line\"; done"])
        .current_dir(dir.path());
    let started = held.start(run, &command, &[]).unwrap();
    let first = talk(started, "one").await;
    assert_eq!(first, "got one\n");

    // The daemon went away; the next one reads on from where it stopped.
    let listed = held.list().unwrap();
    assert_eq!(
        listed.iter().map(|held| held.run).collect::<Vec<_>>(),
        [run]
    );
    let read = u64::try_from(first.len()).unwrap();
    let second = talk(held.rejoin(run, read).unwrap(), "two").await;
    assert_eq!(second, "got two\n");
    // From the start, everything the client wrote is there.
    assert_eq!(held.spool(run, 0, 100).unwrap(), b"got one\ngot two\n");

    // Killed through a third daemon: the exit comes after the output.
    let agent = held.rejoin(run, 16).unwrap();
    // The output is read to its end, as the supervisor reads it: the exit
    // comes once it is.
    let mut stdout = agent.stdout;
    let draining = tokio::spawn(async move {
        let mut rest = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stdout, &mut rest)
            .await
            .unwrap();
        rest
    });
    agent.kill.fire();
    let exit = tokio::time::timeout(Duration::from_secs(10), agent.exit)
        .await
        .expect("the client exits");
    assert_eq!(exit, ProcessExit::Signal(9));
    assert!(
        draining.await.unwrap().is_empty(),
        "nothing more was written"
    );

    // The host has gone; what it left still answers a daemon.
    tokio::time::timeout(Duration::from_secs(10), async {
        while dir.path().join(format!("runs/{run}/host.sock")).exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the host ends once its client's exit is told");
    let left = held.rejoin(run, 0).unwrap();
    let mut spooled = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut { left.stdout }, &mut spooled)
        .await
        .unwrap();
    assert_eq!(spooled, b"got one\ngot two\n");
    assert_eq!(left.exit.await, ProcessExit::Signal(9));
    held.release(run);
    assert!(!dir.path().join(format!("runs/{run}")).exists());
}
