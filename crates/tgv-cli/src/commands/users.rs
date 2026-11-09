//! `tgv users`: the gateway's named users.
use crate::context::Context;
use anyhow::Result;
use clap::{Args, Subcommand};
use std::process::ExitCode;
use tgv_daemon::host::RandomIds;
use tgv_gateway::user::{UserToken, Users};
use tgv_recorder::caller::UserId;

/// Named users of the gateway, configured in `[[users]]`.
#[derive(Args)]
pub struct UsersCmd {
    #[command(subcommand)]
    action: UsersAction,
}

#[derive(Subcommand)]
enum UsersAction {
    /// Make a new token for a user. The token is printed once; the
    /// daemon's configuration keeps only its digest.
    Token(TokenArgs),
}

#[derive(Args)]
struct TokenArgs {
    /// The user's id: lowercase letters, digits, '-' or '_'.
    id: UserId,
}

impl UsersCmd {
    pub fn run(self, _ctx: &Context) -> Result<ExitCode> {
        match self.action {
            UsersAction::Token(args) => {
                let token = UserToken::generate(&RandomIds);
                print!("{}", TokenArgs::instructions(&args.id, &token));
                Ok(ExitCode::SUCCESS)
            }
        }
    }
}

impl TokenArgs {
    fn instructions(id: &UserId, token: &UserToken) -> String {
        format!(
            "Token for user {id} (shown once; the user's tools send it in {header}):\n\
             \n\
             {secret}\n\
             \n\
             Add to the daemon's tgv.toml, then restart it:\n\
             \n\
             [[users]]\n\
             id = \"{id}\"\n\
             token_blake3 = \"{digest}\"\n",
            header = Users::TOKEN_HEADER,
            secret = token.expose(),
            digest = token.digest(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::TokenArgs;
    use tgv_gateway::user::{UserConfig, UserToken};
    use tgv_recorder::{caller::UserId, id::SequentialIds};

    #[test]
    fn the_printed_configuration_names_the_tokens_digest() {
        let token = UserToken::generate(&SequentialIds::default());
        let text = TokenArgs::instructions(&UserId::parse("alice").unwrap(), &token);
        let (_, toml_part) = text.split_once("[[users]]\n").unwrap();
        let config: UserConfig = toml::from_str(toml_part).unwrap();
        assert_eq!(config.id.as_str(), "alice");
        assert_eq!(config.token_blake3, Some(token.digest()));
        assert!(text.contains(token.expose()));
    }
}
