//! The control API's bodies: what `/control/v1` answers and streams. Like the
//! read API's (`api::read`), each is the gateway's own type, converted once
//! from the supervisor, store, harness, or operator type it reports with every
//! field named, so a change there does not reach the API until it is made
//! here too. ts-rs writes them to the UI (`ui/apps/web/src/lib/wire`).

pub mod history;
pub mod operator;
pub mod runs;
pub mod sessions;
pub mod sign_in;
#[cfg(test)]
mod tests;
pub mod tools;
