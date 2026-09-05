//! Small, focused examples mounted and quoted by the feature pages.

pub mod arrays;
pub mod daisyui;
pub mod daisyui_builtin;
pub mod editor;
pub mod generated;
pub mod minimal;
pub mod ui_schema;

/// Reports a form operation failure where a developer will see it.
///
/// `eprintln!` is a no-op on `wasm32-unknown-unknown`, so in the browser the failure goes to the
/// console; natively (the examples' own tests) it goes to stderr.
pub fn report_form_error(error: &schemaform_dioxus::HandleError) {
    let message = format!("form operation failed: {error}");
    #[cfg(target_arch = "wasm32")]
    web_sys::console::error_1(&message.into());
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{message}");
}
