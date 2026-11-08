//! Session-only client overrides. Authentication remains owned by the client.
use crate::{
    Harness,
    endpoint::GatewayEndpoint,
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

#[derive(Serialize)]
pub struct LaunchPlan {
    #[serde(skip)]
    harness: Harness,
    pub executable: &'static str,
    pub arguments: Vec<LaunchArg>,
    pub environment: BTreeMap<String, String>,
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
                    .insert("TGV_PI_GATEWAY".into(), gateway.url("/").to_string());
            }
            Self::ClaudeCode => {
                plan.environment.insert(
                    "ANTHROPIC_BASE_URL".into(),
                    gateway.url("/anthropic").to_string(),
                );
                plan.environment
                    .insert("CLAUDE_CODE_GATEWAY_HINT_HEADERS".into(), "1".into());
            }
            Self::Codex => {
                plan.executable = "codex";
                // Replace the entire provider table so an existing entry cannot
                // accidentally add a bearer token or credential helper.
                // WebSockets are offered: a daemon that refuses them answers
                // 426, and Codex falls back to HTTP/SSE for the session.
                let base = toml::Value::String(gateway.url("/codex").to_string());
                plan.arguments = vec![
                    "--no-daemon".into(),
                    "--config".into(),
                    "model_provider=\"tgv_session\"".into(),
                    "--config".into(),
                    format!(
                        "model_providers.tgv_session={{name=\"TGV\",base_url={base},wire_api=\"responses\",requires_openai_auth=true,supports_websockets=true}}"
                    )
                    .into(),
                ];
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
                    .insert(Self::OPENCODE_CONFIG.into(), config.to_string());
            }
        }
        plan
    }

    /// The environment variable opencode reads inline configuration from.
    const OPENCODE_CONFIG: &'static str = "OPENCODE_CONFIG_CONTENT";
}

impl LaunchPlan {
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
        command.args(args).envs(&self.environment);
        command
    }

    /// Replace this process with the client. Returns only on failure, or
    /// (off Unix) with the client's exit status.
    pub fn execute(&self, args: &[OsString], env: &HostEnv) -> Result<ExitCode, LaunchError> {
        let extension = match self.harness {
            Harness::Pi => {
                eprintln!(
                    "TGV routes Pi's anthropic, openai, and openai-codex providers; other providers keep their own endpoints."
                );
                Some(PiCache::locate(env)?.materialize()?)
            }
            Harness::OpenCode => {
                if env.is_set(Harness::OPENCODE_CONFIG) {
                    eprintln!(
                        "TGV replaces {} for this session; keep other settings in opencode.json.",
                        Harness::OPENCODE_CONFIG
                    );
                }
                eprintln!(
                    "TGV routes opencode's anthropic and openai providers; other providers keep their own endpoints."
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
    use super::{GatewayEndpoint, Harness, LaunchArg, OsString, Path, PiCache};
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
                        || key == "TGV_PI_GATEWAY"
                        || key == "OPENCODE_CONFIG_CONTENT")
            );
            if harness == Harness::Codex {
                let Some(LaunchArg::Literal(last)) = plan.arguments.last() else {
                    unreachable!("codex provider override must be a literal argument");
                };
                let config: toml::Value = toml::from_str(last).unwrap();
                let provider = &config["model_providers"]["tgv_session"];
                assert_eq!(provider["requires_openai_auth"].as_bool(), Some(true));
                assert_eq!(provider["supports_websockets"].as_bool(), Some(true));
                assert_eq!(
                    provider["base_url"].as_str(),
                    Some("http://[::1]:8787/codex")
                );
            }
        }
    }

    #[test]
    fn opencode_gets_only_base_urls_for_its_native_providers() {
        let gateway = GatewayEndpoint::parse("https://tgv.example.ts.net").unwrap();
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
                        "options": { "baseURL": "https://tgv.example.ts.net/anthropic/v1" }
                    },
                    "openai": {
                        "options": { "baseURL": "https://tgv.example.ts.net/openai/v1" }
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
        let path = Path::new("/cache/tgv/pi/tgv-x.ts");
        let command = plan.command(Some(path), &[OsString::from("--model")]);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["--extension", "/cache/tgv/pi/tgv-x.ts", "--model"]);
    }
}
