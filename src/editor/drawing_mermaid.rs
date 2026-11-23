//! Editable flowcharts and image fallback for the other Mermaid diagram types.
use super::{DrawingScene, Element, ElementOptions, MAX_SOURCE, World, drawing_scene, factory, scene};
use anyhow::{Result, ensure};
use base64::Engine as _;

pub(super) fn mermaid_scene(input: &str) -> Result<DrawingScene> {
    ensure!(input.len() as u64 <= MAX_SOURCE, "Mermaid source exceeds the drawing size limit");
    let mut doc = DrawingScene::new();
    if let Ok(elements) = scene::mermaid::mermaid_elements(input) {
        doc.scene.elements = elements;
        return Ok(doc);
    }

    let svg = mermaid_svg::render(input)?;
    let (width, height) = quiet_excalidraw::render::export::intrinsic_svg_size(&svg).map_err(anyhow::Error::msg)?;
    ensure!(width <= 4096 && height <= 4096 && u64::from(width) * u64::from(height) <= drawing_scene::MAX_IMAGE_PIXELS,
        "Rendered Mermaid diagram exceeds drawing image limits");
    let id = factory::new_id();
    let data_url = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg));
    doc.files.insert(id.clone(), scene::scene::BinaryFileData::new(id.clone(), "image/svg+xml", data_url));
    let size = World::new(f64::from(width), f64::from(height));
    let mut element = factory::new_image(0., 0., size.x, size.y, Some(id), &ElementOptions::default());
    if let Element::Image(image) = &mut element { image.status = scene::element::ImageStatus::Saved; }
    doc.scene.elements.push(element);
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_flowchart_is_editable_and_sequence_is_an_embedded_image() {
        let flowchart = mermaid_scene("flowchart LR\n A[Start] --> B[End]").unwrap();
        assert!(flowchart.scene.elements.iter().any(|el| matches!(el, Element::Arrow(_))));
        assert!(flowchart.files.is_empty());

        let sequence = mermaid_scene("sequenceDiagram\n Alice->>Bob: Hello").unwrap();
        assert!(matches!(sequence.scene.elements.as_slice(), [Element::Image(_)]));
        let png = sequence.export_png(drawing_scene::ExportOptions { embed: true, ..Default::default() }).unwrap();
        let reopened = DrawingScene::load_exported(&png).unwrap();
        assert_eq!(reopened.files, sequence.files);
        assert_eq!(reopened.scene.elements, sequence.scene.elements);
        assert!(mermaid_scene("not a diagram").is_err());
    }
}
