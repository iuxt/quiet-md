use std::borrow::Cow;

use gpui::*;

include!(concat!(env!("OUT_DIR"), "/icons_generated.rs"));

const PREFIX: &str = "excalidraw-icons/";

#[derive(Clone, Copy, Debug, Default)]
pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(path.strip_prefix(PREFIX).and_then(|name| ICON_FILES.iter()
            .find(|(file, _)| *file == name).map(|(_, svg)| Cow::Borrowed(svg.as_bytes()))))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICON_FILES.iter().filter_map(|(file, _)| {
            let full = format!("{PREFIX}{file}");
            full.starts_with(path).then(|| full.into())
        }).collect())
    }
}

pub fn icon_path(name: &str) -> SharedString {
    format!("{PREFIX}{name}.svg").into()
}

pub fn has_icon(name: &str) -> bool {
    ICON_FILES
        .iter()
        .any(|(file, _)| *file == format!("{name}.svg"))
}

pub fn icon_names() -> impl Iterator<Item = &'static str> {
    ICON_FILES
        .iter()
        .map(|(file, _)| file.strip_suffix(".svg").unwrap_or(file))
}

pub fn icon_source(name: &str) -> Option<&'static str> {
    ICON_FILES
        .iter()
        .find(|(file, _)| *file == format!("{name}.svg"))
        .map(|(_, svg)| *svg)
}

pub fn icon(name: &str, size: f32, color: Rgba) -> impl IntoElement + use<> {
    debug_assert!(
        has_icon(name),
        "{name} is not in the bundled icon set — check the spelling against assets/icons/"
    );
    svg()
        .path(icon_path(name))
        .flex_shrink_0()
        .size(px(size))
        .text_color(color)
}
