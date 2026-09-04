use dioxus::prelude::*;
use dioxus_code::{Code, code};

use crate::components::{
    DocsCallout, ExampleSection, ExternalAction, InlineCode, PageHeader, snippet_theme,
};
use crate::examples::daisyui::DaisyuiControlsExample;

#[component]
pub fn Daisyui() -> Element {
    rsx! {
        PageHeader {
            eyebrow: "Renderers",
            title: "daisyUI controls",
            intro: "A custom control renderer owns its whole control region. Here every control kind is rendered by the daisyUI registry's Field parts and widgets, so only the layout and the submit button still come from the built-in renderer.",
        }
        ExampleSection {
            title: "Registry widgets on the headless edit hooks",
            intro: rsx! {
                "The "
                InlineCode { "schemaform_daisyui" }
                " component registers one "
                InlineCode { "ControlRenderer" }
                " above the built-in priority for every control kind, plus one per widget symbol. It maps "
                InlineCode { "use_text_edit" }
                ", "
                InlineCode { "use_boolean_edit" }
                ", and "
                InlineCode { "use_choice_edit" }
                " onto "
                InlineCode { "dioxus-field" }
                " bindings and the node presentation onto field metadata. Strings, numbers, and integers are an "
                InlineCode { "Input" }
                "; a non-nullable boolean is a native checkbox and a nullable one the registry "
                InlineCode { "Checkbox" }
                " showing null as indeterminate; a write-only boolean or choice is a replacement select that never echoes its value; choices are a "
                InlineCode { "NativeSelect" }
                " unless the UI schema names "
                InlineCode { "daisyui:radio" }
                " or "
                InlineCode { "daisyui:select" }
                "; constants are read-only output. Try a two-character name, set the newsletter to null and back, pick a billing cycle with the arrow keys, then submit."
            },
            demo: rsx! { DaisyuiControlsExample {} },
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
