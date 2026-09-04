//! Presentation components used by the demo application.
//!
//! Two kinds of module live here. The documentation UI (`code`, `common`,
//! `examples`, `layout`, `nav`) is hand-written for this gallery. The
//! `dx components` members (`button`, `checkbox`, `field`, `input`,
//! `native_select`, `radio_group`, `select`, `schemaform_daisyui`) are laid out
//! the way `dx components add` installs them: one directory per component with
//! a `mod.rs`, importable as `crate::components::<name>`.
//!
//! `button`, `checkbox`, `field`, `input`, `native_select`, `radio_group`, and
//! `select` are copied verbatim from the `dioxus-daisyui-components` registry at
//! the revision `schemaform_daisyui`'s manifest pins. They are committed rather
//! than installed in CI, so `dx components add` never runs in the pipeline.

mod code;
mod common;
mod examples;
mod layout;
mod nav;

// The registry components are copied verbatim, and the gallery does not use
// every value of every axis they offer.
#[allow(dead_code)]
pub mod button;
#[allow(dead_code)]
pub mod checkbox;
#[allow(dead_code)]
pub mod field;
#[allow(dead_code)]
pub mod input;
#[allow(dead_code)]
pub mod native_select;
#[allow(dead_code)]
pub mod radio_group;
pub mod schemaform_daisyui;
#[allow(dead_code)]
pub mod select;

pub use code::*;
pub use common::*;
pub use examples::*;
pub use layout::*;
pub use nav::*;
