//! Excalidraw files use local points. The vendored geometry uses world points.
use anyhow::{Result, anyhow, ensure};
use base64::Engine as _;
use crate::{model::Drawing, store};
use quiet_excalidraw::core::{bounds::Bounds, element::Element, geometry::Point, scene::{BinaryFileData, Scene}};
use quiet_excalidraw::render::export;
use serde_json::{Map, Value, json};
use std::{collections::{HashMap, HashSet}, io::Cursor, path::Path};

pub(super) const MAX_SOURCE: u64 = 8 * 1024 * 1024;
pub(super) const MAX_ELEMENTS: usize = 16_384;
pub(super) const MAX_POINTS: usize = 32_768;
pub(super) const MAX_SCENE_POINTS: usize = 524_288;
pub(super) const MAX_COORD: f64 = 100_000.;
pub(super) const MAX_IMAGE_PIXELS: u64 = 16_000_000;
pub(super) const MAX_IMAGES: usize = 32;
const PREVIEW_SCALE: f32 = 2.;
const MAX_PREVIEW_EDGE: u32 = 2048;
const MAX_PNG: u64 = 20 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct ExportOptions { pub background: bool, pub dark: bool, pub embed: bool, pub scale: f32, pub show_frames: bool }

impl Default for ExportOptions {
    fn default() -> Self { Self { background: true, dark: false, embed: false, scale: 2., show_frames: true } }
}

#[derive(Clone)]
pub(super) struct DrawingScene {
    pub scene: Scene,
    pub background: String,
    pub grid_enabled: bool,
    pub export_region: Option<Bounds>,
    pub grid_size: f64,
    pub grid_step: usize,
    pub files: HashMap<String, BinaryFileData>,
    template: Map<String, Value>,
    #[cfg(test)]
    pub(super) library_item: Map<String, Value>,
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

fn migrate_element(fields: &mut Map<String, Value>) {
    for (key, default) in [
        ("version", json!(1)), ("versionNonce", json!(0)), ("isDeleted", json!(false)), ("groupIds", json!([])),
        ("locked", json!(false)), ("updated", json!(0)), ("seed", json!(1)), ("angle", json!(0)),
        ("strokeStyle", json!("solid")), ("strokeWidth", json!(2)), ("roughness", json!(1)), ("opacity", json!(100)),
        ("fillStyle", json!("hachure")), ("strokeColor", json!("#1e1e1e")), ("backgroundColor", json!("transparent")),
        ("x", json!(0)), ("y", json!(0)), ("width", json!(0)), ("height", json!(0)),
    ] { fields.entry(key).or_insert(default); }
    if let Some(Value::Array(ids)) = fields.remove("boundElementIds") {
        fields.entry("boundElements").or_insert_with(|| Value::Array(ids.iter().map(|id| json!({"id":id, "type":"arrow"})).collect()));
    }
    if fields.get("type").and_then(Value::as_str) == Some("draw") { fields.insert("type".into(), json!("line")); }
    if matches!(fields.get("type").and_then(Value::as_str), Some("line" | "arrow"))
        && fields.get("points").and_then(Value::as_array).is_none_or(|points| points.len() < 2) {
        fields.insert("points".into(), json!([[0,0],[fields["width"],fields["height"]]]));
    }
    if fields.get("roundness").is_none_or(Value::is_null) && fields.get("strokeSharpness") == Some(&json!("round")) {
        let kind = if matches!(fields.get("type").and_then(Value::as_str), Some("rectangle" | "embeddable" | "iframe")) { 1 } else { 2 };
        fields.insert("roundness".into(), json!({"type":kind}));
    }
    if fields.get("type") == Some(&json!("text")) {
        let text = fields.get("text").cloned().unwrap_or_else(|| json!(""));
        fields.entry("originalText").or_insert(text);
        let font = fields.get("font").and_then(Value::as_str).and_then(|s| s.split_once(' ')).map(|(px, name)| (px.trim_end_matches("px").parse::<f64>().unwrap_or(20.), name.to_owned()));
        let (size, family) = font.unwrap_or((20., "Excalifont".into()));
        fields.entry("fontSize").or_insert(json!(size));
        let family = serde_json::from_value::<quiet_excalidraw::types::FontFamily>(json!(family)).unwrap_or(quiet_excalidraw::types::FontFamily::Excalifont);
        fields.entry("fontFamily").or_insert(json!(family.code()));
        fields.entry("textAlign").or_insert(json!("left"));
        fields.entry("verticalAlign").or_insert(json!("top"));
        fields.entry("autoResize").or_insert(json!(true));
    }
}

fn load_element(mut fields: Map<String, Value>) -> Result<Element> {
    // Older Excalidraw scenes used a focus/gap binding. Fill the fields
    // understood by the Rust model while retaining the original on export.
    for key in ["startBinding", "endBinding"] {
        if let Some(Value::Object(b)) = fields.get_mut(key) {
            if b.get("fixedPoint").is_none_or(Value::is_null) {
                b.insert("fixedPoint".into(), json!([0.5, 0.5])); b.insert("mode".into(), json!("orbit"));
            }
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
        ensure!((1. ..=512.).contains(&t.font_size) && (0.5..=4.).contains(&t.line_height), "text exceeds drawing limits");
    }
    if let Element::Image(image) = &element {
        ensure!(image.scale.iter().all(|v| *v == 1. || *v == -1.), "invalid image scale");
        if let Some(crop) = &image.crop {
            ensure!(crop.x >= 0. && crop.y >= 0. && crop.width > 0. && crop.height > 0. && crop.natural_width > 0. && crop.natural_height > 0.
                && crop.natural_width <= 4096. && crop.natural_height <= 4096. && crop.x + crop.width <= crop.natural_width + 0.001
                && crop.y + crop.height <= crop.natural_height + 0.001, "invalid image crop");
        }
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

fn decode_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let value: Value = serde_json::from_slice(bytes).or_else(|_| serde_json::from_str(&bytes.iter().map(|b| char::from(*b)).collect::<String>()))?;
    let Some(encoded) = value.get("encoded").and_then(Value::as_str) else { return Ok(serde_json::to_vec(&value)?) };
    ensure!(value["encoding"] == "bstring", "unknown drawing metadata encoding");
    let bytes = encoded.chars().map(|c| u8::try_from(c as u32)).collect::<std::result::Result<Vec<_>, _>>()?;
    let bytes = if value["compressed"] == true {
        let mut out = Vec::new(); flate2::read::ZlibDecoder::new(bytes.as_slice()).take(MAX_SOURCE + 1).read_to_end(&mut out)?; out
    } else { bytes };
    ensure!(bytes.len() as u64 <= MAX_SOURCE, "drawing source is too large"); Ok(bytes)
}

fn embedded_png_source(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut at = 8;
    while at + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into()?) as usize;
        let end = at.checked_add(len).and_then(|end| end.checked_add(12)).filter(|end| *end <= bytes.len()).ok_or_else(|| anyhow!("invalid PNG chunk"))?;
        if &bytes[at + 4..at + 8] == b"tEXt" {
            let data = &bytes[at + 8..end - 4];
            if let Some(n) = data.iter().position(|b| *b == 0) && &data[..n] == b"application/vnd.excalidraw+json" {
                return decode_metadata(&data[n + 1..]);
            }
        }
        at = end;
    }
    Err(anyhow!("PNG has no drawing source"))
}

fn encode_metadata(bytes: &[u8]) -> Result<String> {
    use std::io::Write;
    let mut zip = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default()); zip.write_all(bytes)?;
    let encoded: String = zip.finish()?.into_iter().map(char::from).collect();
    let json = json!({"version":"1", "encoding":"bstring", "compressed":true, "encoded":encoded}).to_string();
    Ok(json.chars().map(|c| if c.is_ascii() { c.to_string() } else { format!("\\u{:04x}", c as u32) }).collect())
}

