use dioxus::prelude::*;

use crate::app::Route;
use crate::components::{DocsCallout, ExternalAction, PageHeader};

#[component]
pub fn Home() -> Element {
    let groups = [
        (
            "Generated controls",
            "Compile JSON Schema into scalar controls with validation, presence, and submission semantics.",
            Route::Generated {},
        ),
        (
            "Homogeneous arrays",
            "Add, remove, and reorder scalar or fixed-object items with stable item identity.",
            Route::Arrays {},
        ),
        (
            "Authored UI schema",
            "Arrange controls with plain text, responsive grids, groups, and tabs.",
            Route::Presentation {},
        ),
        (
            "Schema playground",
            "Edit a data schema and UI schema side by side with the rendered form.",
            Route::Playground {},
        ),
    ];

    rsx! {
        PageHeader {
            eyebrow: "Schemaform",
            title: "Runtime JSON Schema forms for Dioxus",
            intro: "Compile trusted Draft 2020-12 data schemas into framework-neutral form state, then render accessible browser controls with the Dioxus adapter. Every page below mounts a working example and shows the exact source behind it.",
        }

        div { class: "mt-10 grid gap-4 sm:grid-cols-2",
            for (title , blurb , route) in groups {
                Link {
                    to: route,
                    class: "group rounded-2xl border border-base-300 bg-base-100 p-5 shadow-sm transition-colors hover:border-primary/40 hover:bg-base-200/40",
                    p { class: "font-semibold tracking-tight group-hover:text-primary", "{title}" }
                    p { class: "mt-1 text-sm leading-6 text-base-content/65", "{blurb}" }
                }
            }
        }

        div { class: "mt-10 grid gap-4 sm:grid-cols-3",
            Feature { value: "Draft 2020-12", label: "data schema" }
            Feature { value: "UI schema v1", label: "presentation" }
            Feature { value: "Browser CSR", label: "Dioxus adapter" }
        }

        DocsCallout {
            title: "Capability profile",
            action: Some(ExternalAction::new(
                "Read the support profile",
                "https://github.com/sagikazarmark/schemaform/blob/main/docs/support-profile.md",
            )),
            "The first release deliberately supports a finite editable profile. Unsupported or ambiguous constructs produce explicit capability findings instead of guessed controls."
        }
    }
}

#[component]
fn Feature(#[props(into)] value: String, #[props(into)] label: String) -> Element {
    rsx! {
        div { class: "rounded-2xl border border-base-300 bg-base-200/35 p-4",
            p { class: "font-mono text-sm font-semibold text-primary", "{value}" }
            p { class: "mt-1 text-xs uppercase tracking-wider text-base-content/50", "{label}" }
        }
    }
}
