//! `tgv clients`: the gateway's named clients.
use crate::context::Context;
use anyhow::Result;
use clap::{Args, Subcommand};
use std::process::ExitCode;
use tgv_daemon::host::RandomIds;
use tgv_gateway::client::{ClientToken, Clients};
use tgv_recorder::client::ClientId;

/// Named clients of the gateway, configured in `[[clients]]`.
#[derive(Args)]
pub struct ClientsCmd {
    #[command(subcommand)]
    action: ClientsAction,
}

#[derive(Subcommand)]
enum ClientsAction {
    /// Make a new token for a client. The token is printed once; the
    /// daemon's configuration keeps only its digest.
    Token(TokenArgs),
}

#[derive(Args)]
struct TokenArgs {
    /// The client's id: lowercase letters, digits, '-' or '_'.
    id: ClientId,
}

impl ClientsCmd {
    pub fn run(self, _ctx: &Context) -> Result<ExitCode> {
        match self.action {
            ClientsAction::Token(args) => {
                let token = ClientToken::generate(&RandomIds);
                print!("{}", TokenArgs::instructions(&args.id, &token));
                Ok(ExitCode::SUCCESS)
            }
        }
    }
}

impl TokenArgs {
    fn instructions(id: &ClientId, token: &ClientToken) -> String {
        format!(
            "Token for client {id} (shown once; give it to the client, which sends it in {header}):\n\
             \n\
             {secret}\n\
             \n\
             Add to the daemon's tgv.toml, then restart it:\n\
             \n\
             [[clients]]\n\
             id = \"{id}\"\n\
             token_blake3 = \"{digest}\"\n",
            header = Clients::TOKEN_HEADER,
            secret = token.expose(),
            digest = token.digest(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::TokenArgs;
    use tgv_gateway::client::{ClientConfig, ClientToken};
    use tgv_recorder::{client::ClientId, id::SequentialIds};

    #[test]
    fn the_printed_configuration_names_the_tokens_digest() {
        let token = ClientToken::generate(&SequentialIds::default());
        let text = TokenArgs::instructions(&ClientId::parse("alice").unwrap(), &token);
        let (_, toml_part) = text.split_once("[[clients]]\n").unwrap();
        let config: ClientConfig = toml::from_str(toml_part).unwrap();
        assert_eq!(config.id.as_str(), "alice");
        assert_eq!(config.token_blake3, Some(token.digest()));
        assert!(text.contains(token.expose()));
    }
}