impl DrawingScene {
    #[cfg(test)]
    pub(super) fn load_library(bytes: &[u8]) -> Result<Vec<Self>> {
        ensure!(bytes.len() as u64 <= MAX_SOURCE, "drawing library is too large");
        let root: Value = serde_json::from_slice(bytes)?;
        ensure!(root["type"] == "excalidrawlib", "expected an Excalidraw library");
        let items = root.get("libraryItems").or_else(|| root.get("library")).and_then(Value::as_array).ok_or_else(|| anyhow!("missing library items"))?;
        ensure!(items.len() <= 64, "too many library items");
        items.iter().map(|item| {
            let elements = if item.is_array() { item } else { &item["elements"] };
            let mut doc = Self::load(&serde_json::to_vec(&json!({"type":"excalidraw", "elements":elements, "files":root.get("files").cloned().unwrap_or_else(|| json!({}))}))?)?;
            doc.library_item = item.as_object().cloned().unwrap_or_default();
            doc.library_item.remove("elements");
            doc.init_library_item();
            Ok(doc)
        }).collect()
    }

    #[cfg(test)]
    pub(super) fn init_library_item(&mut self) {
        self.library_item.entry("id").or_insert_with(|| json!(uuid::Uuid::new_v4().to_string()));
        self.library_item.entry("status").or_insert(json!("unpublished"));
        self.library_item.entry("created").or_insert_with(|| json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64));
    }

    #[cfg(test)]
    pub(super) fn library_json(items: &[Self]) -> Result<Vec<u8>> {
        let mut files = serde_json::Map::new();
        let items = items.iter().map(|source| {
            let mut doc = source.clone();
            let old_files = doc.files.clone();
            for (id, file) in old_files {
                if files.get(&id).is_some_and(|old| *old != serde_json::to_value(&file).unwrap_or_default()) {
                    let new_id = uuid::Uuid::new_v4().to_string(); let mut data = file; data.id = new_id.clone();
                    doc.files.remove(&id); doc.files.insert(new_id.clone(), data);
                    for e in &mut doc.scene.elements { if let Element::Image(i) = e && i.file_id.as_deref() == Some(&id) { i.file_id = Some(new_id.clone()); } }
                }
            }
            doc.retain_used_files();
            let value: Value = serde_json::from_slice(&doc.json()?)?;
            if let Some(data) = value["files"].as_object() { files.extend(data.clone()); }
            doc.init_library_item();
            doc.library_item.insert("elements".into(), value["elements"].clone());
            Ok(Value::Object(doc.library_item))
        }).collect::<Result<Vec<_>>>()?;
        let bytes = serde_json::to_vec(&json!({"type":"excalidrawlib", "version":2, "source":"https://quiet.md", "libraryItems":items, "files":files}))?;
        ensure!(bytes.len() as u64 <= MAX_SOURCE, "drawing library is too large"); Ok(bytes)
    }

    pub(super) fn load_exported(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() as u64 <= MAX_SOURCE * 4, "drawing file is too large");
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Self::load(&embedded_png_source(bytes)?);
        }
        if let Ok(svg) = std::str::from_utf8(bytes) && svg.trim_start().starts_with('<') {
            if let Some((_, payload)) = svg.split_once("<!-- payload-start -->") && let Some((payload, _)) = payload.split_once("<!-- payload-end -->") {
                let encoded = base64::engine::general_purpose::STANDARD.decode(payload.trim())?;
                return Self::load(&decode_metadata(&encoded)?);
            }
            let mut reader = quick_xml::Reader::from_str(svg); let mut metadata = false;
            loop {
                match reader.read_event()? {
                    quick_xml::events::Event::Start(e) if e.name().as_ref() == b"metadata" => metadata = e.attributes().flatten().any(|a| a.key.as_ref() == b"id" && a.value.as_ref() == b"excalidraw-scene"),
                    quick_xml::events::Event::Text(e) if metadata => return Self::load(e.unescape()?.as_bytes()),
                    quick_xml::events::Event::End(e) if e.name().as_ref() == b"metadata" => metadata = false,
                    quick_xml::events::Event::Eof => break,
                    _ => {}
                }
            }
            return Err(anyhow!("SVG has no drawing source"));
        }
        Self::load(bytes)
    }

    #[cfg(test)]
    pub(super) fn editable_png(&self) -> Result<Vec<u8>> { self.export_png(ExportOptions { embed: true, ..ExportOptions::default() }) }

    pub(super) fn export_png(&self, options: ExportOptions) -> Result<Vec<u8>> {
        let background = if options.background { &self.background } else { "transparent" };
        let svg = self.render_export(background, options);
        let png = export::rasterize_pixels(&svg, options.scale.clamp(1., 3.), 4096).map_err(|e| anyhow!(e))?.encode_png().map_err(|e| anyhow!(e))?;
        if options.embed { Ok(export::embed_png_scene_data(&png, &encode_metadata(&self.json()?)?)) } else { Ok(png) }
    }

    #[cfg(test)]
    pub(super) fn editable_svg(&self) -> Result<String> { self.export_svg(ExportOptions { embed: true, ..ExportOptions::default() }) }

    pub(super) fn export_svg(&self, options: ExportOptions) -> Result<String> {
        let mut svg = self.standalone_svg_with(options);
        if options.embed {
            let encoded = base64::engine::general_purpose::STANDARD.encode(encode_metadata(&self.json()?)?.as_bytes());
            let metadata = format!("<metadata><!-- payload-type:application/vnd.excalidraw+json --><!-- payload-version:2 --><!-- payload-start -->{encoded}<!-- payload-end --></metadata>");
            if let Some(at) = svg.rfind("</svg>") { svg.insert_str(at, &metadata); }
        }
        Ok(svg)
    }

    pub(super) fn read(root: &Path, source: &str) -> Result<Self> {
        Self::load(&source_bytes(root, source)?)
    }

    pub(super) fn new() -> Self {
        Self { scene: Scene::new(), background: "#ffffff".into(), grid_enabled:false, export_region: None, grid_size:20., grid_step:5, files: HashMap::new(), template: Map::new(), #[cfg(test)] library_item: Map::new(), original: HashMap::new() }
    }

    #[cfg(test)]
    pub(super) fn retain_used_files(&mut self) {
        let used: HashSet<_> = self.scene.non_deleted().filter_map(|e| if let Element::Image(i) = e { i.file_id.as_deref() } else { None }).collect();
        self.files.retain(|id, _| used.contains(id.as_str()));
        if let Some(Value::Object(files)) = self.template.get_mut("files") { files.retain(|id, _| used.contains(id.as_str())); }
    }

    pub(super) fn load(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() as u64 <= MAX_SOURCE, "drawing file is too large");
        let mut template: Map<String, Value> = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))?;
        ensure!(template.get("type").and_then(Value::as_str) == Some("excalidraw"), "expected an Excalidraw scene");
        let Some(Value::Array(elements)) = template.remove("elements") else { return Err(anyhow!("missing drawing elements")) };

        let mut result = Self::new();
        if let Some(bg) = template.get("appState").and_then(|s| s.get("viewBackgroundColor")).and_then(Value::as_str)
            && (bg == "transparent" || bg.strip_prefix('#').is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))) {
            result.background = bg.into();
        }
        if let Some(state) = template.get("appState") {
            result.grid_enabled = state["gridModeEnabled"].as_bool().unwrap_or(false);
            if let Some(size) = state["gridSize"].as_f64().filter(|size| (1. ..=1000.).contains(size)) { result.grid_size = size; }
            if let Some(step) = state["gridStep"].as_u64().filter(|step| (1..=100).contains(step)) { result.grid_step = step as usize; }
        }
        let mut seen = HashSet::new();
        let mut points = 0;
        for value in elements {
            let Value::Object(mut fields) = value else { return Err(anyhow!("invalid drawing element")) };
            if fields.get("isDeleted") == Some(&Value::Bool(true)) || fields.get("type").and_then(Value::as_str) == Some("selection") { continue; }
            migrate_element(&mut fields);
            let mut element = load_element(fields.clone())?;
            points += element_points(&mut element).map_or(0, |p| p.len());
            ensure!(points <= MAX_SCENE_POINTS && seen.insert(element.id().to_string()), "too many points or duplicate element id");
            result.original.insert(element.id().into(), (fields, official_element(&element)?));
            result.scene.elements.push(element);
        }
        ensure!(result.scene.elements.len() <= MAX_ELEMENTS, "too many drawing elements");
        if let Some(files) = template.get_mut("files").filter(|f| !f.is_null()) {
            let used: HashSet<_> = result.scene.non_deleted().filter_map(|e| match e { Element::Image(i) => i.file_id.as_deref(), _ => None }).collect();
            if let Value::Object(map) = files { map.retain(|id, _| used.contains(id.as_str())); }
            let mut files: HashMap<String, BinaryFileData> = serde_json::from_value(files.clone())?;
            ensure!(files.len() <= MAX_IMAGES, "too many drawing images");
            let mut pixels = 0;
            for file in files.values_mut() {
                let image = drawing_image(file)?;
                let (w, h) = image.dimensions()?;
                pixels += u64::from(w) * u64::from(h);
                ensure!(w <= 4096 && h <= 4096 && pixels <= MAX_IMAGE_PIXELS, "drawing images exceed the pixel limit");
                image.normalize_bitmap(file)?;
            }
            result.files = files;
        }
        result.template = template;
        Ok(result)
    }

    pub(super) fn json(&self) -> Result<Vec<u8>> {
        let mut root = self.template.clone();
        root.insert("type".into(), json!("excalidraw"));
        root.entry("version").or_insert_with(|| json!(2));
        root.entry("source").or_insert_with(|| json!("https://quiet.md"));
        let state = root.entry("appState").or_insert_with(|| json!({}));
        if !state.is_object() { *state = json!({}); }
        state["viewBackgroundColor"] = json!(self.background);
        if self.grid_size.fract() == 0. { state["gridSize"] = json!(self.grid_size as u64); }
        else { state["gridSize"] = json!(self.grid_size); }
        state["gridStep"] = json!(self.grid_step);
        state["gridModeEnabled"] = json!(self.grid_enabled);
        let original_files = self.template.get("files");
        let mut files = Map::new();
        for element in self.scene.non_deleted() {
            if let Element::Image(image) = element && let Some(id) = &image.file_id && !files.contains_key(id) {
                if let Some(raw) = original_files.and_then(|f| f.get(id)) { files.insert(id.clone(), raw.clone()); }
                else if let Some(file) = self.files.get(id) { files.insert(id.clone(), serde_json::to_value(file)?); }
            }
        }
        root.insert("files".into(), Value::Object(files));
        let elements = self.scene.non_deleted().map(|element| {
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

    fn render_export(&self, background: &str, options: ExportOptions) -> String {
        export::SvgExporter::export_region(&self.scene, background, &self.files, options.dark, options.show_frames, self.export_region)
    }

    #[cfg(test)]
    fn sample_color(&self, p: Point, dark: bool, show_frames: bool) -> Option<String> {
        let b = export::visible_bounds(&self.scene, show_frames)?;
        let svg = self.render_export(&self.background, ExportOptions { dark, show_frames, ..Default::default() });
        let [r, g, b, a] = export::sample_svg(&svg, (p.x - b.min_x + export::DEFAULT_EXPORT_PADDING) as f32,
            (p.y - b.min_y + export::DEFAULT_EXPORT_PADDING) as f32)?;
        (a > 0).then(|| format!("#{r:02x}{g:02x}{b:02x}"))
    }

    pub(super) fn svg(&self) -> String { export::SvgExporter::export_with_files(&self.scene, &self.background, &self.files) }

    #[cfg(test)]
    pub(super) fn standalone_svg(&self) -> String { self.standalone_svg_with(ExportOptions::default()) }

    fn standalone_svg_with(&self, options: ExportOptions) -> String {
        use std::fmt::Write;
        let fonts: HashSet<_> = self.scene.non_deleted().filter_map(|e| match e {
            Element::Text(t) => Some(t.font_family),
            _ => None,
        }).collect();
        let mut css = String::new();
        for &(family, data) in quiet_excalidraw::fonts::BUNDLED {
            if fonts.contains(&family) {
                let _ = write!(css, "@font-face{{font-family:'{}';src:url(data:font/ttf;base64,{}) format('truetype')}}", family.name(),
                    base64::engine::general_purpose::STANDARD.encode(data));
            }
        }
        let mut svg = self.render_export(if options.background { &self.background } else { "transparent" }, options);
        if !css.is_empty() && let Some(start) = svg.find('>') {
            svg.insert_str(start + 1, &format!("<style>{css}</style>"));
        }
        svg
    }

    pub(super) fn preview_dimensions(&self) -> Result<(u32, u32)> {
        export::raster_dimensions(&self.svg(), PREVIEW_SCALE).map_err(|s| anyhow!(s))
    }

    pub(super) fn png(&self) -> Result<Vec<u8>> {
        export::rasterize_png(&self.svg(), PREVIEW_SCALE).map_err(|s| anyhow!(s))
    }
}

pub(crate) fn source_bytes(root: &Path, source: &str) -> Result<Vec<u8>> {
    store::read_drawing_asset(root, source, MAX_SOURCE)
}

fn legacy_png(root: &Path, preview: &str) -> Result<Vec<u8>> {
    let bytes = store::read_drawing_asset(root, preview, MAX_PNG)?;
    let (w, h) = image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png).into_dimensions()?;
    ensure!(w.max(h) <= MAX_PREVIEW_EDGE + 1, "drawing preview exceeds the pixel limit");
    Ok(bytes)
}

pub(crate) fn preview_png(root: &Path, d: &Drawing) -> Result<Vec<u8>> {
    if !d.preview.is_empty() && let Ok(bytes) = legacy_png(root, &d.preview) { return Ok(bytes) }
    DrawingScene::read(root, &d.source)?.png()
}

pub(super) fn preview_pixels(root: &Path, source: &str, preview: &str, edge: u32) -> Result<image::RgbaImage> {
    let edge = if edge == 0 { MAX_PREVIEW_EDGE } else { edge.min(MAX_PREVIEW_EDGE) };
    if !preview.is_empty() && let Ok(bytes) = legacy_png(root, preview)
        && let Ok(mut im) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png) {
        if im.width().max(im.height()) > edge { im = im.thumbnail(edge, edge); }
        let mut pixels = im.into_rgba8();
        for p in pixels.as_chunks_mut::<4>().0 { p.swap(0, 2); }
        return Ok(pixels);
    }
    let doc = DrawingScene::read(root, source)?;
    let raster = export::rasterize_pixels(&doc.svg(), PREVIEW_SCALE, edge).map_err(|s| anyhow!(s))?;
    let (w, h) = (raster.width(), raster.height());
    let mut bytes = raster.take();
    // tiny-skia uses premultiplied RGBA, GPUI uses straight BGRA.
    for p in bytes.as_chunks_mut::<4>().0 {
        let alpha = u32::from(p[3]);
        if alpha > 0 && alpha < 255 {
            for c in &mut p[..3] { *c = ((u32::from(*c) * 255 + alpha / 2) / alpha) as u8; }
        }
        p.swap(0, 2);
    }
    Ok(image::RgbaImage::from_raw(w, h, bytes).expect("pixmap dimensions match its pixels"))
}

