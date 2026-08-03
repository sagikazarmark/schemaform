use dioxus::prelude::*;
use dioxus_code::{Code, code};

use crate::components::{ExampleSection, InlineCode, PageHeader, snippet_theme};
use crate::examples::ui_schema::AuthoredUiSchemaExample;

#[component]
pub fn Presentation() -> Element {
    rsx! {
        PageHeader {
            eyebrow: "Presentation",
            title: "An independent authored UI schema",
            intro: "The optional UI schema describes presentation without becoming a second data model. It binds controls by RFC 6901 JSON Pointer and can add layout and plain text around them.",
        }
        ExampleSection {
            title: "Stack, text, responsive grid, group, and tabs",
            intro: rsx! {
                "The authored document reorders the data schema's properties, places names in a 12-column "
                InlineCode { "grid" }
                ", groups contact fields, and moves plan and newsletter controls into "
                InlineCode { "tabs" }
                ". Resize the page to see compact and wide grid spans."
            },
            demo: rsx! { AuthoredUiSchemaExample {} },
            code: rsx! {
                Code { src: code!("src/examples/ui_schema.rs"), theme: snippet_theme() }
            },
        }
    }
}
