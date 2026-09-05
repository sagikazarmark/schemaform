//! Native tests of the `schemaform_daisyui` renderer package.
//!
//! They live here rather than beside the component so that the component directory — what a
//! `dx components add` install copies — ships no test code and no `dioxus-ssr` dependency. The
//! modules fall in two groups, kept apart so the second can move to the component's registry:
//!
//! **Adapter contract — stays with schemaform.** `contract` mounts a form through the package as
//! a real consumer and asserts what `schemaform-dioxus` promises every renderer: binding through
//! every seam, the ids it hands out resolving, finding visibility, the summary. It asserts on
//! bindings, ARIA relationships and the adapter's markers, never on daisyUI classes or widget
//! structure, so it fails when schemaform breaks a seam and not when daisyUI changes its output.
//!
//! **daisyUI presentation — moves with the component.** `mapping` drives the `dioxus-field`
//! bindings the component builds from the adapter's edit hooks, through a capturing renderer and
//! a `VirtualDom`, without rendering markup; it is the component's reusable contract with other
//! `dioxus-field` consumers and moves as it is. `controls`, `collection`, `shell`, and `findings`
//! observe the markup `dioxus-ssr` renders for a form bound through every seam, as a browser
//! would see it; the registry does not accept render-to-string tests, so they move as browser
//! specs against the component's examples.
//!
//! `support` is the harness both groups share. Everything browser-only — focus, live-region
//! announcements, DOM resynchronisation after a rejected write, the compound select's deferred
//! focus exit — is covered by the Playwright suite in `e2e/`, which draws the same line.

// Adapter contract: stays.
mod contract;

// daisyUI presentation: moves with the component.
mod collection;
mod controls;
mod findings;
mod mapping;
mod shell;

mod support;
