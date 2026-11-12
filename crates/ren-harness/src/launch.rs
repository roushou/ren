//! Session-only client overrides. Authentication remains owned by the client.
use crate::{
    Harness,
    drive::{Opening, claude::ClaudeStreamJson, pi::PiRpc},
    endpoint::{GatewayEndpoint, UserToken},
    host::HostEnv,
    pi::{PiCache, PiCacheError},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::Path,
    process::{Command, ExitCode},
};

/// Why a client could not be launched.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error(transparent)]
    PiExtension(#[from] PiCacheError),
    #[error("cannot launch client; check it is installed on PATH")]
    Spawn(#[source] std::io::Error),
}

/// A harness Ren cannot drive through a structured mode yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0} cannot be driven by Ren yet")]
pub struct NotDrivable(pub Harness);

/// One launcher argument. The Pi extension path is resolved only when the
/// client is actually executed; dry runs show a placeholder instead.
#[derive(Clone)]
pub enum LaunchArg {
    Literal(String),
    PiExtension,
}

impl Serialize for LaunchArg {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Literal(value) => serializer.serialize_str(value),
            Self::PiExtension => serializer.serialize_str(PiCache::PLACEHOLDER),
        }
    }
}

impl From<&str> for LaunchArg {
    fn from(value: &str) -> Self {
        Self::Literal(value.into())
    }
}

impl From<String> for LaunchArg {
    fn from(value: String) -> Self {
        Self::Literal(value)
    }
}

/// One environment variable's value for the client. A secret is passed to
/// the client but shown as `<redacted>` in a dry run.
#[derive(Clone)]
pub enum EnvValue {
    Plain(String),
    Secret(String),
}

impl EnvValue {
    fn expose(&self) -> &str {
        match self {
            Self::Plain(value) | Self::Secret(value) => value,
        }
    }
}

impl Serialize for EnvValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Plain(value) => serializer.serialize_str(value),
            Self::Secret(_) => serializer.serialize_str("<redacted>"),
        }
    }
}

impl From<String> for EnvValue {
    fn from(value: String) -> Self {
        Self::Plain(value)
    }
}

impl From<&str> for EnvValue {
    fn from(value: &str) -> Self {
        Self::Plain(value.into())
    }
}

#[derive(Serialize)]
pub struct LaunchPlan {
    #[serde(skip)]
    harness: Harness,
    pub executable: &'static str,
    pub arguments: Vec<LaunchArg>,
    pub environment: BTreeMap<String, EnvValue>,
}

impl Harness {
    /// How to launch this harness through `gateway` for one session, without
    /// editing its files. Authentication stays with the client.
    pub fn launch(self, gateway: &GatewayEndpoint) -> LaunchPlan {
        let mut plan = LaunchPlan {
            harness: self,
            executable: "claude",
            arguments: vec![],
            environment: BTreeMap::new(),
        };
        match self {
            Self::Pi => {
                plan.executable = "pi";
                plan.arguments = vec!["--extension".into(), LaunchArg::PiExtension];
                plan.environment
                    .insert("REN_PI_GATEWAY".into(), gateway.url("/").to_string().into());
            }
            Self::ClaudeCode => {
                plan.environment.insert(
                    "ANTHROPIC_BASE_URL".into(),
                    gateway.url("/anthropic").to_string().into(),
                );
                plan.environment
                    .insert("CLAUDE_CODE_GATEWAY_HINT_HEADERS".into(), "1".into());
            }
            Self::Codex => {
                plan.executable = "codex";
                plan.arguments = vec!["--no-daemon".into()];
                plan.arguments.extend(Self::codex_provider(gateway));
            }
            Self::OpenCode => {
                plan.executable = "opencode";
                // opencode's documented runtime override; it merges over the
                // global and project files. Only the two providers' base URLs
                // are set: keys, sign-in, and every other provider stay
                // opencode's own. Its OpenAI WebSocket is refused with 426, and
                // opencode keeps the session on HTTP/SSE.
                let config = serde_json::json!({
                    "provider": {
                        "anthropic": {
                            "options": { "baseURL": gateway.url("/anthropic/v1").as_str() }
                        },
                        "openai": {
                            "options": { "baseURL": gateway.url("/openai/v1").as_str() }
                        },
                    }
                });
                plan.environment
                    .insert(Self::OPENCODE_CONFIG.into(), config.to_string().into());
            }
        }
        plan
    }

