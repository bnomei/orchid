//! Orchid task-file orchestration library.
//!
//! Coordinates scoped agent work from repo-local specs: leases, packets, Git
//! attribution, and JSON ACKs for coordinator scripts. The public entry is
//! [`run`], which parses CLI arguments and dispatches command handlers.

mod cli;
mod core;
mod gitstate;
mod goal;
mod model;
mod orchestration;
mod paths;
mod planner;
mod runtime;
mod specs;
mod taskfile;

pub use cli::run;
