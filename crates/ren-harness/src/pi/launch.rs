//! Launching Pi through the gateway: with Ren's extension, which sends its
//! providers there.

use crate::{
    Harness,
    endpoint::GatewayEndpoint,
    launch::{LaunchArg, LaunchPlan},
};
use std::collections::BTreeMap;

impl LaunchPlan {
    /// Pi with Ren's extension, which sends its providers to `gateway`.
    pub(crate) fn pi(gateway: &GatewayEndpoint) -> Self {
        let mut environment = BTreeMap::new();
        environment.insert("REN_PI_GATEWAY".into(), gateway.url("/").to_string().into());
        Self {
            harness: Harness::Pi,
            executable: "pi",
            arguments: vec!["--extension".into(), LaunchArg::PiExtension],
            environment,
        }
    }
}
