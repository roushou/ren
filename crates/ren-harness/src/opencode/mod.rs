//! opencode: what Ren knows of it, in one place. How it is launched through
//! the gateway (`launch`); the attribution its requests carry (`headers`);
//! and its tools (`tools`). Ren cannot drive it through a structured mode
//! yet. The rest of Ren reads opencode through the shared types these fill.

mod headers;
mod launch;
mod tools;

pub use tools::OpenCodeTool;
