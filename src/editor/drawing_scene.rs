//! Excalidraw files use local points. The vendored geometry uses world points.
use anyhow::{Result, anyhow, ensure};
use base64::Engine as _;
use quiet_excalidraw::core::{bounds::{Bounds, element_bounds}, element::Element, geometry::Point, scene::{BinaryFileData, Scene}};
use quiet_excalidraw::render::export;
use serde_json::{Map, Value, json};
use std::{collections::{HashMap, HashSet}, io::Cursor};

pub(super) const MAX_SOURCE: u64 = 8 * 1024 * 1024;
pub(super) const MAX_ELEMENTS: usize = 600;
pub(super) const MAX_POINTS: usize = 3000;
pub(super) const MAX_SCENE_POINTS: usize = 30_000;
pub(super) const MAX_COORD: f64 = 100_000.;
pub(super) const MAX_IMAGE_PIXELS: u64 = 16_000_000;
pub(super) const MAX_IMAGES: usize = 32;

#[derive(Clone)]
pub(super) struct DrawingScene {
    pub scene: Scene,
    pub background: String,
    pub files: HashMap<String, BinaryFileData>,
    template: Map<String, Value>,
    original: HashMap<String, (Map<String, Value>, Map<String, Value>)>,
}

fn element_points(element: &mut Element) -> Option<&mut Vec<Point>> {
    match element {
        Element::Line(l) => Some(&mut l.linear.points),
        Element::Arrow(a) => Some(&mut a.linear.points),
        Element::Freedraw(f) => Some(&mut f.points),
        _ => None,
    }
}

fn official_element(element: &Element) -> Result<Map<String, Value>> {
    let mut element = element.clone();
    if let Some(points) = element_points(&mut element) && let Some(origin) = points.first().copied() {
        for p in points { *p -= origin; }
        if let Element::Arrow(a) = &mut element && let Some(segments) = &mut a.fixed_segments {
            for s in segments { s.start -= origin; s.end -= origin; }
        }
        let b = element.base_mut();
        b.x = origin.x;
        b.y = origin.y;
    }
    let Value::Object(fields) = serde_json::to_value(element)? else { unreachable!() };
    Ok(fields)
}

fn load_element(mut fields: Map<String, Value>) -> Result<Element> {
    // Older Excalidraw scenes used a focus/gap binding. Fill the fields
    // understood by the Rust model while retaining the original on export.
    for key in ["startBinding", "endBinding"] {
        if let Some(Value::Object(b)) = fields.get_mut(key) {
            if b.get("fixedPoint").is_none_or(Value::is_null) { b.insert("fixedPoint".into(), json!([0.5, 0.5])); }
            b.entry("mode").or_insert_with(|| json!("inside"));
        }
    }
    let mut element: Element = serde_json::from_value(Value::Object(fields))?;
    let b = element.base();
    let origin = Point::new(b.x, b.y);
    ensure!(b.id.len() <= 128 && b.seed.unsigned_abs() <= u32::MAX as u64 && b.version >= 0, "invalid element metadata");
    ensure!(coord(b.x) && coord(b.y) && b.width >= 0. && b.height >= 0. && b.width <= MAX_COORD * 2. && b.height <= MAX_COORD * 2., "element outside canvas limits");
    ensure!(b.angle.abs() <= std::f64::consts::TAU * 2. && (0.1..=64.).contains(&b.stroke_width) && (0. ..=10.).contains(&b.roughness), "invalid drawing style");
    if let Element::Text(t) = &element {
        ensure!(t.text.len() <= 8192 && (1. ..=512.).contains(&t.font_size) && (0.5..=4.).contains(&t.line_height), "text exceeds drawing limits");
    }
    if let Some(points) = element_points(&mut element) {
        ensure!(!points.is_empty() && points.len() <= MAX_POINTS, "too many drawing points");
        for p in points.iter_mut() { *p += origin; ensure!(coord(p.x) && coord(p.y), "point outside canvas limits"); }
        let bounds = Bounds::from_points(points);
        let base = element.base_mut();
        (base.x, base.y, base.width, base.height) = (bounds.min_x, bounds.min_y, bounds.width(), bounds.height());
    }
    if let Element::Arrow(a) = &mut element && let Some(segments) = &mut a.fixed_segments {
        for s in segments { s.start += origin; s.end += origin; }
    }
    Ok(element)
}

