use dioxus::prelude::*;

use crate::app::Route;
use crate::components::PageHeader;

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = segments.join("/");
    rsx! {
        PageHeader {
            eyebrow: "404",
            title: "Example not found",
            intro: format!("No demo page exists at /{path}."),
        }
        Link { to: Route::Home {}, class: "btn btn-primary mt-8", "Back to overview" }
    }
}
