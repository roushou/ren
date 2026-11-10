//! `ren mcp`: serve Ren's history to coding agents as an MCP server over
//! stdio. It reads the targeted daemon through its read-only management API,
//! like `ren tail`, so it works the same against this host's daemon or a
//! gateway on the tailnet. Requests are answered one at a time, in order.
//!
//! Stdout carries only protocol messages; logs go to stderr.
use crate::commands::history::TargetArgs;
use crate::context::Context;
use crate::management::Management;
use crate::mcp::{BodyExcerpt, McpServer, PendingCall, Step, ToolCall, ToolOutcome};
use anyhow::Result;
use clap::Args;
use serde_json::Value;
use std::process::ExitCode;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Stdin, Stdout};

/// Serve recorded history to coding agents over MCP (stdio).
#[derive(Args)]
pub struct McpCmd {
    #[command(flatten)]
    target: TargetArgs,
}

/// One line of input.
enum Line {
    Message(Vec<u8>),
    /// Longer than the limit; its bytes were discarded.
    Oversized,
    End,
}

impl McpCmd {
    /// The longest message read, newline included.
    const LINE_LIMIT: u64 = 1 << 20;

    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let gateway = self.target.gateway(ctx)?;
        tracing::info!(gateway = gateway.root(), "MCP server reading from stdin");
        let mut server = McpServer::new(crate::Cli::build().version());
        let mut input = BufReader::new(tokio::io::stdin());
        let mut output = tokio::io::stdout();
        loop {
            let reply = match Self::next_line(&mut input).await? {
                Line::End => return Ok(ExitCode::SUCCESS),
                Line::Oversized => server.oversized(),
                Line::Message(line) if line.iter().all(u8::is_ascii_whitespace) => continue,
                Line::Message(line) => match server.handle(&line) {
                    Step::Reply(reply) => reply,
                    Step::Run(pending) => Self::execute(&gateway, pending).await,
                    Step::Silent => continue,
                },
            };
            Self::write(&mut output, &reply).await?;
        }
    }

    async fn execute(gateway: &Management, pending: PendingCall) -> Value {
        let outcome = match &pending.tool {
            ToolCall::Read(request) => gateway.serve(request).await.map(Ok),
            ToolCall::GetBody {
                call,
                direction,
                budget,
            } => {
                let budget = *budget;
                gateway
                    .preview(*call, *direction)
                    .await
                    .map(|preview| BodyExcerpt::new(preview, budget).map(BodyExcerpt::into_value))
            }
        };
        pending.answer(match outcome {
            Ok(Ok(data)) => ToolOutcome::Data(data),
            Ok(Err(_)) => {
                ToolOutcome::Failed("the gateway's body preview has an unexpected shape".to_owned())
            }
            Err(error) => ToolOutcome::Failed(format!("{:#}", anyhow::Error::from(error))),
        })
    }

    /// The next line, up to [`Self::LINE_LIMIT`] bytes; a longer one is read
    /// to its end and discarded.
    async fn next_line(input: &mut BufReader<Stdin>) -> Result<Line> {
        let mut line = Vec::new();
        let read = (&mut *input)
            .take(Self::LINE_LIMIT)
            .read_until(b'\n', &mut line)
            .await?;
        if read == 0 {
            return Ok(Line::End);
        }
        if line.last() == Some(&b'\n') || u64::try_from(read)? < Self::LINE_LIMIT {
            return Ok(Line::Message(line));
        }
        loop {
            line.clear();
            let read = (&mut *input)
                .take(Self::LINE_LIMIT)
                .read_until(b'\n', &mut line)
                .await?;
            if read == 0 || line.last() == Some(&b'\n') {
                return Ok(Line::Oversized);
            }
        }
    }

    async fn write(output: &mut Stdout, message: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        output.write_all(&line).await?;
        output.flush().await?;
        Ok(())
    }
}
