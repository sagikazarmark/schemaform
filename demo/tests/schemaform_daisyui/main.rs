//! Native tests of the `schemaform_daisyui` renderer package.
//!
//! They live here rather than beside the component so that the component directory — what a
//! `dx components add` install copies — ships no test code and no `dioxus-ssr` dependency. Two
//! kinds of test share this crate:
//!
//! - `mapping` drives the `dioxus-field` bindings the component builds from the adapter's edit
//!   hooks, through a capturing renderer and a `VirtualDom`, without rendering markup. These are
//!   the component's reusable contract with other `dioxus-field` consumers.
//! - `controls`, `collection`, `shell`, and `findings` mount a form through every daisyUI seam
//!   and observe the markup `dioxus-ssr` renders for it, as a browser would see it.
//!
//! Everything browser-only — focus, live-region announcements, DOM resynchronisation after a
//! rejected write, the compound select's deferred focus exit — is covered by the Playwright suite
//! in `e2e/`.

mod collection;
mod controls;
mod findings;
mod mapping;
mod shell;
mod support;
