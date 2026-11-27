//! Shared library manifest. Every document has a fixed body path and an optional parent.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use crate::store::{InfoRow, Meta, PageWidth};
use crate::model::{Document, RichText};

/// Version of the shared manifest and its on-disk library layout.
pub const VERSION: u32 = 3;
const LEGACY_VERSION: u32 = 2;
pub const FILE: &str = "quiet.json";
pub const NOTES: &str = "Notes";
pub(crate) const MAX_BYTES: usize = 16 << 20;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    #[serde(rename = "folders", default, skip_serializing)] legacy_folders: Option<Vec<LegacyFolder>>,
    pub notes: Vec<Note>,
    #[serde(default)] pub trash: Vec<Deleted>,
    #[serde(flatten)] pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for Manifest {
    fn default() -> Self { Self { version: VERSION, legacy_folders: None, notes: vec![], trash: vec![], extra: BTreeMap::new() } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct LegacyFolder {
    pub id: String,
    pub name: String,
    #[serde(rename = "parentId", default)] pub parent: Option<String>,
    #[serde(rename = "createdAt", default, with = "timestamp")] pub created: u64,
    #[serde(rename = "updatedAt", default, with = "timestamp")] pub modified: u64,
    #[serde(default, skip_serializing_if = "Options::is_default")] pub options: Options,
    #[serde(flatten)] pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub filename: String,
    #[serde(rename = "parentId", alias = "folderId", default)] pub parent: Option<String>,
    #[serde(rename = "createdAt", default, with = "timestamp")] pub created: u64,
    #[serde(rename = "updatedAt", default, with = "timestamp")] pub modified: u64,
    #[serde(default, skip_serializing_if = "Options::is_default")] pub options: Options,
    #[serde(flatten)] pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Deleted {
    pub id: String,
    pub title: String,
    /// Missing only on a legacy deleted folder.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub filename: Option<String>,
    #[serde(rename = "parentId", alias = "folderId", default)] pub parent: Option<String>,
    #[serde(rename = "createdAt", default, with = "timestamp")] pub created: u64,
    #[serde(rename = "updatedAt", default, with = "timestamp")] pub modified: u64,
    #[serde(rename = "deletedAt", with = "timestamp")] pub deleted: u64,
    #[serde(rename = "rootId", default, skip_serializing_if = "Option::is_none")] pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Options::is_default")] pub options: Options,
    #[serde(flatten)] pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    #[serde(skip_serializing_if = "std::ops::Not::not")] pub pinned: bool,
    #[serde(skip_serializing_if = "zero")] pub starred: u64,
    #[serde(skip_serializing_if = "zero")] pub order: u64,
    #[serde(skip_serializing_if = "Option::is_none")] pub root_order: Option<u64>,
    #[serde(skip_serializing_if = "PageWidth::is_standard")] pub width: PageWidth,
    pub catalogue_closed: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")] pub reading: bool,
    #[serde(skip_serializing_if = "InfoRow::is_default")] pub info: InfoRow,
    #[serde(skip_serializing_if = "std::ops::Not::not")] pub refs_hidden: bool,
}

fn zero(n: &u64) -> bool { *n == 0 }
impl Default for Options {
    fn default() -> Self { Self::of(&Meta::default()) }
}
impl Options {
    pub fn of(m: &Meta) -> Self { Self { pinned: m.pinned, starred: m.starred, order: m.order, root_order: m.root_order, width: m.width,
        catalogue_closed: m.catalogue_closed, reading: m.reading, info: m.info, refs_hidden: m.refs_hidden } }
    fn is_default(&self) -> bool { *self == Self::default() }
    fn meta(&self) -> Meta { Meta { pinned: self.pinned, starred: self.starred, order: self.order, root_order: self.root_order, width: self.width,
        catalogue_closed: self.catalogue_closed, reading: self.reading, info: self.info, refs_hidden: self.refs_hidden, ..Default::default() } }
}

impl Deleted {
    pub fn body_rel(&self) -> Option<String> { self.filename.as_ref().map(|f| format!("{NOTES}/{f}")) }
}

pub(crate) struct Upgrade {
    pub manifest: Manifest,
    pub bodies: Vec<(String, String)>,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let manifest = Self::decode(bytes)?;
        manifest.validate()?; Ok(manifest)
    }

    pub(crate) fn parse_historical(bytes: &[u8]) -> Result<Self> {
        let manifest = Self::decode(bytes)?;
        if manifest.version == LEGACY_VERSION { manifest.validate_layout(true)?; }
        else { manifest.validate()?; }
        Ok(manifest)
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        Self::check_size(bytes.len())?;
        serde_json::from_slice(bytes).context("quiet.json 清单无效")
    }

    pub(crate) fn upgrade(bytes: &[u8]) -> Result<Option<Upgrade>> {
        let mut manifest = Self::parse_historical(bytes)?;
        if manifest.version == VERSION { return Ok(None); }
        let mut bodies = vec![];
        let mut add_body = |filename: &str, title: &str| {
            let doc = Document { title: RichText::plain(title), ..Default::default() };
            bodies.push((format!("{NOTES}/{filename}"), crate::docxml::serialize(&doc)));
        };
        for f in manifest.legacy_folders.take().unwrap_or_default() {
            let filename = format!("{}.xml", f.id);
            add_body(&filename, &f.name);
            manifest.notes.push(Note { id: f.id, title: f.name, filename, parent: f.parent,
                created: f.created, modified: f.modified, options: f.options, extra: f.extra });
        }
        for t in manifest.trash.iter_mut().filter(|t| t.filename.is_none()) {
            let filename = format!("{}.xml", t.id);
            add_body(&filename, &t.title);
            t.filename = Some(filename);
        }
        manifest.version = VERSION;
        manifest.notes.sort_by(|a, b| a.id.cmp(&b.id));
        manifest.trash.sort_by(|a, b| a.id.cmp(&b.id));
        manifest.validate()?;
        Ok(Some(Upgrade { manifest, bodies }))
    }

    pub(crate) fn check_size(len: usize) -> Result<()> {
        ensure!(len <= MAX_BYTES, "文档库清单超过大小限制");
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == VERSION, "不支持的文档库清单版本");
        ensure!(self.legacy_folders.is_none(), "清单中不能包含旧文件夹");
        self.validate_layout(false)
    }

    fn validate_layout(&self, legacy: bool) -> Result<()> {
        let mut ids = HashSet::new();
        let mut insert = |id: &str| -> Result<()> {
            ensure!(uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id) && ids.insert(id.to_string()), "清单中存在无效或重复的 UUID"); Ok(())
        };
        let mut parents = HashMap::new();
        for f in self.legacy_folders.iter().flatten() {
            insert(&f.id)?;
            ensure!(!f.name.trim().is_empty(), "文件夹名称不能为空");
            parents.insert(f.id.as_str(), f.parent.as_deref());
        }
        for n in &self.notes {
            insert(&n.id)?;
            validate_filename(&n.id, &n.filename)?;
            if !legacy { parents.insert(n.id.as_str(), n.parent.as_deref()); }
        }
        if legacy {
            for n in &self.notes { ensure!(n.parent.as_deref().is_none_or(|p| parents.contains_key(p)), "文章所在的文件夹不存在"); }
        }
        validate_parents(&parents, false)?;

        let mut deleted_parents = HashMap::new();
        for t in &self.trash {
            insert(&t.id)?;
            if let Some(filename) = &t.filename { validate_filename(&t.id, filename)?; }
            else { ensure!(legacy, "回收站文档缺少正文文件名"); }
            deleted_parents.insert(t.id.as_str(), t.parent.as_deref());
        }
        validate_parents(&deleted_parents, true)
    }

    pub fn index(&self) -> HashMap<String, Meta> {
        let mut index = HashMap::new();
        for n in &self.notes {
            index.insert(n.id.clone(), Meta { parent: n.parent.clone(), created: n.created, modified: n.modified,
                name: Some(n.title.clone()), markdown: n.filename.ends_with(".md"), rel: format!("{NOTES}/{}", n.filename), ..n.options.meta() });
        }
        index
    }

    pub fn deleted_meta(t: &Deleted) -> Meta {
        Meta { parent: t.parent.clone(), created: t.created, modified: t.modified,
            name: Some(t.title.clone()),
            markdown: t.filename.as_ref().is_some_and(|f| f.ends_with(".md")),
            rel: t.filename.as_ref().map(|f| format!("{NOTES}/{f}")).unwrap_or_default(), ..t.options.meta() }
    }

    pub fn update(&self, index: &HashMap<String, Meta>, trash: impl Iterator<Item = Deleted>) -> Self {
        let mut next = Self { extra: self.extra.clone(), ..Default::default() };
        let extras: HashMap<_, _> = self.notes.iter().map(|n| (&n.id, &n.extra))
            .chain(self.trash.iter().map(|t| (&t.id, &t.extra))).collect();
        for (id, m) in index.iter().filter(|(_, m)| !m.local()) {
            let extra = extras.get(id).map(|extra| (*extra).clone()).unwrap_or_default();
            next.notes.push(Note { id: id.clone(), title: m.name.clone().unwrap_or_default(), filename: format!("{id}.{}", if m.markdown { "md" } else { "xml" }),
                parent: m.parent.clone(), created: m.created, modified: m.modified, options: Options::of(m), extra });
        }
        // Explicit order fields determine the sidebar; stable id ordering minimizes Git noise.
        next.notes.sort_by(|a, b| a.id.cmp(&b.id));
        next.trash = trash.collect(); next.trash.sort_by(|a, b| a.id.cmp(&b.id)); next
    }
}