pub(super) enum DrawingImage { Bitmap(Vec<u8>, image::ImageFormat), Svg(String) }

impl DrawingImage {
    pub(super) fn dimensions(&self) -> Result<(u32, u32)> {
        match self {
            Self::Bitmap(bytes, format) => Ok(image::ImageReader::with_format(Cursor::new(bytes), *format).into_dimensions()?),
            Self::Svg(svg) => export::intrinsic_svg_size(svg).map_err(|s| anyhow!(s)),
        }
    }

    pub(super) fn normalize_bitmap(self, file: &mut BinaryFileData) -> Result<()> {
        if matches!(self, Self::Bitmap(_, image::ImageFormat::Bmp | image::ImageFormat::Ico)) {
            let mut png = Cursor::new(Vec::new());
            self.pixels()?.write_to(&mut png, image::ImageFormat::Png)?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
            file.mime_type = "image/png".into();
            file.data_url = format!("data:image/png;base64,{encoded}");
        }
        Ok(())
    }

    pub(super) fn pixels(self) -> Result<image::RgbaImage> {
        Ok(match self {
            Self::Bitmap(bytes, format) => image::load_from_memory_with_format(&bytes, format)?.into_rgba8(),
            Self::Svg(svg) => {
                let image = export::rasterize_pixels(&svg, 1., 4096).map_err(|s| anyhow!(s))?;
                let png = image.encode_png().map_err(|e| anyhow!(e))?;
                image::load_from_memory_with_format(&png, image::ImageFormat::Png)?.into_rgba8()
            }
        })
    }
}

