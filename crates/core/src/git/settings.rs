use super::*;

const SETTINGS: &str = ".quiet/git.json";

impl LibraryRepo {
    pub fn settings_at(root: &Path) -> Result<Config> {
        match read_settings(&safe_path(root, SETTINGS)?)? {
            Some(config) => Ok(config),
            None if root.join(".git").is_dir() => Self::open(root)?.legacy_config(),
            None => Ok(Config::default()),
        }
    }

    pub fn config(&self) -> Result<Config> {
        let path = safe_path(&self.root, SETTINGS)?;
        let mut config = match read_settings(&path)? { Some(config) => config, None => self.legacy_config()? };
        config.branch = self.repo.head().ok().and_then(|h| h.shorthand().ok().map(str::to_string))
            .or_else(|| self.repo.find_reference("HEAD").ok().and_then(|h| h.symbolic_target().ok().flatten().and_then(|s| s.strip_prefix("refs/heads/")).map(str::to_string)))
            .unwrap_or_else(|| "main".into());
        Ok(config)
    }

    /// Import older repositories under the library write lock, without changing Git's config.
    pub(crate) fn migrate_config(&self) -> Result<()> {
        let path = safe_path(&self.root, SETTINGS)?;
        if read_settings(&path)?.is_none() {
            let config = self.legacy_config()?;
            validate_settings(&config)?;
            crate::store::save_json(&path, &config)?;
        }
        Ok(())
    }

    fn legacy_config(&self) -> Result<Config> {
        let git = self.repo.config()?;
        let mut config = Config {
            name: git.get_string("user.name").unwrap_or_default(), email: git.get_string("user.email").unwrap_or_default(),
            remote: self.repo.find_remote("origin").ok().and_then(|r| r.url().ok().map(str::to_string)).unwrap_or_default(),
            lfs_url: git.get_string("lfs.url").unwrap_or_default(), ssh_key: git.get_string("quiet.sshKey").unwrap_or_default(),
            ..Config::default()
        };
        if let Ok(endpoint) = transport::lfs_endpoint(&config) {
            config.lfs_ca_info = transport::ca_info(&git, &endpoint)?.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        }
        Ok(config)
    }

    pub fn configure(&self, config: &Config) -> Result<()> {
        validate_settings(config)?;
        let _lock = Store::lock(&self.root)?;
        let path = safe_path(&self.root, SETTINGS)?;
        ensure!(!path.is_dir(), "无法保存 Quiet.md Git 设置");
        let config = Config { name: config.name.trim().into(), email: config.email.trim().into(), ..config.clone() };
        // Only publish the staged app settings after Git's copy was written successfully.
        crate::store::write_atomic_after(&path, &serde_json::to_vec_pretty(&config)?, || self.write_git_config(&config))
            .context("无法保存 Quiet.md Git 设置")
    }

    fn write_git_config(&self, config: &Config) -> Result<()> {
        let mut local = self.repo.config()?.open_level(git2::ConfigLevel::Local)?;
        local.set_str("user.name", &config.name)?;
        local.set_str("user.email", &config.email)?;
        optional_key(&mut local, "quiet.sshKey", &config.ssh_key)?;
        optional_key(&mut local, "lfs.url", &config.lfs_url)?;
        if let Ok(origin) = transport::lfs_origin(config) {
            let origin = origin.ascii_serialization();
            // Git LFS clients may look up either spelling of the origin.
            for slash in ["", "/"] {
                optional_key(&mut local, &format!("http.{origin}{slash}.sslCAInfo"), &config.lfs_ca_info)?;
            }
        }
        if config.remote.is_empty() {
            if self.repo.find_remote("origin").is_ok() { self.repo.remote_delete("origin")?; }
        } else if self.repo.find_remote("origin").is_ok() { self.repo.remote_set_url("origin", &config.remote)?; }
        else { self.repo.remote("origin", &config.remote)?; }
        Ok(())
    }

    pub(super) fn remote(&self, config: &Config) -> Result<git2::Remote<'_>> {
        let git = self.repo.config()?;
        if git.get_string("remote.origin.url").is_ok_and(|url| url == config.remote) { return Ok(self.repo.find_remote("origin")?); }
        Ok(self.repo.remote_anonymous(&config.remote)?)
    }
}

fn read_settings(path: &Path) -> Result<Option<Config>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).context("无法读取 Quiet.md Git 设置")?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("无法读取 Quiet.md Git 设置"),
    }
}

fn validate_settings(config: &Config) -> Result<()> {
    if !config.remote.is_empty() { validate_remote(&config.remote)?; }
    if !config.lfs_url.is_empty() { http_url(&config.lfs_url)?; }
    Ok(())
}

fn optional_key(git: &mut git2::Config, key: &str, value: &str) -> Result<()> {
    if !value.is_empty() { git.set_str(key, value)?; }
    else if let Err(e) = git.remove(key) && e.code() != git2::ErrorCode::NotFound { return Err(e.into()); }
    Ok(())
}
