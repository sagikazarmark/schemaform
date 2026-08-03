use dioxus::prelude::*;
use dioxus_code::{Code, code};

use crate::components::{ExampleLayout, ExampleSection, InlineCode, PageHeader, snippet_theme};
use crate::examples::generated::GeneratedControlsExample;
use crate::examples::minimal::MinimalExample;

#[component]
pub fn Generated() -> Element {
    rsx! {
        PageHeader {
            eyebrow: "Generated forms",
            title: "Controls, validation, and submission",
            intro: "Omit the UI schema to generate a complete form from the declared properties of a fixed object. Canonical form data stays in the core while the Dioxus adapter renders semantic browser controls.",
        }
        ExampleSection {
            title: "The basic path",
            intro: rsx! {
                InlineCode { "FormDefinition::compile" }
                " generates the definition tree. "
                InlineCode { "use_form" }
                " owns browser-reactive state, and the default render configuration binds accessible built-in controls."
            },
            demo: rsx! { MinimalExample {} },
            code: rsx! {
                Code { src: code!("src/examples/minimal.rs"), theme: snippet_theme() }
            },
            layout: ExampleLayout::Columns,
        }
        ExampleSection {
            title: "One schema, many scalar states",
            intro: rsx! {
                "Try a two-character name or an age below 18, then submit. The same form also demonstrates a boolean, finite choice, nullable value, constant, read-only value, and a write-only replacement. "
                InlineCode { "SubmissionSnapshot" }
                " is emitted only after every parse and validation blocker is clear."
            },
            demo: rsx! { GeneratedControlsExample {} },
            code: rsx! {
                Code { src: code!("src/examples/generated.rs"), theme: snippet_theme() }
            },
        }
    }
}
