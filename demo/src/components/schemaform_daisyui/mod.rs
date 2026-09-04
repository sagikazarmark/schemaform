//! daisyUI presentation for `schemaform-dioxus`: a control renderer, a structure
//! bundle (collection and shell), and a finding presenter.
//!
//! See `README.md` beside this file for the component's scope, layout, and the
//! mapping it performs.

mod boolean;
mod choice;
mod collection;
mod component;
mod constant;
mod findings;
mod mapping;
mod parts;
mod shell;
#[cfg(test)]
mod test_support;
mod text;

pub use component::*;
pub use findings::findings;
// The renderer types let a host compose this component's slots with another
// package's; the mapping is its reusable surface for other `dioxus-field`
// consumers. The gallery itself only needs `controls()`, `structure()`, and
// `findings()`.
#[allow(unused_imports)]
pub use collection::DaisyuiCollection;
#[allow(unused_imports)]
pub use findings::DaisyuiFindings;
#[allow(unused_imports)]
pub use mapping::*;
#[allow(unused_imports)]
pub use shell::DaisyuiShell;