    /// Codex's configuration overrides that send its model traffic to
    /// `gateway`. They replace the entire provider table so an existing entry
    /// cannot accidentally add a bearer token or credential helper.
    /// WebSockets are offered: a daemon that refuses them answers 426, and
    /// Codex falls back to HTTP/SSE for the session. A user token is sent from
    /// the environment, when it is set.
    fn codex_provider(gateway: &GatewayEndpoint) -> [LaunchArg; 4] {
        let base = toml::Value::String(gateway.url("/codex").to_string());
        [
            "--config".into(),
            "model_provider=\"ren_session\"".into(),
            "--config".into(),
            format!(
                "model_providers.ren_session={{name=\"Ren\",base_url={base},wire_api=\"responses\",requires_openai_auth=true,supports_websockets=true,env_http_headers={{\"{header}\"=\"{variable}\"}}}}",
                header = UserToken::HEADER,
                variable = UserToken::ENV,
            )
            .into(),
        ]
    }

    /// The environment variable opencode reads inline configuration from.
    const OPENCODE_CONFIG: &'static str = "OPENCODE_CONFIG_CONTENT";

    /// The program Ren runs to drive this harness, when it can drive it.
    pub fn driven_program(self) -> Result<&'static str, NotDrivable> {
        match self {
            Self::Codex => Ok("codex"),
            Self::ClaudeCode => Ok("claude"),
            Self::Pi => Ok("pi"),
            Self::OpenCode => Err(NotDrivable(self)),
        }
    }

    /// How to start this harness in its structured mode, driven by Ren
    /// through `drive`, with its model traffic sent to `gateway`. The client
    /// keeps its own sign-in. Codex, Claude Code, and Pi can be driven so far.
    pub fn drive(
        self,
        gateway: &GatewayEndpoint,
        opening: &Opening,
    ) -> Result<LaunchPlan, NotDrivable> {
        match self {
            Self::Codex => {
                let mut arguments: Vec<LaunchArg> =
                    vec!["app-server".into(), "--listen".into(), "stdio://".into()];
                arguments.extend(Self::codex_provider(gateway));
                Ok(LaunchPlan {
                    harness: self,
                    executable: "codex",
                    arguments,
                    environment: BTreeMap::new(),
                })
            }
            Self::ClaudeCode => {
                // The same environment as a launched session: its traffic
                // goes through the gateway, which reads its hint headers.
                let mut plan = self.launch(gateway);
                plan.arguments = ClaudeStreamJson::arguments(opening)
                    .into_iter()
                    .map(LaunchArg::Literal)
                    .collect();
                Ok(plan)
            }
            Self::Pi => {
                // The same extension and gateway as a launched session.
                let mut plan = self.launch(gateway);
                plan.arguments.extend(
                    PiRpc::arguments(opening)
                        .into_iter()
                        .map(LaunchArg::Literal),
                );
                Ok(plan)
            }
            Self::OpenCode => Err(NotDrivable(self)),
        }
    }

    /// Whether a launched session can send a user token. opencode's header
    /// configuration is not yet confirmed, so it sends none.
    pub fn sends_user_token(self) -> bool {
        match self {
            Self::ClaudeCode | Self::Codex | Self::Pi => true,
            Self::OpenCode => false,
        }
    }
}

