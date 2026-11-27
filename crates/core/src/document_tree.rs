use super::*;

pub(super) fn subtree<'a>(root: &str, parents: impl Iterator<Item = (&'a String, Option<&'a String>)>) -> Vec<String> {
    let mut children: HashMap<&str, Vec<&String>> = HashMap::new();
    for (id, parent) in parents {
        if let Some(parent) = parent { children.entry(parent).or_default().push(id); }
    }
    let mut ids = vec![root.to_string()];
    let mut i = 0;
    while i < ids.len() {
        if let Some(children) = children.get(ids[i].as_str()) { ids.extend(children.iter().map(|id| (*id).clone())); }
        i += 1;
    }
    ids
}

impl Store {
    /// Independent handle for background tree writes; begin_write reloads under the library lock.
    pub fn tree_writer(&self) -> Self {
        Self { file_changes: vec![], root: self.root.clone(), index: self.index.clone(), trash: self.trash.clone(), manifest: self.manifest.clone(), manifest_cache: Default::default(),
            loaded: Default::default(), loaded_titles: Default::default(), copied: Default::default(), parsed: HashMap::new(), parsed_disk: self.parsed_disk.clone(), warnings: vec![], batch: Default::default() }
    }

    /// Adopt a completed worker's tree and parse cache, keeping open editors'
    /// conflict baselines and attachment copies in this handle.
    pub fn receive_tree(&mut self, writer: Self) {
        assert_eq!(self.root, writer.root);
        self.index = writer.index; self.trash = writer.trash; self.manifest = writer.manifest;
        self.manifest_cache = writer.manifest_cache;
        self.parsed = writer.parsed;
        self.parsed_disk = writer.parsed_disk;
        self.warnings.extend(writer.warnings);
        self.loaded.get_mut().retain(|id, _| self.index.contains_key(id));
        self.loaded_titles.get_mut().retain(|id, _| self.index.contains_key(id));
        self.copied.get_mut().retain(|id, _| self.index.contains_key(id));
    }

    pub fn take_warnings(&mut self) -> Vec<String> { std::mem::take(&mut self.warnings) }

    pub(super) fn validate_parent(&self, id: Option<&str>, parent: Option<&str>) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        let mut p = parent;
        while let Some(key) = p {
            anyhow::ensure!(Some(key) != id && seen.insert(key), "不能移动到自身或子文档中");
            let m = self.index.get(key).filter(|m| !m.local()).ok_or_else(|| anyhow::anyhow!("目标文档不存在"))?;
            p = m.parent.as_deref();
        }
        Ok(())
    }

    pub fn initialize_manual_order(&mut self) -> Result<()> {
        let _lock = self.begin_write()?;
        let mut settings = self.settings();
        let initialized = settings.home_manual_initialized;
        let previous = self.index.clone();
        let mut groups: HashMap<Option<String>, Vec<String>> = HashMap::new();
        for (id, m) in self.index.iter().filter(|(_, m)| !m.local()) { groups.entry(m.parent.clone()).or_default().push(id.clone()); }
        let mut changed = false;
        // Freeze the order the sidebar shows (`Meta::place`): pages not yet placed
        // keep their spot at the top instead of dropping to the end.
        for (parent, mut ids) in groups {
            ids.sort_by(|a, b| (self.index[a].place(), a).cmp(&(self.index[b].place(), b)));
            if initialized && ids.iter().all(|id| self.index[id].place().0) { continue; }
            changed = true;
            for (i, id) in ids.iter().enumerate() {
                let m = self.index.get_mut(id).unwrap();
                if parent.is_none() { m.root_order = Some(i as u64 + 1); }
                else { m.order = i as u64 + 1; m.root_order = None; }
            }
        }
        if !changed && initialized { return Ok(()); }
        if let Err(e) = self.save_index() { self.index = previous; return Err(e); }
        self.record_tree_changes(&previous);
        settings.home_manual_initialized = true;
        self.save_settings(&settings)
    }

    /// Persist one drop atomically; neither document content nor timestamps change.
    pub fn move_in_tree(&mut self, id: &str, parent: Option<&str>, before: Option<&str>) -> Result<()> {
        let _lock = self.begin_write()?;
        anyhow::ensure!(self.index.get(id).is_some_and(|m| !m.local()) && self.index_has(id), "文档不存在");
        self.validate_parent(Some(id), parent)?;
        if let Some(anchor) = before {
            anyhow::ensure!(self.index.get(anchor).is_some_and(|m| m.parent.as_deref() == parent) && self.index.get(anchor).is_some_and(|m| !m.local()) && self.index_has(anchor), "目标位置已改变，请重试");
            if anchor == id { return Ok(()); }
        }
        let mut siblings: Vec<String> = self.index.iter().filter(|(k, m)| m.parent.as_deref() == parent && !m.local() && self.index_has(k)).map(|(k, _)| k.clone()).collect();
        // The sidebar's order (`Meta::place`, ties by id), so the numbers written
        // here match the order the drop was made in.
        siblings.sort_by(|ka, kb| (self.index[ka].place(), ka).cmp(&(self.index[kb].place(), kb)));
        siblings.retain(|key| key != id);
        let pos = before.and_then(|key| siblings.iter().position(|x| x == key)).unwrap_or(siblings.len());
        siblings.insert(pos, id.to_string());
        let previous = self.index.clone();
        self.index.get_mut(id).unwrap().parent = parent.map(str::to_string);
        for (i, key) in siblings.iter().enumerate() {
            let m = self.index.get_mut(key).unwrap();
            if parent.is_none() { m.root_order = Some(i as u64 + 1); }
            else { m.order = i as u64 + 1; m.root_order = None; }
        }
        if let Err(e) = self.save_index() { self.index = previous; return Err(e); }
        self.record_tree_changes(&previous);
        Ok(())
    }
}