pub(super) fn drawing_image(file: &BinaryFileData) -> Result<DrawingImage> {
    let (mime, encoded) = file.data_url.split_once(";base64,").ok_or_else(|| anyhow!("expected an embedded drawing image"))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    if mime == "data:image/svg+xml" { return Ok(DrawingImage::Svg(String::from_utf8(bytes)?)); }
    let format = match mime {
        "data:image/png" => image::ImageFormat::Png, "data:image/jpeg" | "data:image/jfif" => image::ImageFormat::Jpeg,
        "data:image/gif" => image::ImageFormat::Gif, "data:image/webp" => image::ImageFormat::WebP,
        "data:image/bmp" => image::ImageFormat::Bmp, "data:image/x-icon" => image::ImageFormat::Ico,
        _ => return Err(anyhow!("unsupported embedded image format")),
    };
    Ok(DrawingImage::Bitmap(bytes, format))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quiet_excalidraw::core::{arrowhead::Arrowhead, bounds::element_bounds, factory::{ElementOptions, new_arrow}, transform::translate_element};

    #[test]
    fn official_legacy_exports_and_library_reopen() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/harness/fixtures/excalidraw");
        for name in ["test_embedded_v1.png", "test_embedded_v1.svg", "smiley_embedded_v2.png", "smiley_embedded_v2.svg"] {
            let doc = DrawingScene::load_exported(&std::fs::read(root.join(name)).unwrap()).unwrap();
            assert!(!doc.scene.elements.is_empty(), "{name}");
            assert_eq!(DrawingScene::load(&doc.json().unwrap()).unwrap().scene.elements, doc.scene.elements, "{name}");
        }
        let items = DrawingScene::load_library(&std::fs::read(root.join("fixture_library.excalidrawlib")).unwrap()).unwrap();
        let saved = DrawingScene::library_json(&items).unwrap();
        let again = DrawingScene::load_library(&saved).unwrap();
        assert_eq!(again[0].library_item, items[0].library_item);
        assert_eq!(again[0].scene.elements, items[0].scene.elements);
    }

    #[test]
    fn long_text_deleted_elements_and_unused_files_round_trip() {
        use quiet_excalidraw::core::factory::new_text;
        let mut original = DrawingScene::new();
        let text = "长文字".repeat(1050);
        original.scene.elements.push(new_text(0., 0., &text, 16., &ElementOptions::default()));
        original.scene.elements[0].base_mut().width = 120.;
        let mut raw: Value = serde_json::from_slice(&original.json().unwrap()).unwrap();
        raw["files"]["unused"] = json!({"dataURL":"obsolete"});
        let mut doc = DrawingScene::load(&serde_json::to_vec(&raw).unwrap()).unwrap();
        let mut deleted = new_text(5., 5., "removed text", 16., &ElementOptions::default());
        deleted.base_mut().is_deleted = true;
        doc.scene.elements.push(deleted);
        let svg = doc.editable_svg().unwrap();
        let payload = svg.split_once("<!-- payload-start -->").unwrap().1.split_once("<!-- payload-end -->").unwrap().0;
        let svg_json = decode_metadata(&base64::engine::general_purpose::STANDARD.decode(payload).unwrap()).unwrap();
        let png_json = embedded_png_source(&doc.editable_png().unwrap()).unwrap();
        for bytes in [doc.json().unwrap(), svg_json, png_json] {
            let raw: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(raw["elements"].as_array().unwrap().len(), 1);
            assert_eq!(raw["elements"][0]["text"], text);
            assert_eq!(raw["files"], json!({}));
        }
    }

    #[test]
    fn png_export_and_preview_have_separate_edge_limits() {
        let mut doc = DrawingScene::new();
        doc.scene.elements.push(quiet_excalidraw::factory::new_rectangle(0., 0., 2100., 20., &ElementOptions::default()));
        assert_eq!(doc.preview_dimensions().unwrap().0, 2048);
        let bytes = doc.export_png(ExportOptions::default()).unwrap();
        assert_eq!(image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png).into_dimensions().unwrap().0, 4096);
    }

    #[test]
    fn png_svg_and_library_keep_editable_unicode_source() {
        let mut doc = DrawingScene::new();
        doc.scene.elements.push(quiet_excalidraw::core::factory::new_text(10., 20., "中文 🚀 <&>", 20., &ElementOptions::default()));
        let original: Value = serde_json::from_slice(&doc.json().unwrap()).unwrap();
        for bytes in [doc.editable_png().unwrap(), doc.editable_svg().unwrap().into_bytes()] {
            let reopened = DrawingScene::load_exported(&bytes).unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&reopened.json().unwrap()).unwrap(), original);
        }
        let legacy = export::embed_png_scene_data(&doc.png().unwrap(), std::str::from_utf8(&doc.json().unwrap()).unwrap());
        assert_eq!(DrawingScene::load_exported(&legacy).unwrap().scene.elements, doc.scene.elements);
        let items = DrawingScene::load_library(&DrawingScene::library_json(&[doc.clone()]).unwrap()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].scene.elements, doc.scene.elements);
    }

    #[test]
    fn export_options_control_transparency_scale_theme_and_source() {
        let mut doc = DrawingScene::new();
        doc.scene.elements.push(quiet_excalidraw::core::factory::new_rectangle(0., 0., 100., 50., &ElementOptions::default()));
        let options = ExportOptions { background: false, dark: true, scale: 1., ..ExportOptions::default() };
        let bytes = doc.export_png(options).unwrap(); let png = image::load_from_memory(&bytes).unwrap().into_rgba8();
        assert_eq!(png.dimensions(), (120, 70)); assert_eq!(png.get_pixel(0, 0)[3], 0);
        assert!(DrawingScene::load_exported(&bytes).is_err());
        let bytes = doc.export_png(ExportOptions { scale: 3., embed: true, ..options }).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 360);
        assert_eq!(DrawingScene::load_exported(&bytes).unwrap().scene.elements, doc.scene.elements);
        let light = doc.export_svg(ExportOptions { dark: false, ..options }).unwrap();
        let dark = doc.export_svg(options).unwrap();
        assert_ne!(dark, light); assert!(!dark.contains("payload-start"));
    }

    #[test]
    fn transformed_image_pixels_and_selection_assets_match_the_source() {
        use quiet_excalidraw::core::{factory::new_image, element::ImageCrop};
        let pixels = image::RgbaImage::from_fn(4, 2, |x, _| if x < 2 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) });
        let mut png = Cursor::new(Vec::new()); pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let mut doc = DrawingScene::new();
        let data = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png.into_inner()));
        doc.files.insert("used".into(), BinaryFileData::new("used", "image/png", data.clone()));
        doc.files.insert("unused".into(), BinaryFileData::new("unused", "image/png", data));
        let mut e = new_image(0., 0., 200., 100., Some("used".into()), &ElementOptions::default());
        if let Element::Image(i) = &mut e { i.scale = [-1., 1.]; }
        doc.scene.elements.push(e);
        let raster = image::load_from_memory(&doc.png().unwrap()).unwrap().into_rgba8();
        assert_eq!(raster.get_pixel(80, 100).0, [0, 0, 255, 255]);
        assert_eq!(raster.get_pixel(340, 100).0, [255, 0, 0, 255]);
        if let Element::Image(i) = &mut doc.scene.elements[0] { i.crop = Some(ImageCrop { x: 2., y: 0., width: 2., height: 2., natural_width: 4., natural_height: 2. }); i.base.angle = std::f64::consts::FRAC_PI_2; }
        let raster = image::load_from_memory(&doc.png().unwrap()).unwrap().into_rgba8();
        assert_eq!(raster.get_pixel(raster.width() / 2, raster.height() / 2).0, [0, 0, 255, 255]);
        let mut doc = DrawingScene::load(&doc.json().unwrap()).unwrap();
        assert_eq!(doc.files.len(), 1);
        doc.scene.elements.clear(); doc.retain_used_files();
        assert!(doc.files.is_empty());
        let json: Value = serde_json::from_slice(&doc.json().unwrap()).unwrap();
        assert_eq!(json["files"], json!({}));
    }

    #[test]
    fn embedded_svg_images_render_on_canvas_and_in_export() {
        use quiet_excalidraw::factory::new_image;
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="#ff0000"/></svg>"##;
        let id = "vector";
        let data = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg));
        let mut doc = DrawingScene::new();
        doc.files.insert(id.into(), BinaryFileData::new(id, "image/svg+xml", data.clone()));
        doc.scene.elements.push(new_image(0., 0., 80., 40., Some(id.into()), &ElementOptions::default()));
        let doc = DrawingScene::load(&doc.json().unwrap()).unwrap();
        let image = drawing_image(&doc.files[id]).unwrap();
        assert_eq!(image.dimensions().unwrap(), (80,40));
        assert_eq!(image.pixels().unwrap().get_pixel(40,20).0, [255,0,0,255]);
        assert_eq!(doc.sample_color(Point::new(40.,20.),false,true).as_deref(), Some("#ff0000"));
        assert_eq!(doc.files[id].data_url, data);
    }

    #[test]
    fn frame_export_clips_children_without_changing_their_geometry() {
        use quiet_excalidraw::core::factory::{new_frame, new_rectangle};
        let mut doc = DrawingScene::new(); let options = ElementOptions::default();
        let frame = new_frame(0., 0., 100., 100., None, &options);
        let mut child = new_rectangle(50., 50., 100., 100., &options);
        child.base_mut().frame_id = Some(frame.id().to_string());
        child.base_mut().background_color = "#ff0000".into(); child.base_mut().fill_style = quiet_excalidraw::core::types::FillStyle::Solid;
        doc.scene.elements = vec![frame, child];
        let raster = image::load_from_memory(&doc.png().unwrap()).unwrap().into_rgba8();
        assert_eq!(raster.get_pixel(160, 160).0, [255, 0, 0, 255]);
        assert_eq!(raster.dimensions(), (240,240));
        assert_eq!(raster.get_pixel(235, 160).0, [255, 255, 255, 255]);
        assert_eq!(doc.sample_color(Point::new(130.,80.),false,false).as_deref(), Some("#ff0000"));
        assert_eq!(doc.scene.elements[1].base().width, 100.);
    }

    #[test]
    fn selected_frame_export_uses_its_exact_region_and_preserves_editable_scene() {
        use quiet_excalidraw::core::factory::{new_frame, new_rectangle};
        let mut doc = DrawingScene::new(); let style = ElementOptions { roughness: 0., ..Default::default() };
        let frame = new_frame(0., 0., 100., 80., Some("Frame title".into()), &style);
        let mut child = new_rectangle(50., 30., 100., 100., &style);
        child.base_mut().frame_id = Some(frame.id().to_string());
        child.base_mut().background_color = "#ff0000".into();
        child.base_mut().fill_style = quiet_excalidraw::core::types::FillStyle::Solid;
        doc.export_region = Some(element_bounds(&frame)); doc.scene.elements = vec![frame, child];
        let settings = ExportOptions { embed: true, scale: 1., ..ExportOptions::default() };
        let png = doc.export_png(settings).unwrap();
        let pixels = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(pixels.dimensions(), (100,80));
        assert_eq!(pixels.get_pixel(99,60).0, [255,0,0,255]);
        let svg = doc.export_svg(settings).unwrap();
        assert!(!svg.contains("Frame title"));
        assert_eq!(DrawingScene::load_exported(svg.as_bytes()).unwrap().scene.elements, doc.scene.elements);
        assert_eq!(DrawingScene::load_exported(&png).unwrap().scene.elements, doc.scene.elements);
    }

    #[test]
    fn compressed_metadata_and_png_chunk_lengths_are_bounded() {
        let encoded = encode_metadata(&vec![b'x'; MAX_SOURCE as usize + 1]).unwrap();
        assert!(decode_metadata(encoded.as_bytes()).is_err());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&u32::MAX.to_be_bytes()); png.extend_from_slice(b"tEXt1234");
        assert!(DrawingScene::load_exported(&png).is_err());
    }

    #[test]
    fn mixed_text_exports_han_in_the_selected_style() {
        use quiet_excalidraw::core::{factory::new_text, types::FontFamily};
        let mut doc = DrawingScene::new();
        doc.scene.elements.push(new_text(0., 0., "A&画\u{e0100}图！B", 24., &ElementOptions::default()));
        for family in [FontFamily::Excalifont, FontFamily::Nunito, FontFamily::ComicShanns, FontFamily::Lilita] {
            let Element::Text(t) = &mut doc.scene.elements[0] else { unreachable!() };
            t.font_family = family;
            let svg = DrawingScene::load(&doc.json().unwrap()).unwrap().svg();
            assert!(svg.contains(&format!("A&amp;<tspan font-family=\"{},", family.han_families()[0])));
            assert!(svg.contains(">画\u{e0100}图！</tspan>B</tspan>"));
        }
    }

    #[test]
    fn bound_text_style_survives_layout_and_export() {
        use quiet_excalidraw::core::{binding::sync_container_text, factory::{new_rectangle, new_text}, types::{FontFamily, TextAlign, VerticalAlign}};
        let mut doc = DrawingScene::new();
        let style = ElementOptions::default();
        let rect = new_rectangle(10., 20., 180., 130., &style);
        let mut label = new_text(0., 0., "Quiet.md", 12., &style);
        let Element::Text(t) = &mut label else { unreachable!() };
        t.container_id = Some(rect.id().into());
        t.font_family = FontFamily::ComicShanns;
        t.text_align = TextAlign::Right;
        t.vertical_align = VerticalAlign::Bottom;
        doc.scene.elements = vec![rect, label];
        sync_container_text(&mut doc.scene);
        let mut doc = DrawingScene::load(&doc.json().unwrap()).unwrap();
        sync_container_text(&mut doc.scene);
        let png = doc.png().unwrap();
        let svg = doc.standalone_svg();
        assert!(svg.contains("@font-face{font-family:'Comic Shanns Regular'"));
        assert!(svg.contains("text-anchor=\"end\""));
        let Element::Text(t) = &mut doc.scene.elements[1] else { unreachable!() };
        assert_eq!(t.font_size, 12.);
        assert_eq!(t.font_family, FontFamily::ComicShanns);
        assert_eq!(t.text_align, TextAlign::Right);
        assert_eq!(t.vertical_align, VerticalAlign::Bottom);
        assert_eq!(t.base.y + t.base.height, 142.);
        t.font_family = FontFamily::Lilita;
        assert_ne!(png, doc.png().unwrap());
    }

    #[test]
    fn excalidraw_local_points_and_unknown_fields_survive_an_edit() {
        let mut scene = DrawingScene::new();
        scene.scene.elements.push(new_arrow(vec![Point::new(190., 85.), Point::new(25., 95.)], None, Some(Arrowhead::Arrow), &ElementOptions::default()));
        let mut raw: Value = serde_json::from_slice(&scene.json().unwrap()).unwrap();
        assert_eq!(raw["elements"][0]["x"], 190.);
        assert_eq!(raw["elements"][0]["points"], json!([[0., 0.], [-165., 10.]]));
        raw["elements"][0]["futureField"] = json!({"kept": true});
        raw["appState"]["gridSize"] = json!(35);
        raw["appState"]["gridStep"] = json!(7);
        raw["appState"]["gridModeEnabled"] = json!(true);
        let mut reopened = DrawingScene::load(raw.to_string().as_bytes()).unwrap();
        let Element::Arrow(arrow) = &reopened.scene.elements[0] else { panic!("expected arrow") };
        assert_eq!(arrow.linear.points[0], Point::new(190., 85.));
        translate_element(&mut reopened.scene.elements[0], 13., -5.);
        let saved: Value = serde_json::from_slice(&reopened.json().unwrap()).unwrap();
        assert_eq!(saved["elements"][0]["x"], 203.);
        assert_eq!(saved["elements"][0]["y"], 80.);
        assert_eq!(saved["elements"][0]["points"], raw["elements"][0]["points"]);
        assert_eq!(saved["elements"][0]["futureField"], raw["elements"][0]["futureField"]);
        assert_eq!(saved["appState"]["gridSize"], 35);
        assert_eq!(saved["appState"]["gridStep"], 7);
        assert_eq!(saved["appState"]["gridModeEnabled"], true);
        let png = reopened.png().unwrap();
        let (w, h) = reopened.preview_dimensions().unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().into_rgba8().dimensions(), (w, h));
        assert!(w > h && h > 0);

        reopened.background = "#fffce8".into();
        let file = BinaryFileData::new("image-1", "image/png", format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)));
        reopened.files.insert(file.id.clone(), file.clone());
        reopened.scene.elements.push(quiet_excalidraw::factory::new_image(0., 0., f64::from(w), f64::from(h), Some(file.id.clone()), &ElementOptions::default()));
        let stored = DrawingScene::load(&reopened.json().unwrap()).unwrap();
        assert_eq!(stored.background, reopened.background);
        assert_eq!(stored.files.get(&file.id), Some(&file));
        assert_eq!(stored.grid_size, 35.);
        assert_eq!(stored.grid_step, 7);
        assert!(stored.grid_enabled);
    }
}