impl LaunchPlan {
    /// The plan with `token` sent to the gateway on every request. Claude
    /// Code takes it as a custom header, after any the user set; Codex and
    /// the Pi extension read it from the environment.
    #[must_use]
    pub fn presenting(mut self, token: &UserToken, env: &HostEnv) -> Self {
        let header = format!("{}: {}", UserToken::HEADER, token.expose());
        match self.harness {
            Harness::ClaudeCode => {
                let headers = match env.get(Self::CUSTOM_HEADERS) {
                    Some(existing) if !existing.trim().is_empty() => {
                        format!("{existing}\n{header}")
                    }
                    Some(_) | None => header,
                };
                self.environment
                    .insert(Self::CUSTOM_HEADERS.into(), EnvValue::Secret(headers));
            }
            Harness::Codex | Harness::Pi => {
                self.environment.insert(
                    UserToken::ENV.into(),
                    EnvValue::Secret(token.expose().into()),
                );
            }
            Harness::OpenCode => {}
        }
        self
    }

    /// Claude Code's extra request headers, one `Name: Value` per line.
    const CUSTOM_HEADERS: &'static str = "ANTHROPIC_CUSTOM_HEADERS";

    /// Whether the client loads Ren's Pi extension, whose path the command
    /// then needs.
    pub fn loads_pi_extension(&self) -> bool {
        self.arguments
            .iter()
            .any(|argument| matches!(argument, LaunchArg::PiExtension))
    }

    /// Build the client command. `extension` replaces the Pi extension
    /// placeholder; without it the placeholder text is passed literally.
    pub fn command(&self, extension: Option<&Path>, args: &[OsString]) -> Command {
        let mut command = Command::new(self.executable);
        for argument in &self.arguments {
            match (argument, extension) {
                (LaunchArg::Literal(value), _) => command.arg(value),
                (LaunchArg::PiExtension, Some(path)) => command.arg(path),
                (LaunchArg::PiExtension, None) => command.arg(PiCache::PLACEHOLDER),
            };
        }
        command.args(args).envs(
            self.environment
                .iter()
                .map(|(name, value)| (name, value.expose())),
        );
        command
    }

