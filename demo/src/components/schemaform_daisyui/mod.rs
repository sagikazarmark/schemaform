//! daisyUI presentation for `schemaform-dioxus` text controls.
//!
//! See `README.md` beside this file for the component's scope, layout, and the
//! mapping it performs.

mod component;
mod mapping;

pub use component::*;
// The mapping is this component's reusable surface for other `dioxus-field`
// consumers; the gallery itself only needs `controls()`.
#[allow(unused_imports)]
pub use mapping::*;
