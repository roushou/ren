//! `ren users`: the gateway's named users.
use crate::context::Context;
use anyhow::Result;
use clap::{Args, Subcommand};
use ren_daemon::host::RandomIds;
use ren_gateway::user::{TokenDigest, Users};
use ren_harness::endpoint::UserToken;
use ren_recorder::caller::UserId;
use std::process::ExitCode;

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
                let token = Users::new_token(&RandomIds);
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
             Add to the daemon's ren.toml, then restart it:\n\
             \n\
             [[users]]\n\
             id = \"{id}\"\n\
             token_blake3 = \"{digest}\"\n",
            header = UserToken::HEADER,
            secret = token.expose(),
            digest = TokenDigest::of(token),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::TokenArgs;
    use ren_gateway::user::{TokenDigest, UserConfig, Users};
    use ren_recorder::{caller::UserId, id::SequentialIds};

    #[test]
    fn the_printed_configuration_names_the_tokens_digest() {
        let token = Users::new_token(&SequentialIds::default());
        let text = TokenArgs::instructions(&UserId::parse("alice").unwrap(), &token);
        let (_, toml_part) = text.split_once("[[users]]\n").unwrap();
        let config: UserConfig = toml::from_str(toml_part).unwrap();
        assert_eq!(config.id.as_str(), "alice");
        assert_eq!(config.token_blake3, Some(TokenDigest::of(&token)));
        assert!(text.contains(token.expose()));
    }
}
