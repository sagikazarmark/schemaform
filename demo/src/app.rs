//! Router and shared application shell.

use dioxus::prelude::*;

use crate::components::{DemoFooter, DemoHeader, Sidebar, SidebarNavLink, SidebarNavSection};
use crate::pages::*;

/// The site stylesheet: Tailwind, daisyUI, and the daisyUI theme for the built-in renderer's
/// class hooks (`src/forms.css`), compiled together by `npm run build`.
const STYLE: Asset = asset!("assets/style.css");

#[derive(Routable, Clone, PartialEq, Debug)]
pub enum Route {
    #[layout(DemoLayout)]
    #[route("/")]
    Home {},
    #[route("/generated")]
    Generated {},
    #[route("/arrays")]
    Arrays {},
    #[route("/presentation")]
    Presentation {},
    #[route("/daisyui")]
    Daisyui {},
    #[route("/daisyui/rtl")]
    DaisyuiRtl {},
    #[route("/daisyui/builtin")]
    DaisyuiBuiltin {},
    #[route("/playground")]
    Playground {},
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

#[component]
pub fn App() -> Element {
    rsx! {
        document::Stylesheet { href: STYLE }
        Router::<Route> {}
    }
}

#[component]
fn DemoLayout() -> Element {
    let mut hydrated = use_signal(|| false);
    use_effect(move || hydrated.set(true));

    rsx! {
        div {
            class: "min-h-screen bg-base-100 text-base-content",
            "data-demo-hydrated": if hydrated() { "true" } else { "false" },
            DemoHeader {
                home: Route::Home {},
                mark: "df",
                name: "schemaform",
                github_url: "https://github.com/sagikazarmark/schemaform",
            }
            div { class: "mx-auto w-full max-w-7xl lg:flex lg:gap-8 lg:px-6",
                Sidebar {
                    SidebarNavSection { label: "Start",
                        SidebarNavLink { route: Route::Home {}, label: "Overview" }
                        SidebarNavLink { route: Route::Generated {}, label: "Generated controls" }
                    }
                    SidebarNavSection { label: "Structure",
                        SidebarNavLink { route: Route::Arrays {}, label: "Arrays" }
                        SidebarNavLink { route: Route::Presentation {}, label: "Authored UI schema" }
                    }
                    SidebarNavSection { label: "Renderers",
                        SidebarNavLink { route: Route::Daisyui {}, label: "daisyUI form" }
                        SidebarNavLink { route: Route::DaisyuiRtl {}, label: "daisyUI form, RTL" }
                        SidebarNavLink { route: Route::DaisyuiBuiltin {}, label: "Unstyled built-in" }
                    }
                    SidebarNavSection { label: "Explore",
                        SidebarNavLink { route: Route::Playground {}, label: "Schema playground" }
                    }
                }
                main { id: "main-content", class: "min-w-0 flex-1 px-4 py-8 sm:px-6 lg:px-0 lg:py-12",
                    Outlet::<Route> {}
                }
            }
            DemoFooter {
                description: "A docs-by-example gallery for Schemaform.",
                links: rsx! {
                    a {
                        class: "hover:text-base-content",
                        href: "https://github.com/sagikazarmark/schemaform",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "Repository"
                    }
                },
            }
        }
    }
}
