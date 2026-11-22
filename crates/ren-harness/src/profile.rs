//! What Ren knows of each client as data: its program and the packages it
//! needs, what it takes mid-run, the approval policies it runs under, how it
//! takes images, how a launched session carries a user token, whether a
//! driven run names its default model, the user context it adds to prompts,
//! and the ways it signs in. Each client declares its own
//! [`ClientProfile`] in its module; [`Harness::profile`] picks it. Behaviour
//! that differs by client (its codec, its tools' effects, its headers) stays
//! in the client's module behind `match`.

use crate::{
    Harness,
    attachment::Takes,
    launch::LaunchPlan,
    settings::{ApprovalPolicy, Midway},
    sign_in::{Console, SignInFlow, SignInMethod, SignInPlan},
    tools::UserContext,
};
use ren_depot::package::{PackageId, Requirement};

/// One client's facts.
#[derive(Debug)]
pub struct ClientProfile {
    /// The program Ren runs to drive it.
    pub program: &'static str,
    /// The packages it needs on its host: its own program, and the runtime
    /// that program runs on, where it needs one.
    pub packages: &'static [PackageId],
    /// What a running conversation takes mid-run.
    pub midway: &'static [Midway],
    /// The approval policies it can be run under.
    pub approvals: &'static [ApprovalPolicy],
    /// Whether, and how, it takes images in its structured mode.
    pub images: Takes,
    /// How a launched session carries a user token to the gateway.
    pub token: TokenCarrier,
    /// What a driven run with no model chosen runs on.
    pub default_model: DefaultModel,
    /// What Ren tells the operator when it launches the client.
    pub notices: &'static [LaunchNotice],
    /// The ways it signs in through Ren, first the usual one.
    pub sign_in: SignIns,
    /// The prompt its login prints with no newline, so it begins the next
    /// line read; none where it prints none.
    pub login_prompt: Option<&'static str>,
    /// What it keeps under the home: its sign-in, settings, and sessions.
    /// A run that sees only what it needs sees these.
    pub keeps: &'static [HomeEntry],
    /// The context it adds to a user's prompt.
    pub(crate) user_context: UserContext,
}

/// A file or folder under the operator's home, by its path relative to it
/// (`.claude`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HomeEntry(&'static str);

impl HomeEntry {
    /// `relative`, a path under the home that does not leave it.
    pub const fn new(relative: &'static str) -> Self {
        Self(relative)
    }

    /// Where it is under `home`.
    pub fn under(self, home: &std::path::Path) -> std::path::PathBuf {
        home.join(self.0)
    }
}

/// How a launched session sends a user token to the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenCarrier {
    /// In the client's own custom-header variable, after any the user set
    /// (Claude Code's `ANTHROPIC_CUSTOM_HEADERS`).
    CustomHeader,
    /// In `REN_USER_TOKEN`, which the client's provider configuration or
    /// Ren's extension reads (Codex, Pi).
    Env,
    /// It sends none: its header configuration is not confirmed (opencode).
    None,
}

/// What a driven run with no model chosen runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultModel {
    /// The run names the client's default itself: Ren's override makes the
    /// client take another, so the listing's default is chosen (opencode).
    Named,
    /// The client's own choice, left to it.
    ClientOwn,
}

/// Something Ren tells the operator when it launches a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchNotice {
    /// Always: which of the client's providers Ren routes.
    Routes(&'static str),
    /// When the host sets `variable`, which the launch replaces.
    Replaces {
        variable: &'static str,
        advice: &'static str,
    },
}

/// The ways a client signs in through Ren, in order, each with its plan.
#[derive(Debug, Clone, Copy)]
pub struct SignIns {
    methods: &'static [SignInMethod],
    routes: &'static [SignInRoute],
}

/// One way a client signs in: its login command, what it runs on, and how
/// it goes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SignInRoute {
    pub(crate) method: SignInMethod,
    pub(crate) console: Console,
    pub(crate) flow: SignInFlow,
    pub(crate) command: fn() -> LaunchPlan,
}

/// The sign-in table of a client: each method once, with its console, flow,
/// and login command, written once for both the method list and the plans.
macro_rules! sign_ins {
    ($($method:ident => $console:ident, $flow:ident, $command:expr;)*) => {
        $crate::profile::SignIns::new(
            &[$($crate::sign_in::SignInMethod::$method),*],
            &[$($crate::profile::SignInRoute {
                method: $crate::sign_in::SignInMethod::$method,
                console: $crate::sign_in::Console::$console,
                flow: $crate::sign_in::SignInFlow::$flow,
                command: $command,
            }),*],
        )
    };
}
pub(crate) use sign_ins;

impl SignIns {
    pub(crate) const fn new(
        methods: &'static [SignInMethod],
        routes: &'static [SignInRoute],
    ) -> Self {
        Self { methods, routes }
    }

    /// The methods, first the usual one.
    pub fn methods(&self) -> &'static [SignInMethod] {
        self.methods
    }

    /// The plan for `method`, when the client signs in that way.
    pub fn plan(&self, method: SignInMethod) -> Option<SignInPlan> {
        self.routes
            .iter()
            .find(|route| route.method == method)
            .map(|route| SignInPlan {
                launch: (route.command)(),
                console: route.console,
                flow: route.flow,
            })
    }
}

impl ClientProfile {
    /// The packages it needs, with no version bound where the client
    /// documents none.
    pub fn requirements(&self) -> Vec<Requirement> {
        self.packages
            .iter()
            .cloned()
            .map(Requirement::any)
            .collect()
    }
}

impl Harness {
    /// What Ren knows of this client.
    pub fn profile(self) -> &'static ClientProfile {
        static CLAUDE_CODE: ClientProfile = ClientProfile::CLAUDE_CODE;
        static CODEX: ClientProfile = ClientProfile::CODEX;
        static PI: ClientProfile = ClientProfile::PI;
        static OPENCODE: ClientProfile = ClientProfile::OPENCODE;
        match self {
            Self::ClaudeCode => &CLAUDE_CODE,
            Self::Codex => &CODEX,
            Self::Pi => &PI,
            Self::OpenCode => &OPENCODE,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Harness;
    use ren_protocol::label::Labeled;

    /// A run in a container sees only what its client keeps: a client that
    /// keeps nothing would start there signed out.
    #[test]
    fn every_client_keeps_something_under_the_home() {
        let home = std::path::Path::new("/home/op");
        for harness in Harness::ALL {
            let keeps = harness.profile().keeps;
            assert!(!keeps.is_empty(), "{harness}");
            for entry in keeps {
                let path = entry.under(home);
                assert!(path.starts_with(home) && path != home, "{harness}: {path:?}");
            }
        }
    }

    #[test]
    fn every_sign_in_method_has_one_plan_in_its_order() {
        for harness in Harness::ALL {
            let sign_in = harness.profile().sign_in;
            let routed: Vec<_> = sign_in.routes.iter().map(|route| route.method).collect();
            assert_eq!(routed, sign_in.methods, "{harness}");
            for method in sign_in.methods {
                assert!(sign_in.plan(*method).is_some(), "{harness} {method}");
            }
        }
    }

    #[test]
    fn every_offered_policy_and_midway_is_listed_once() {
        for harness in Harness::ALL {
            let profile = harness.profile();
            let mut midway = profile.midway.to_vec();
            midway.dedup();
            assert_eq!(midway, profile.midway, "{harness}");
            let mut approvals = profile.approvals.to_vec();
            approvals.dedup();
            assert_eq!(approvals, profile.approvals, "{harness}");
        }
    }
}
