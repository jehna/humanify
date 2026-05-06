pub mod http;

// TODO(task-5): trait JsonStrategy lives here

pub use http::{classify_error, HttpClient, StrategyError};
