//! Pi: what Ren knows of it, in one place. Its RPC mode and how Ren drives
//! it and lists its models (`rpc`); the extension Ren bundles for it and
//! its cache (`cache`); how it is launched through the gateway (`launch`);
//! the attribution its extension sends (`headers`); and its tools (`tools`).
//! The rest of Ren reads Pi through the shared types these fill.

mod cache;
mod headers;
mod launch;
mod rpc;
mod tools;

pub use cache::{PiCache, PiCacheError};
pub use rpc::{PiModels, PiRpc};
pub use tools::PiTool;
