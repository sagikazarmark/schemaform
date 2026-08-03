//! Schemaform docs-by-example application.

mod app;
mod components;
mod examples;
mod pages;

fn main() {
    dioxus::launch(app::App);
}