fn coord(v: f64) -> bool { v.abs() <= MAX_COORD }

impl DrawingScene {
    pub(super) fn new() -> Self {
        Self { scene: Scene::new(), background: "#ffffff".into(), files: HashMap::new(), template: Map::new(), original: HashMap::new() }
    }

    pub(super) fn load(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() as u64 <= MAX_SOURCE, "drawing file is too large");
        let mut template: Map<String, Value> = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))?;
        ensure!(template.get("type").and_then(Value::as_str) == Some("excalidraw"), "expected an Excalidraw scene");
        let Some(Value::Array(elements)) = template.remove("elements") else { return Err(anyhow!("missing drawing elements")) };
        ensure!(elements.len() <= MAX_ELEMENTS, "too many drawing elements");
        let mut result = Self::new();
        if let Some(bg) = template.get("appState").and_then(|s| s.get("viewBackgroundColor")).and_then(Value::as_str)
            && (bg == "transparent" || bg.strip_prefix('#').is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))) {
            result.background = bg.into();
        }
        if let Some(files) = template.get("files").filter(|f| !f.is_null()) {
            let files: HashMap<String, BinaryFileData> = serde_json::from_value(files.clone())?;
            ensure!(files.len() <= MAX_IMAGES, "too many drawing images");
            let mut pixels = 0;
            for file in files.values() {
                let (bytes, format) = image_data(file)?;
                let (w, h) = image::ImageReader::with_format(Cursor::new(bytes), format).into_dimensions()?;
                pixels += u64::from(w) * u64::from(h);
                ensure!(w <= 4096 && h <= 4096 && pixels <= MAX_IMAGE_PIXELS, "drawing images exceed the pixel limit");
            }
            result.files = files;
        }
        let mut seen = HashSet::new();
        let mut points = 0;
        for value in elements {
            let Value::Object(fields) = value else { return Err(anyhow!("invalid drawing element")) };
            let mut element = load_element(fields.clone())?;
            points += element_points(&mut element).map_or(0, |p| p.len());
            ensure!(points <= MAX_SCENE_POINTS && seen.insert(element.id().to_string()), "too many points or duplicate element id");
            result.original.insert(element.id().into(), (fields, official_element(&element)?));
            result.scene.elements.push(element);
        }
        result.template = template;
        Ok(result)
    }

    pub(super) fn remaining_points(&self) -> usize {
        let count = self.scene.elements.iter().map(|e| match e {
            Element::Freedraw(f) => f.points.len(), Element::Line(l) => l.linear.points.len(), Element::Arrow(a) => a.linear.points.len(), _ => 0,
        }).sum::<usize>();
        MAX_SCENE_POINTS.saturating_sub(count)
    }

    pub(super) fn json(&self) -> Result<Vec<u8>> {
        let mut root = self.template.clone();
        root.insert("type".into(), json!("excalidraw"));
        root.entry("version").or_insert_with(|| json!(2));
        root.entry("source").or_insert_with(|| json!("https://quiet.md"));
        let state = root.entry("appState").or_insert_with(|| json!({}));
        if !state.is_object() { *state = json!({}); }
        state["viewBackgroundColor"] = json!(self.background);
        let files = root.entry("files").or_insert_with(|| json!({}));
        if !files.is_object() { *files = json!({}); }
        for element in self.scene.non_deleted() {
            if let Element::Image(image) = element && let Some(id) = &image.file_id
                && files.get(id).is_none() && let Some(file) = self.files.get(id) {
                files[id] = serde_json::to_value(file)?;
            }
        }
        let elements = self.scene.elements.iter().map(|element| {
            let mut now = official_element(element)?;
            if let Some((raw, original)) = self.original.get(element.id()) {
                let mut fields = raw.clone();
                for (key, value) in now {
                    if original.get(&key) != Some(&value) { fields.insert(key, value); }
                }
                now = fields;
            }
            Ok(Value::Object(now))
        }).collect::<Result<Vec<_>>>()?;
        root.insert("elements".into(), Value::Array(elements));
        let bytes = serde_json::to_vec(&root)?;
        ensure!(bytes.len() as u64 <= MAX_SOURCE, "drawing file is too large");
        Ok(bytes)
    }

    pub(super) fn preview(&self) -> Result<(Vec<u8>, u32, u32)> {
        let png = export::export_png_with_files(&self.scene, &self.background, 2., &self.files).map_err(|s| anyhow!(s))?;
        let (w, h) = image::ImageReader::with_format(Cursor::new(&png), image::ImageFormat::Png).into_dimensions()?;
        Ok((png, w, h))
    }
}

