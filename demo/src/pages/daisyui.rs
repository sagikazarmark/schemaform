use dioxus::prelude::*;
use dioxus_code::{Code, code};

use crate::components::{
    DocsCallout, ExampleSection, ExternalAction, InlineCode, PageHeader, snippet_theme,
};
use crate::examples::daisyui::DaisyuiTextControlsExample;

#[component]
pub fn Daisyui() -> Element {
    rsx! {
        PageHeader {
            eyebrow: "Renderers",
            title: "daisyUI text controls",
            intro: "A custom control renderer owns its whole control region. Here every string, number, and integer control is rendered by the daisyUI registry's Field and Input parts, while every other node kind still comes from the built-in renderer.",
        }
        ExampleSection {
            title: "Registry widgets on the headless text edit hook",
            intro: rsx! {
                "The "
                InlineCode { "schemaform_daisyui" }
                " component registers one "
                InlineCode { "ControlRenderer" }
                " above the built-in priority for string, number, and integer controls. It maps "
                InlineCode { "use_text_edit" }
                " onto a "
                InlineCode { "dioxus-field" }
                " binding and the node presentation onto field metadata, so the registry's "
                InlineCode { "Input" }
                " shows the label, help, errors, and invalid state the core computes. Try a two-character name, a non-numeric age, or the nickname's presence affordances, then submit."
            },
            demo: rsx! { DaisyuiTextControlsExample {} },
            code: rsx! {
                Code { src: code!("src/examples/daisyui.rs"), theme: snippet_theme() }
            },
        }
        DocsCallout {
            title: "The mapping lives in the demo",
            action: Some(ExternalAction::new(
                "Read the component's README",
                "https://github.com/sagikazarmark/schemaform/blob/main/demo/src/components/schemaform_daisyui/README.md",
            )),
            "The published crates do not depend on dioxus-field or the registry. The component is laid out as a dx components member under src/components so it can move to a registry later, and it is browser-CSR only."
        }
    }
}
