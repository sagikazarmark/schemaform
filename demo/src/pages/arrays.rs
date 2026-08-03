use dioxus::prelude::*;
use dioxus_code::{Code, code};

use crate::components::{ExampleSection, InlineCode, PageHeader, snippet_theme};
use crate::examples::arrays::ArraysExample;

#[component]
pub fn Arrays() -> Element {
    rsx! {
        PageHeader {
            eyebrow: "Structure",
            title: "Stable homogeneous arrays",
            intro: "Array rows carry opaque item identity, so editing state, findings, focus, and DOM identity follow the logical row when indices move.",
        }
        ExampleSection {
            title: "Scalar and fixed-object items",
            intro: rsx! {
                "The first collection uses string items; the second uses fixed-object items with text and choice controls. Add operations use schema defaults, while "
                InlineCode { "minItems" }
                " and "
                InlineCode { "maxItems" }
                " gate only changes that cross or worsen their bounds."
            },
            demo: rsx! { ArraysExample {} },
            code: rsx! {
                Code { src: code!("src/examples/arrays.rs"), theme: snippet_theme() }
            },
        }
    }
}
