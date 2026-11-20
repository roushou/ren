//! The `[github]` section: which repository each project is watched on,
//! the token it is read with, and how often. `ren_github` tests the client
//! and the poller themselves. A fixture token only.
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use ren_daemon::config::{Config, ConfigError, GithubOptions, WatchedRepository};
use ren_harness::host::HostEnv;
use std::time::Duration;

const TOKEN: &str = "github_pat_FIXTURE";

fn options(endpoint: String) -> GithubOptions {
    GithubOptions {
        endpoint,
        repository: vec![WatchedRepository {
            project: "acme-web".parse().unwrap(),
            github: "acme/web".parse().unwrap(),
        }],
        ..GithubOptions::default()
    }
}

fn config(endpoint: String) -> Config {
    Config {
        github: options(endpoint),
        ..Config::default()
    }
}

#[test]
fn a_watch_needs_its_token_and_a_safe_endpoint() {
    let endpoint = || GithubOptions::default().endpoint;
    let missing = config(endpoint()).resolve_with(&HostEnv::default());
    assert!(
        matches!(missing, Err(ConfigError::MissingGithubToken(name)) if name == "REN_GITHUB_TOKEN")
    );
    // Checking the structure alone does not need it.
    config(endpoint()).check().unwrap();

    let insecure = config("http://github.example/graphql".into()).validate();
    assert!(matches!(insecure, Err(ConfigError::GithubEndpoint(_))));

    let mut twice = config(endpoint());
    let first = twice.github.repository[0].clone();
    twice.github.repository.push(first);
    assert!(matches!(
        twice.validate(),
        Err(ConfigError::GithubProjectTwice(_))
    ));

    // Nothing named, nothing watched, and no token needed.
    let off = Config::default().resolve_with(&HostEnv::default()).unwrap();
    assert!(off.github().is_none());
}

#[test]
fn the_github_section_reads_from_toml() {
    let config: Config = toml::from_str(
        "[github]\ntoken_env = \"ACME_TOKEN\"\npoll_seconds = 120\n\n[[github.repository]]\nproject = \"acme-web\"\ngithub = \"acme/web\"\n",
    )
    .unwrap();
    config.validate().unwrap();
    let runtime = config
        .resolve_with(&HostEnv::from_pairs([("ACME_TOKEN", TOKEN)]))
        .unwrap();
    let setup = runtime.github().unwrap();
    assert_eq!(setup.period.duration(), Duration::from_secs(120));
    assert_eq!(
        setup.projects,
        [("acme-web".parse().unwrap(), "acme/web".parse().unwrap())]
    );
    assert!(!format!("{setup:?}").contains(TOKEN));
    assert!(toml::from_str::<Config>("[github]\npoll_seconds = 5\n").is_err());
}