pub(super) fn image_data(file: &BinaryFileData) -> Result<(Vec<u8>, image::ImageFormat)> {
    use image::ImageFormat::*;
    let (mime, data) = file.data_url.split_once(";base64,").ok_or_else(|| anyhow!("expected an embedded drawing image"))?;
    let format = match mime {
        "data:image/png" => Png, "data:image/jpeg" => Jpeg, "data:image/gif" => Gif, "data:image/webp" => WebP,
        _ => return Err(anyhow!("unsupported embedded image format")),
    };
    Ok((base64::engine::general_purpose::STANDARD.decode(data)?, format))
}

pub(super) fn bounds(scene: &Scene) -> Option<Bounds> {
    scene.non_deleted().map(element_bounds).reduce(|a, b| a.union(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quiet_excalidraw::core::{arrowhead::Arrowhead, factory::{ElementOptions, new_arrow}, transform::translate_element};

    #[test]
    fn excalidraw_local_points_and_unknown_fields_survive_an_edit() {
        let mut scene = DrawingScene::new();
        scene.scene.elements.push(new_arrow(vec![Point::new(190., 85.), Point::new(25., 95.)], None, Some(Arrowhead::Arrow), &ElementOptions::default()));
        let mut raw: Value = serde_json::from_slice(&scene.json().unwrap()).unwrap();
        assert_eq!(raw["elements"][0]["x"], 190.);
        assert_eq!(raw["elements"][0]["points"], json!([[0., 0.], [-165., 10.]]));
        raw["elements"][0]["futureField"] = json!({"kept": true});
        raw["appState"]["gridSize"] = json!(20);
        let mut reopened = DrawingScene::load(raw.to_string().as_bytes()).unwrap();
        let Element::Arrow(arrow) = &reopened.scene.elements[0] else { panic!("expected arrow") };
        assert_eq!(arrow.linear.points[0], Point::new(190., 85.));
        translate_element(&mut reopened.scene.elements[0], 13., -5.);
        let saved: Value = serde_json::from_slice(&reopened.json().unwrap()).unwrap();
        assert_eq!(saved["elements"][0]["x"], 203.);
        assert_eq!(saved["elements"][0]["y"], 80.);
        assert_eq!(saved["elements"][0]["points"], raw["elements"][0]["points"]);
        assert_eq!(saved["elements"][0]["futureField"], raw["elements"][0]["futureField"]);
        assert_eq!(saved["appState"]["gridSize"], 20);
        let (png, w, h) = reopened.preview().unwrap();
        assert!(png.starts_with(b"\x89PNG") && w > h && h > 0);

        reopened.background = "#fffce8".into();
        let file = BinaryFileData::new("image-1", "image/png", format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)));
        reopened.files.insert(file.id.clone(), file.clone());
        reopened.scene.elements.push(quiet_excalidraw::factory::new_image(0., 0., f64::from(w), f64::from(h), Some(file.id.clone()), &ElementOptions::default()));
        let stored = DrawingScene::load(&reopened.json().unwrap()).unwrap();
        assert_eq!(stored.background, reopened.background);
        assert_eq!(stored.files.get(&file.id), Some(&file));
        assert_eq!(stored.template["appState"]["gridSize"], 20);
    }
}
