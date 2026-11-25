//! Deploying builds of Ren (docs/plans/deployments.md): building a commit
//! once in a container, starting it on a copy of production's data, the
//! canary's checks and promotion, the previews pass, the service that runs
//! them when asked, and production's install and upgrade. It does its I/O
//! openly: it runs cargo, Podman, systemd, Tailscale, and the built `ren`.

pub mod build_job;
pub mod builds;
pub mod canary;
pub mod daemon;
pub mod daemon_container;
pub mod file;
pub mod gvisor_service;
pub mod install;
pub mod invocation;
pub mod pod;
pub mod progress;
pub mod replica;
pub mod shot;
pub mod tailscale;
pub mod ui;
pub mod unit;

pub use canary::Deployer;
