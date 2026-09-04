use dioxus::prelude::*;
use dioxus_code::{Code, code};

use crate::app::Route;
use crate::components::{ExampleSection, InlineCode, PageHeader, snippet_theme};
use crate::examples::daisyui_builtin::BuiltinComparisonExample;
use crate::pages::daisyui::DaisyuiVariants;

#[component]
pub fn DaisyuiBuiltin() -> Element {
    rsx! {
        PageHeader {
            eyebrow: "Renderers",
            title: "Unstyled built-in comparison",
            intro: "The daisyUI form's definition and baseline through the built-in renderer alone, with no theme applied, for comparing behaviour and accessibility side by side with the daisyUI page.",
        }
        DaisyuiVariants { current: Route::DaisyuiBuiltin {} }
        ExampleSection {
            title: "The same form, unthemed",
            intro: rsx! {
                "The example shares "
                InlineCode { "definition()" }
                " and "
                InlineCode { "baseline_form_data()" }
                " with the daisyUI page and binds them through "
                InlineCode { "ControlRegistry::with_builtins()" }
                ", extended so the two daisyUI widget symbols the UI schema names resolve to the built-in control renderer. The "
                InlineCode { "data-schemaform-unstyled" }
                " wrapper opts the form out of the demo's daisyUI theme, which otherwise styles the built-in class hooks site-wide, and restores the browser's own form-control defaults that Tailwind's preflight resets. The tabs and groups are the same built-in code path as on the daisyUI page; the arrays, the shell, and the finding summary are the built-in renderers the daisyUI page replaces through the structure and presenter seams. Item identity, focus after array mutations, live-region announcements, and summary focus-to-target are the adapter's on both pages, so they behave identically; what differs is who renders the chrome."
            },
            demo: rsx! { BuiltinComparisonExample {} },
            code: rsx! {
                Code { src: code!("src/examples/daisyui_builtin.rs"), theme: snippet_theme() }
            },
        }
    }
}
