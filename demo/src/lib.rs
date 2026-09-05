//! Schemaform docs-by-example application.
//!
//! The crate is a library so its integration tests under `tests/` can mount the gallery's
//! components — in particular the `schemaform_daisyui` renderer package, whose tests live there
//! rather than beside the component so that the component directory ships no test code. The
//! binary in `main.rs` only launches [`app::App`].

pub mod app;
pub mod components;
pub mod examples;
pub mod pages;
