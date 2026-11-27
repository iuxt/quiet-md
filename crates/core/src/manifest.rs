//! Shared library manifest. Folders are identities, never paths on disk.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use crate::store::{InfoRow, Meta, PageWidth};

/// Version of the shared manifest and its on-disk library layout.
pub const VERSION: u32 = 2;
pub const FILE: &str = "quiet.json";
pub const NOTES: &str = "Notes";
pub(crate) const MAX_BYTES: usize = 16 << 20;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub folders: Vec<Folder>,
    pub notes: Vec<Note>,
    #[serde(default)] pub trash: Vec<Deleted>,
    #[serde(flatten)] pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for Manifest {
    fn default() -> Self { Self { version: VERSION, folders: vec![], notes: vec![], trash: vec![], extra: BTreeMap::new() } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
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
    #[serde(rename = "folderId", default)] pub folder: Option<String>,
    #[serde(rename = "createdAt", default, with = "timestamp")] pub created: u64,
    #[serde(rename = "updatedAt", default, with = "timestamp")] pub modified: u64,
    #[serde(default, skip_serializing_if = "Options::is_default")] pub options: Options,
    #[serde(flatten)] pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Deleted {
    pub id: String,
    pub title: String,
    /// None is a virtual folder; a note keeps its original UUID filename.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub filename: Option<String>,
    #[serde(rename = "folderId", default)] pub parent: Option<String>,
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

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::check_size(bytes.len())?;
        let manifest: Self = serde_json::from_slice(bytes).context("quiet.json 清单无效")?;
        manifest.validate()?; Ok(manifest)
    }

    pub(crate) fn check_size(len: usize) -> Result<()> {
        ensure!(len <= MAX_BYTES, "文档库清单超过大小限制");
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == VERSION, "不支持的文档库清单版本");
        let mut ids = HashSet::new();
        let mut insert = |id: &str| -> Result<()> {
            ensure!(uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id) && ids.insert(id.to_string()), "清单中存在无效或重复的 UUID"); Ok(())
        };
        for f in &self.folders { insert(&f.id)?; ensure!(!f.name.trim().is_empty(), "文件夹名称不能为空"); }
        for n in &self.notes { insert(&n.id)?; validate_filename(&n.id, &n.filename)?; }
        for t in &self.trash { insert(&t.id)?; if let Some(filename) = &t.filename { validate_filename(&t.id, filename)?; } }
        let parents: HashMap<_, _> = self.folders.iter().map(|f| (f.id.as_str(), f.parent.as_deref())).collect();
        let mut complete = HashSet::new();
        for f in &self.folders {
            if complete.contains(f.id.as_str()) { continue }
            let mut seen = HashSet::from([f.id.as_str()]); let mut parent = f.parent.as_deref();
            while let Some(id) = parent {
                if complete.contains(id) { break }
                ensure!(seen.insert(id), "文件夹层级不能包含循环");
                parent = *parents.get(id).context("父文件夹不存在")?;
            }
            complete.extend(seen);
        }
        for n in &self.notes { ensure!(n.folder.as_deref().is_none_or(|p| parents.contains_key(p)), "文章所在的文件夹不存在"); }
        Ok(())
    }

    pub fn index(&self) -> HashMap<String, Meta> {
        let mut index = HashMap::new();
        for f in &self.folders {
            index.insert(f.id.clone(), Meta { folder: Some(f.name.clone()), parent: f.parent.clone(), created: f.created, modified: f.modified, ..f.options.meta() });
        }
        for n in &self.notes {
            index.insert(n.id.clone(), Meta { parent: n.folder.clone(), created: n.created, modified: n.modified,
                name: Some(n.title.clone()), markdown: n.filename.ends_with(".md"), rel: format!("{NOTES}/{}", n.filename), ..n.options.meta() });
        }
        index
    }

    pub fn deleted_meta(t: &Deleted) -> Meta {
        Meta { parent: t.parent.clone(), created: t.created, modified: t.modified,
            folder: t.filename.is_none().then(|| t.title.clone()), name: Some(t.title.clone()),
            markdown: t.filename.as_ref().is_some_and(|f| f.ends_with(".md")),
            rel: t.filename.as_ref().map(|f| format!("{NOTES}/{f}")).unwrap_or_default(), ..t.options.meta() }
    }

    pub fn update(&self, index: &HashMap<String, Meta>, trash: impl Iterator<Item = Deleted>) -> Self {
        let mut next = Self { extra: self.extra.clone(), ..Default::default() };
        let extras: HashMap<_, _> = self.notes.iter().map(|n| (&n.id, &n.extra)).chain(self.folders.iter().map(|f| (&f.id, &f.extra)))
            .chain(self.trash.iter().map(|t| (&t.id, &t.extra))).collect();
        for (id, m) in index.iter().filter(|(_, m)| !m.local()) {
            if let Some(name) = &m.folder {
                let extra = extras.get(id).map(|extra| (*extra).clone()).unwrap_or_default();
                next.folders.push(Folder { id: id.clone(), name: name.clone(), parent: m.parent.clone(), created: m.created, modified: m.modified, options: Options::of(m), extra });
            } else {
                let extra = extras.get(id).map(|extra| (*extra).clone()).unwrap_or_default();
                next.notes.push(Note { id: id.clone(), title: m.name.clone().unwrap_or_default(), filename: format!("{id}.{}", if m.markdown { "md" } else { "xml" }),
                    folder: m.parent.clone(), created: m.created, modified: m.modified, options: Options::of(m), extra });
            }
        }
        // Explicit order fields determine the sidebar; stable id ordering minimizes Git noise.
        next.folders.sort_by(|a, b| a.id.cmp(&b.id)); next.notes.sort_by(|a, b| a.id.cmp(&b.id));
        next.trash = trash.collect(); next.trash.sort_by(|a, b| a.id.cmp(&b.id)); next
    }
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
