use dioxus::prelude::*;

use crate::components::{DocsCallout, ExternalAction, PageHeader};
use crate::examples::editor::SchemaEditorExample;

#[component]
pub fn Playground() -> Element {
    rsx! {
        PageHeader {
            eyebrow: "Explore",
            title: "Edit both schemas, render one form",
            intro: "Switch between the JSON Schema and UI schema tabs on the left, apply a valid pair, and interact with the newly compiled form on the right. On narrow screens the panes stack without losing either editor.",
        }
        section { class: "mt-10 rounded-[2rem] border border-base-300 bg-base-100 p-4 shadow-sm sm:p-6",
            SchemaEditorExample {}
        }
        DocsCallout {
            title: "Stable UI schema v1",
            action: Some(ExternalAction::new(
                "Open the v1 meta-schema",
                "https://github.com/sagikazarmark/schemaform/blob/main/ui-schema-v1.schema.json",
            )),
            "The playground uses stable UI schema v1. Its JSON vocabulary and headless meaning are frozen independently of DOM structure and styling."
        }
    }
}
