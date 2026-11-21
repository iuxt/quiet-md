#[path = "bitmap.rs"]
pub mod bitmap;
#[path = "canvas.rs"]
pub mod canvas;
#[path = "chrome.rs"]
pub mod chrome;
#[path = "design.rs"]
pub mod design;
#[path = "editor.rs"]
pub mod editor;
#[path = "export.rs"]
pub mod export;
#[path = "i18n.rs"]
pub mod i18n;
#[path = "icons.rs"]
pub mod icons;
#[path = "layers.rs"]
pub mod layers;
#[path = "library.rs"]
pub mod library;
#[path = "properties.rs"]
pub mod properties;
#[path = "state.rs"]
pub mod state;
#[path = "theme.rs"]
pub mod theme;

pub use canvas::*;
pub use chrome::*;
pub use design::*;
pub use editor::*;
pub use export::*;
pub use i18n::*;
pub use icons::*;
pub use layers::*;
pub use library::*;
pub use properties::*;
pub use state::*;
pub use theme::*;

fn with_alpha(color: gpui::Rgba, a: f32) -> gpui::Rgba {
    gpui::Rgba { a, ..color }
}
