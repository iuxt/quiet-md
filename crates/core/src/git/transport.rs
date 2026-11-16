//! SSH discovery for LFS and URL-scoped Git TLS configuration.
use super::{Auth, Config};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{collections::BTreeMap, io::Read, net::{TcpStream, ToSocketAddrs}, path::PathBuf, time::Duration};

pub(super) struct SshRemote { pub host: String, pub port: u16, pub username: String, pub path: String }
impl SshRemote {
    pub fn parse(remote: &str) -> Result<Option<Self>> {
        let url = if remote.starts_with("ssh://") {
            url::Url::parse(remote).context("无效的 SSH 仓库地址")?
        } else if !remote.contains("://") && !std::path::Path::new(remote).is_absolute() {
            let Some((host, path)) = remote.split_once(':') else { return Ok(None) };
            ensure!(!host.is_empty() && !path.is_empty() && !host.contains('/'), "无效的 SSH 仓库地址");
            url::Url::parse(&format!("ssh://{host}/{path}")).context("无效的 SSH 仓库地址")?
        } else { return Ok(None) };
        ensure!(url.password().is_none() && url.query().is_none() && url.fragment().is_none(), "SSH 地址中不能包含密码、查询或片段");
        let host = url.host_str().context("SSH 地址缺少服务器")?.trim_start_matches('[').trim_end_matches(']').to_string();
        let path = percent_encoding::percent_decode_str(url.path().trim_start_matches('/')).decode_utf8()?.into_owned();
        ensure!(!path.is_empty() && !path.chars().any(char::is_control), "SSH 地址缺少有效仓库路径");
        Ok(Some(Self { host, port: url.port().unwrap_or(22), username: if url.username().is_empty() { "git".into() } else { url.username().to_string() }, path }))
    }

    fn authenticate(&self, config: &Config, auth: &Auth, upload: bool) -> Result<Option<LfsAuth>> {
        let timeout = Duration::from_secs(30);
        let addresses: Vec<_> = (self.host.as_str(), self.port).to_socket_addrs()?.collect();
        let socket = addresses.iter().find_map(|address| TcpStream::connect_timeout(address, timeout).ok()).context("无法连接 SSH 服务器")?;
        let mut session = ssh2::Session::new()?;
        session.set_timeout(30_000); session.set_tcp_stream(socket); session.handshake()?;
        let (key, _) = session.host_key().context("SSH 服务器未提供主机密钥")?;
        check_host(&session, &self.host, self.port, key)?;
        if let Some(key) = ssh_key(&config.ssh_key) {
            session.userauth_pubkey_file(&self.username, None, &key, (!auth.ssh_passphrase.is_empty()).then_some(auth.ssh_passphrase.as_str()))
                .context("SSH 私钥认证失败")?;
        } else { session.userauth_agent(&self.username).context("SSH agent 认证失败，请配置 SSH 私钥")?; }
        ensure!(session.authenticated(), "SSH 认证失败");
        let mut channel = session.channel_session()?;
        channel.exec(&self.command(upload))?;
        // Bound both streams and drain them together so stderr cannot block stdout.
        session.set_blocking(false);
        let mut output = vec![]; let mut errors = vec![];
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let mut received = 0;
            for (stream, bytes) in [(&mut channel.stream(0), &mut output), (&mut channel.stderr(), &mut errors)] {
                let mut buffer = [0; 8192];
                match stream.read(&mut buffer) {
                    Ok(n) => { received += n; bytes.extend_from_slice(&buffer[..n]); },
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {},
                    Err(e) => return Err(e.into()),
                }
                ensure!(bytes.len() <= 1 << 20, "LFS SSH 认证响应过大");
            }
            if channel.eof() && received == 0 { break }
            ensure!(std::time::Instant::now() < deadline, "LFS SSH 认证超时");
            std::thread::sleep(Duration::from_millis(5));
        }
        session.set_blocking(true); channel.wait_close()?;
        let status = channel.exit_status()?;
        let errors = String::from_utf8_lossy(&errors);
        if status == 127 || (status != 0 && errors.contains("git-lfs-authenticate") && errors.contains("not found")) { return Ok(None) }
        ensure!(status == 0, "LFS SSH 认证失败（退出码 {status}）");
        Ok(Some(serde_json::from_slice(&output).context("无效的 LFS SSH 认证响应")?))
    }

    fn command(&self, upload: bool) -> String {
        format!("git-lfs-authenticate '{}' {}", self.path.replace('\'', "'\\''"), if upload { "upload" } else { "download" })
    }
}