    /// Replace this process with the client. Returns only on failure, or
    /// (off Unix) with the client's exit status.
    pub fn execute(&self, args: &[OsString], env: &HostEnv) -> Result<ExitCode, LaunchError> {
        let extension = match self.harness {
            Harness::Pi => {
                eprintln!(
                    "Ren routes Pi's anthropic, openai, and openai-codex providers; other providers keep their own endpoints."
                );
                Some(PiCache::locate(env)?.materialize()?)
            }
            Harness::OpenCode => {
                if env.is_set(Harness::OPENCODE_CONFIG) {
                    eprintln!(
                        "Ren replaces {} for this session; keep other settings in opencode.json.",
                        Harness::OPENCODE_CONFIG
                    );
                }
                eprintln!(
                    "Ren routes opencode's anthropic and openai providers; other providers keep their own endpoints."
                );
                None
            }
            Harness::ClaudeCode | Harness::Codex => None,
        };
        let mut command = self.command(extension.as_deref(), args);
        // Replacing the launcher preserves the terminal, signals, and exact exit
        // status without a supervisor or shell. Pi's immutable cache survives
        // /reload and process replacement; there are no temporary files to clean.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            Err(LaunchError::Spawn(command.exec()))
        }
        #[cfg(not(unix))]
        {
            let status = command.status().map_err(LaunchError::Spawn)?;
            Ok(ExitCode::from(
                status
                    .code()
                    .and_then(|code| u8::try_from(code).ok())
                    .unwrap_or(1),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GatewayEndpoint, Harness, HostEnv, LaunchArg, OsString, Path, PiCache};
    use crate::{drive::Opening, endpoint::UserToken};
    use ren_protocol::native::NativeThreadId;

    const SESSION: &str = "0190f3a0-0000-7000-8000-00000000000a";

    fn opening() -> Opening {
        Opening::New {
            cwd: "/work/acme".into(),
            id: NativeThreadId::new(uuid::Uuid::parse_str(SESSION).unwrap()).unwrap(),
        }
    }

    #[test]
    fn drives_codex_app_server_over_stdio_through_the_gateway() {
        let gateway = GatewayEndpoint::parse("http://127.0.0.1:8787").unwrap();
        let driven = Harness::Codex.drive(&gateway, &opening()).unwrap();
        let launched = Harness::Codex.launch(&gateway);
        let command = driven.command(None, &[]);
        assert_eq!(command.get_program(), "codex");
        assert_eq!(Harness::Codex.driven_program(), Ok("codex"));
        let arguments: Vec<_> = command.get_args().collect();
        assert_eq!(arguments[..3], ["app-server", "--listen", "stdio://"]);
        // The same provider overrides as an interactive session.
        let interactive = launched.command(None, &[]);
        let overrides: Vec<_> = interactive.get_args().skip(1).collect();
        assert_eq!(arguments[3..], overrides[..]);
        assert_eq!(command.get_envs().count(), 0);
        assert_eq!(
            Harness::OpenCode.drive(&gateway, &opening()).err(),
            Some(super::NotDrivable(Harness::OpenCode))
        );
        assert_eq!(
            Harness::OpenCode.driven_program(),
            Err(super::NotDrivable(Harness::OpenCode))
        );
    }

    #[test]
    fn drives_pi_in_rpc_mode_with_rens_extension_through_the_gateway() {
        let gateway = GatewayEndpoint::parse("http://127.0.0.1:8787").unwrap();
        let driven = Harness::Pi.drive(&gateway, &opening()).unwrap();
        assert_eq!(Harness::Pi.driven_program(), Ok("pi"));
        assert!(driven.loads_pi_extension());
        assert!(
            !Harness::Codex
                .drive(&gateway, &opening())
                .unwrap()
                .loads_pi_extension()
        );
        let command = driven.command(Some(Path::new("/cache/ren.ts")), &[]);
        assert_eq!(command.get_program(), "pi");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--extension",
                "/cache/ren.ts",
                "--mode",
                "rpc",
                "--session-id",
                SESSION
            ]
        );
        // The same environment as a launched session.
        let launched = Harness::Pi.launch(&gateway).command(None, &[]);
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            launched.get_envs().collect::<Vec<_>>()
        );
    }

    #[test]
    fn drives_claude_code_in_stream_json_print_mode_through_the_gateway() {
        let gateway = GatewayEndpoint::parse("http://127.0.0.1:8787").unwrap();
        let command = Harness::ClaudeCode
            .drive(&gateway, &opening())
            .unwrap()
            .command(None, &[]);
        assert_eq!(command.get_program(), "claude");
        assert_eq!(Harness::ClaudeCode.driven_program(), Ok("claude"));
        let arguments: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_str().unwrap())
            .collect();
        assert_eq!(
            arguments,
            [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--replay-user-messages",
                "--permission-prompt-tool",
                "stdio",
                "--session-id",
                SESSION,
            ]
        );
        // The same environment as an interactive session: the gateway and
        // its hint headers, and nothing that signs in or names the client.
        let launched = Harness::ClaudeCode.launch(&gateway).command(None, &[]);
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            launched.get_envs().collect::<Vec<_>>()
        );
        let resumed = Harness::ClaudeCode
            .drive(
                &gateway,
                &Opening::Resume {
                    thread: NativeThreadId::new(uuid::Uuid::parse_str(SESSION).unwrap()).unwrap(),
                    cwd: "/work/acme".into(),
                },
            )
            .unwrap()
            .command(None, &[]);
        let tail: Vec<_> = resumed.get_args().skip(10).collect();
        assert_eq!(tail, ["--resume", SESSION]);
    }

    #[test]
    fn keeps_arguments_literal_and_auth_out_of_overrides() {
        let gateway = GatewayEndpoint::parse("http://[::1]:8787").unwrap();
        for harness in [
            Harness::ClaudeCode,
            Harness::Codex,
            Harness::Pi,
            Harness::OpenCode,
        ] {
            let plan = harness.launch(&gateway);
            let argument = OsString::from("$(touch /tmp/never) ' ; \n");
            let command = plan.command(None, std::slice::from_ref(&argument));
            assert_eq!(command.get_args().last(), Some(argument.as_os_str()));
            assert!(
                command
                    .get_envs()
                    .all(|(key, _)| key == "ANTHROPIC_BASE_URL"
                        || key == "CLAUDE_CODE_GATEWAY_HINT_HEADERS"
                        || key == "REN_PI_GATEWAY"
                        || key == "OPENCODE_CONFIG_CONTENT")
            );
            if harness == Harness::Codex {
                let Some(LaunchArg::Literal(last)) = plan.arguments.last() else {
                    unreachable!("codex provider override must be a literal argument");
                };
                let config: toml::Value = toml::from_str(last).unwrap();
                let provider = &config["model_providers"]["ren_session"];
                assert_eq!(provider["requires_openai_auth"].as_bool(), Some(true));
                assert_eq!(provider["supports_websockets"].as_bool(), Some(true));
                assert_eq!(
                    provider["base_url"].as_str(),
                    Some("http://[::1]:8787/codex")
                );
                assert_eq!(
                    provider["env_http_headers"]["x-ren-user-token"].as_str(),
                    Some("REN_USER_TOKEN")
                );
            }
        }
    }

    #[test]
    fn opencode_gets_only_base_urls_for_its_native_providers() {
        let gateway = GatewayEndpoint::parse("https://ren.example.ts.net").unwrap();
        let plan = Harness::OpenCode.launch(&gateway);
        assert_eq!(plan.executable, "opencode");
        assert!(plan.arguments.is_empty());
        let json = serde_json::to_value(&plan).unwrap();
        let config: serde_json::Value = serde_json::from_str(
            json["environment"]["OPENCODE_CONFIG_CONTENT"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        // No key, header, or other provider: authentication stays opencode's,
        // and nothing claims to be another client.
        assert_eq!(
            config,
            serde_json::json!({
                "provider": {
                    "anthropic": {
                        "options": { "baseURL": "https://ren.example.ts.net/anthropic/v1" }
                    },
                    "openai": {
                        "options": { "baseURL": "https://ren.example.ts.net/openai/v1" }
                    },
                }
            })
        );
        assert_eq!(json["environment"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn pi_dry_run_and_execution_share_one_argument_list() {
        let gateway = GatewayEndpoint::parse("http://[::1]:8787").unwrap();
        let plan = Harness::Pi.launch(&gateway);
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            json["arguments"],
            serde_json::json!(["--extension", PiCache::PLACEHOLDER])
        );
        assert!(json.get("harness").is_none());
        let path = Path::new("/cache/ren/pi/ren-x.ts");
        let command = plan.command(Some(path), &[OsString::from("--model")]);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["--extension", "/cache/ren/pi/ren-x.ts", "--model"]);
    }

    #[test]
    fn a_user_token_reaches_each_client_and_never_a_dry_run() {
        let gateway = GatewayEndpoint::parse("http://[::1]:8787").unwrap();
        let token = UserToken::from_random(7, 9);
        let env = HostEnv::from_pairs([("ANTHROPIC_CUSTOM_HEADERS", "X-Team: a")]);
        let sent = |harness: Harness| {
            let plan = harness.launch(&gateway).presenting(&token, &env);
            let dry_run = serde_json::to_string(&plan).unwrap();
            assert!(!dry_run.contains(token.expose()), "{harness:?}");
            plan.command(None, &[])
                .get_envs()
                .filter_map(|(name, value)| {
                    Some((name.to_str()?.to_owned(), value?.to_str()?.to_owned()))
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let claude = sent(Harness::ClaudeCode);
        assert_eq!(
            claude["ANTHROPIC_CUSTOM_HEADERS"],
            format!("X-Team: a\nx-ren-user-token: {}", token.expose())
        );
        for harness in [Harness::Codex, Harness::Pi] {
            assert_eq!(
                sent(harness)["REN_USER_TOKEN"],
                token.expose(),
                "{harness:?}"
            );
        }
        assert!(!Harness::OpenCode.sends_user_token());
        assert!(
            sent(Harness::OpenCode)
                .values()
                .all(|value| !value.contains(token.expose()))
        );
    }
}
