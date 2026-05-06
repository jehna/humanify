//! Shared DSL for integration / e2e tests.
//!
//! Full judge wiring lands in a later task; this skeleton just declares
//! the types and methods the e2e test files will use.

#![allow(dead_code, unused_variables)]

pub fn given(_input_path: &str) -> Scenario {
    unimplemented!("populated in e2e task")
}

pub struct Scenario;
pub struct Outcome;
