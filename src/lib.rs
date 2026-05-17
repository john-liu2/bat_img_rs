// lib.rs — exposes internal modules for integration tests.
// The binary entry point (main.rs) remains separate.
pub mod cli;
pub mod error;
pub mod exif;
pub mod heic;
pub mod pipeline;
pub mod processor;