fn validate_parents<'a>(parents: &HashMap<&'a str, Option<&'a str>>, allow_missing: bool) -> Result<()> {
    let mut complete = HashSet::new();
    for &id in parents.keys() {
        if complete.contains(id) { continue }
        let mut seen = HashSet::new();
        let mut next = Some(id);
        while let Some(id) = next {
            if complete.contains(id) { break }
            ensure!(seen.insert(id), "文档层级不能包含循环");
            next = match parents.get(id) {
                Some(parent) => *parent,
                None if allow_missing => break,
                None => anyhow::bail!("父文档不存在"),
            };
        }
        complete.extend(seen);
    }
    Ok(())
}

pub fn validate_filename(id: &str, filename: &str) -> Result<()> {
    ensure!(filename == format!("{id}.xml") || filename == format!("{id}.md"), "文章文件名必须是固定 UUID.xml 或 UUID.md"); Ok(())
}

mod timestamp {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        let time = i64::try_from(*value).ok().and_then(|v| OffsetDateTime::from_unix_timestamp(v).ok()).ok_or_else(|| serde::ser::Error::custom("无效时间戳"))?;
        serializer.serialize_str(&time.format(&Rfc3339).map_err(serde::ser::Error::custom)?)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)] #[serde(untagged)] enum Value { Number(u64), Text(String) }
        match Value::deserialize(deserializer)? {
            Value::Number(n) => Ok(n),
            Value::Text(text) => OffsetDateTime::parse(&text, &Rfc3339).map_err(D::Error::custom)
                .and_then(|time| u64::try_from(time.unix_timestamp()).map_err(D::Error::custom)),
        }
    }
}
