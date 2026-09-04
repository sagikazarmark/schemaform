//! daisyUI presentation for `schemaform-dioxus` controls.
//!
//! See `README.md` beside this file for the component's scope, layout, and the
//! mapping it performs.

mod boolean;
mod choice;
mod component;
mod constant;
mod mapping;
mod parts;
mod text;

pub use component::*;
// The mapping is this component's reusable surface for other `dioxus-field`
// consumers; the gallery itself only needs `controls()`.
#[allow(unused_imports)]
pub use mapping::*;
