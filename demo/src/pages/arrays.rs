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
            intro: "Array rows carry opaque item identity, so editing state, findings, focus, and DOM identity follow the logical row when indices move. The chrome around the rows is a renderer's; here it is the demo's daisyUI collection.",
        }
        ExampleSection {
            title: "Scalar and fixed-object items",
            intro: rsx! {
                "The first collection uses string items; the second uses fixed-object items with text and choice controls. Add operations use schema defaults, while "
                InlineCode { "minItems" }
                " and "
                InlineCode { "maxItems" }
                " gate only changes that cross or worsen their bounds: the tags may be emptied, the team keeps at least one member. The cards, the joined insert, move, and remove buttons, the add button, and the empty state come from the "
                InlineCode { "schemaform_daisyui" }
                " component's "
                InlineCode { "CollectionRenderer" }
                "; the adapter keeps item identity and keying, moves focus after each mutation, and announces it through the live region the renderer places. Reorder an item and its input keeps its value and focus; remove one and focus lands on its neighbour; remove every tag and the empty state takes their place."
            },
            demo: rsx! { ArraysExample {} },
            code: rsx! {
                Code { src: code!("src/examples/arrays.rs"), theme: snippet_theme() }
            },
        }
    }
}