pub(super) fn check_host(session: &ssh2::Session, host: &str, port: u16, key: &[u8]) -> Result<()> {
    let mut known = session.known_hosts()?;
    let path = std::env::var_os("QUIET_MD_GIT_KNOWN_HOSTS").map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".ssh/known_hosts")));
    if let Some(path) = path && path.exists() { known.read_file(&path, ssh2::KnownHostFileKind::OpenSSH)?; }
    ensure!(matches!(known.check_port(host, port, key), ssh2::CheckResult::Match),
        "SSH 主机密钥未知或不匹配，请先核对服务器并配置 known_hosts");
    Ok(())
}

pub(super) fn ssh_key(configured: &str) -> Option<PathBuf> {
    let key = if configured.is_empty() { std::env::var("QUIET_MD_GIT_SSH_KEY").unwrap_or_default() } else { configured.to_string() };
    if key.is_empty() { return None }
    Some(if let Some(path) = key.strip_prefix("~/") { dirs::home_dir().unwrap_or_default().join(path) } else { PathBuf::from(key) })
}

#[derive(Default, Deserialize)]
pub(super) struct LfsAuth { pub href: Option<String>, #[serde(default)] pub header: BTreeMap<String, String> }

pub(super) fn lfs_endpoint(config: &Config) -> Result<String> {
    if !config.lfs_url.is_empty() { return Ok(config.lfs_url.clone()) }
    let remote = if let Some(ssh) = SshRemote::parse(&config.remote)? {
        let mut url = url::Url::parse("https://localhost")?;
        url.set_host(Some(&ssh.host))?; url.set_path(&ssh.path);
        url.to_string()
    } else { super::http_url(&config.remote)?; config.remote.clone() };
    Ok(format!("{}.git/info/lfs", remote.trim_end_matches('/').trim_end_matches(".git")))
}

pub(super) fn lfs_auth(config: &Config, auth: &Auth, upload: bool) -> Result<LfsAuth> {
    if config.lfs_url.is_empty() && let Some(ssh) = SshRemote::parse(&config.remote)? {
        return Ok(ssh.authenticate(config, auth, upload)?.unwrap_or_default());
    }
    Ok(LfsAuth::default())
}

pub(super) fn ca_info(config: &git2::Config, target: &str) -> Result<Option<PathBuf>> {
    ca_info_key(config, target)?.map(|name| config.get_path(&name).map_err(Into::into)).transpose()
}

pub(super) fn ca_info_key(config: &git2::Config, target: &str) -> Result<Option<String>> {
    let target = url::Url::parse(target)?;
    let mut best = None;
    let mut entries = config.entries(None)?;
    while let Some(entry) = entries.next() {
        let entry = entry?;
        let name = entry.name()?;
        let score = if name == "http.sslcainfo" { Some((0, 0, 0)) }
        else { name.strip_prefix("http.").and_then(|n| n.strip_suffix(".sslcainfo")).and_then(|scope| ca_match(scope, &target)) };
        if let Some(score) = score && best.as_ref().is_none_or(|(previous, _)| score >= *previous) {
            best = Some((score, name.to_string()));
        }
    }
    Ok(best.map(|(_, name)| name))
}

fn ca_match(scope: &str, target: &url::Url) -> Option<(usize, usize, usize)> {
    let scope = url::Url::parse(scope).ok()?;
    if scope.scheme() != target.scheme() || scope.port_or_known_default() != target.port_or_known_default() { return None }
    let host = scope.host_str()?; let target_host = target.host_str()?;
    let parts: Vec<_> = host.split('.').collect(); let other: Vec<_> = target_host.split('.').collect();
    if parts.len() != other.len() || !parts.iter().zip(other).all(|(a, b)| *a == "*" || *a == b) { return None }
    if !scope.username().is_empty() && scope.username() != target.username() { return None }
    let path = scope.path().trim_end_matches('/');
    if target.path() != path && !target.path().strip_prefix(path).is_some_and(|p| p.starts_with('/')) { return None }
    Some((host.len() - host.matches('*').count(), path.len(), usize::from(!scope.username().is_empty())))
}

pub(super) fn http_agent(config: &git2::Config, target: &str) -> Result<ureq::Agent> {
    super::http_url(target)?;
    let mut builder = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).max_redirects(0);
    if let Some(path) = ca_info(config, target)? {
        let pem = crate::store::read_bounded(&path, 4 << 20).with_context(|| format!("无法读取 LFS CA 证书：{}", path.display()))?;
        let mut certificates = vec![];
        for item in ureq::tls::parse_pem(&pem) {
            if let ureq::tls::PemItem::Certificate(cert) = item.context("无效的 LFS CA 证书")? { certificates.push(cert); }
        }
        ensure!(!certificates.is_empty(), "LFS CA 文件中没有 PEM 证书");
        builder = builder.tls_config(ureq::tls::TlsConfig::builder().root_certs(certificates.into()).build());
    }
    Ok(builder.build().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_urls_ports_scp_paths_and_lfs_discovery() {
        for remote in ["ssh://git@git.example.invalid:2222/team/notes.git", "git@git.example.invalid:team/notes.git"] {
            super::super::validate_remote(remote).unwrap();
            let ssh = SshRemote::parse(remote).unwrap().unwrap();
            assert_eq!(ssh.host, "git.example.invalid"); assert_eq!(ssh.username, "git"); assert_eq!(ssh.path, "team/notes.git");
            assert_eq!(ssh.port, if remote.starts_with("ssh://") { 2222 } else { 22 });
            let config = Config { remote: remote.into(), ..Default::default() };
            assert_eq!(lfs_endpoint(&config).unwrap(), "https://git.example.invalid/team/notes.git/info/lfs");
            assert_eq!(ssh.command(true), "git-lfs-authenticate 'team/notes.git' upload");
        }
        for remote in ["ssh://git:secret@host/notes", "ssh://host/", "ssh://host/notes?secret=token", "ssh://host/notes%0Aevil"] {
            assert!(SshRemote::parse(remote).is_err(), "{remote}");
        }
        let ssh = SshRemote::parse("ssh://git@host/team/it%27s.git").unwrap().unwrap();
        assert_eq!(ssh.command(false), "git-lfs-authenticate 'team/it'\\''s.git' download");
        assert!(SshRemote::parse("https://example.invalid/repo.git").unwrap().is_none());
    }

    #[test]
    fn ca_configuration_matches_each_url_and_uses_most_specific_scope() {
        let root = std::env::temp_dir().join(format!("quiet-ca-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap(); let path = root.join("config"); std::fs::write(&path, "").unwrap();
        let mut config = git2::Config::open(&path).unwrap();
        for (scope, ca) in [("http", "default.crt"), ("http.https://git.example.invalid", "host.crt"),
            ("http.https://git.example.invalid/team", "team.crt"), ("http.https://git.example.invalid:8443", "port.crt")] {
            config.set_str(&format!("{scope}.sslCAInfo"), ca).unwrap();
        }
        for (url, ca) in [("https://git.example.invalid/team/repo/info/lfs", "team.crt"),
            ("https://git.example.invalid/teammate/object", "host.crt"), ("https://git.example.invalid:8443/object", "port.crt"),
            ("https://storage.example.invalid/object", "default.crt")] {
            assert_eq!(ca_info(&config, url).unwrap().unwrap(), PathBuf::from(ca));
        }
        config.remove("http.sslCAInfo").unwrap();
        assert!(ca_info(&config, "https://storage.example.invalid/object").unwrap().is_none());
        let missing = root.join("missing.crt"); config.set_str("http.sslCAInfo", missing.to_str().unwrap()).unwrap();
        assert!(http_agent(&config, "https://git.example.invalid/team/object").is_err());
        std::fs::write(root.join("invalid.crt"), "not a PEM certificate").unwrap();
        config.set_str("http.https://git.example.invalid/team.sslCAInfo", root.join("invalid.crt").to_str().unwrap()).unwrap();
        assert!(http_agent(&config, "https://git.example.invalid/team/object").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
