//! Route components for the docs-by-example gallery.

mod arrays;
mod daisyui;
mod daisyui_builtin;
mod generated;
mod home;
mod not_found;
mod playground;
mod presentation;

pub use arrays::Arrays;
pub use daisyui::{Daisyui, DaisyuiRtl};
pub use daisyui_builtin::DaisyuiBuiltin;
pub use generated::Generated;
pub use home::Home;
pub use not_found::NotFound;
pub use playground::Playground;
pub use presentation::Presentation;
