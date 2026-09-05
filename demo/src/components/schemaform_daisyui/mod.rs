//! daisyUI presentation for `schemaform-dioxus`: a control renderer, a structure
//! bundle (collection and shell), and a finding presenter.
//!
//! See `README.md` beside this file for the component's scope, layout, and the
//! mapping it performs. The component's tests live in the crate's `tests/`
//! directory, so this directory ships no test code.

mod boolean;
mod choice;
mod collection;
mod component;
mod constant;
mod findings;
mod mapping;
mod parts;
mod shell;
mod text;

pub use collection::DaisyuiCollection;
pub use component::*;
pub use findings::{DaisyuiFindings, findings};
pub use mapping::*;
pub use shell::DaisyuiShell;
