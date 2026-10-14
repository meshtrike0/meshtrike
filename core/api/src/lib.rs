//! The Core (section 13): engine, public types, host interfaces, and the JSON
//! method surface shared by the Android (UniFFI) and browser (WASM) bindings.
#![forbid(unsafe_code)]

pub mod engine;
pub mod host;
pub mod integrity;
pub mod log;
pub mod lora;
pub mod records;
pub mod rpc;
pub mod types;

pub use engine::{CoreConfig, Engine, Host};
pub use host::*;
pub use types::*;
