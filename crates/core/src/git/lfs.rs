//! Git LFS pointers, verified object cache and the basic HTTP transfer protocol.
use super::{Auth, Config, transport};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::{Path, PathBuf}};

const VERSION: &str = "version https://git-lfs.github.com/spec/v1\n";
const LIMIT: u64 = crate::store::EXPORT_FILE_LIMIT;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointer { pub oid: String, pub size: u64 }

impl Pointer {
    pub fn of(bytes: &[u8]) -> Self { Self { oid: format!("{:x}", Sha256::digest(bytes)), size: bytes.len() as u64 } }
    pub fn text(&self) -> String { format!("{VERSION}oid sha256:{}\nsize {}\n", self.oid, self.size) }
    pub(super) fn from_parts(oid: &str, size: u64) -> Result<Self> {
        ensure!(oid.len() == 64 && oid.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "无效的 LFS 对象标识");
        ensure!(size <= LIMIT, "LFS 指针格式无效或文件超过 256 MiB");
        Ok(Self { oid: oid.to_string(), size })
    }
    pub fn parse(bytes: &[u8]) -> Result<Option<Self>> {
        if !bytes.starts_with(VERSION.as_bytes()) { return Ok(None) }
        let text = std::str::from_utf8(bytes)?;
        let mut lines = text.lines();
        lines.next();
        let oid = lines.next().and_then(|s| s.strip_prefix("oid sha256:")).unwrap_or("");
        let size: u64 = lines.next().and_then(|s| s.strip_prefix("size ")).context("无效的 LFS 文件大小")?.parse()?;
        ensure!(lines.next().is_none(), "LFS 指针格式无效或文件超过 256 MiB");
        Ok(Some(Self::from_parts(oid, size)?))
    }
    fn verify(&self, bytes: &[u8]) -> Result<()> { ensure!(*self == Self::of(bytes), "LFS 文件大小或 SHA-256 校验失败"); Ok(()) }
}

pub struct Lfs { objects: PathBuf }

impl Lfs {
    pub fn new(git_dir: &Path) -> Self { Self { objects: git_dir.join("lfs/objects") } }
    fn path(&self, pointer: &Pointer) -> PathBuf { self.objects.join(&pointer.oid[..2]).join(&pointer.oid[2..4]).join(&pointer.oid) }
    pub fn store(&self, bytes: &[u8]) -> Result<Pointer> {
        ensure!(bytes.len() as u64 <= LIMIT, "LFS 文件超过 256 MiB");
        let pointer = Pointer::of(bytes);
        let path = self.path(&pointer);
        if self.read(&pointer).is_err() {
            std::fs::create_dir_all(path.parent().unwrap())?;
            crate::store::write_atomic(&path, bytes)?;
        }
        Ok(pointer)
    }
    pub fn read(&self, pointer: &Pointer) -> Result<Vec<u8>> {
        self.read_bounded(pointer, LIMIT)
    }
    pub fn read_bounded(&self, pointer: &Pointer, limit: u64) -> Result<Vec<u8>> {
        ensure!(pointer.size <= limit, "文件超过导出大小限制");
        let bytes = crate::store::read_bounded(&self.path(pointer), limit.min(LIMIT))?;
        pointer.verify(&bytes)?;
        Ok(bytes)
    }

    pub fn transfer(&self, config: &Config, git_config: &git2::Config, pointers: &[Pointer], upload: bool, auth: &Auth) -> Result<()> {
        if pointers.is_empty() { return Ok(()) }
        if let Some(path) = super::local_remote(&config.remote) {
            let repo = git2::Repository::open(path)?;
            let other = Self::new(repo.path());
            for p in pointers {
                if upload { other.store(&self.read(p)?)?; }
                else if self.read(p).is_err() { self.store(&other.read(p)?)?; }
            }
            return Ok(());
        }
        let ssh_auth = transport::lfs_auth(config, auth, upload)?;
        let base = ssh_auth.href.unwrap_or(transport::lfs_endpoint(config)?);
        let batch_url = format!("{}/objects/batch", base.trim_end_matches('/'));
        let agent = transport::http_agent(config, git_config, &batch_url)?;
        for chunk in pointers.chunks(100) {
            let mut request = agent.post(&batch_url).header("Accept", "application/vnd.git-lfs+json");
            if !auth.token.is_empty() && !ssh_auth.header.keys().any(|k| k.eq_ignore_ascii_case("authorization")) {
                let value = STANDARD.encode(format!("{}:{}", auth.username, auth.token));
                request = request.header("Authorization", format!("Basic {value}"));
            }
            for (key, value) in &ssh_auth.header { request = request.header(key, value); }
            let mut response = request.send_json(serde_json::json!({
                "operation": if upload { "upload" } else { "download" }, "transfers": ["basic"],
                "ref": { "name": format!("refs/heads/{}", config.branch) }, "objects": chunk,
            })).context("LFS batch 请求失败")?;
            let batch: Batch = response.body_mut().with_config().limit(4 << 20).read_json()?;
            ensure!(batch.transfer.as_deref().is_none_or(|s| s == "basic"), "LFS 服务器不支持 basic 传输");
            ensure!(batch.objects.len() == chunk.len(), "LFS 服务器返回的对象数量不正确");
            let mut seen = std::collections::HashSet::new();
            for object in batch.objects {
                let pointer = chunk.iter().find(|p| p.oid == object.oid && p.size == object.size).context("LFS 返回了未请求的对象")?;
                ensure!(seen.insert(pointer.oid.clone()), "LFS 返回了重复对象");
                if let Some(error) = object.error { bail!("LFS 对象传输失败（{}）：{}", error.code, error.message) }
                if upload {
                    if let Some(action) = object.actions.get("upload") {
                        let bytes = self.read(pointer)?;
                        let agent = transport::http_agent(config, git_config, &action.href)?;
                        let mut request = agent.put(&action.href).header("Content-Type", "application/octet-stream");
                        for (key, value) in &action.header { request = request.header(key, value); }
                        request.send(&bytes[..]).context("上传 LFS 对象失败")?;
                        if let Some(verify) = object.actions.get("verify") {
                            let agent = transport::http_agent(config, git_config, &verify.href)?;
                            let mut request = agent.post(&verify.href).header("Accept", "application/vnd.git-lfs+json");
                            for (key, value) in &verify.header { request = request.header(key, value); }
                            request.send_json(pointer).context("LFS 上传确认失败")?;
                        }
                    }
                } else if self.read(pointer).is_err() {
                    let action = object.actions.get("download").context("LFS 服务器未提供下载地址")?;
                    let agent = transport::http_agent(config, git_config, &action.href)?;
                    let mut request = agent.get(&action.href);
                    for (key, value) in &action.header { request = request.header(key, value); }
                    let bytes = request.call()?.body_mut().with_config().limit(pointer.size + 1).read_to_vec()?;
                    pointer.verify(&bytes)?;
                    self.store(&bytes)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Batch { transfer: Option<String>, objects: Vec<Object> }
#[derive(Deserialize)]
struct Object {
    oid: String, size: u64,
    #[serde(default)] actions: BTreeMap<String, Action>,
    error: Option<ObjectError>,
}
#[derive(Deserialize)]
struct Action { href: String, #[serde(default)] header: BTreeMap<String, String> }
#[derive(Deserialize)]
struct ObjectError { code: u32, message: String }

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{Read, Write}, net::TcpListener, time::Duration};

    #[test]
    fn basic_http_upload_verify_download_and_authentication() {
        let root = std::env::temp_dir().join(format!("quiet-lfs-http-{}", uuid::Uuid::new_v4()));
        let upload = Lfs::new(&root.join("sender"));
        let pointer = upload.store(b"versioned picture").unwrap();
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.local_addr().unwrap());
        let base_server = base.clone(); let expected = pointer.clone();
        let worker = std::thread::spawn(move || {
            server.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            for step in 0..5 {
                let mut stream = loop {
                    match server.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                        result => panic!("LFS request missing at step {step}: {result:?}"),
                    }
                };
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut data = vec![];
                let split = loop {
                    let mut byte = [0]; stream.read_exact(&mut byte).unwrap(); data.push(byte[0]);
                    if data.ends_with(b"\r\n\r\n") { break data.len(); }
                };
                let header = String::from_utf8_lossy(&data[..split]).to_lowercase();
                let length: usize = header.lines().find_map(|line| line.strip_prefix("content-length:")).unwrap_or("0").trim().parse().unwrap();
                let mut body = vec![0; length]; stream.read_exact(&mut body).unwrap();
                let response = match step {
                    0 | 3 => {
                        assert!(header.starts_with("post /info/lfs/objects/batch "));
                        assert!(header.contains("authorization: basic dXNlcjp0b2tlbg==".to_lowercase().as_str()));
                        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        assert_eq!(request["operation"], if step == 0 { "upload" } else { "download" });
                        let actions = if step == 0 { serde_json::json!({
                            "upload": {"href": format!("{base_server}/object"), "header": {"X-Transfer": "signed-upload"}},
                            "verify": {"href": format!("{base_server}/verify")},
                        }) } else { serde_json::json!({"download": {"href": format!("{base_server}/object")}}) };
                        serde_json::to_vec(&serde_json::json!({"transfer": "basic", "objects": [{"oid": expected.oid, "size": expected.size, "actions": actions}]})).unwrap()
                    },
                    1 => { assert!(header.starts_with("put /object ")); assert!(header.contains("x-transfer: signed-upload")); assert!(!header.contains("authorization:")); assert_eq!(body, b"versioned picture"); vec![] },
                    2 => { assert!(header.starts_with("post /verify ")); let p: Pointer = serde_json::from_slice(&body).unwrap(); assert_eq!(p, expected); vec![] },
                    _ => { assert!(header.starts_with("get /object ")); b"versioned picture".to_vec() },
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
                stream.write_all(&response).unwrap();
            }
        });
        let auth = Auth { username: "user".into(), token: "token".into(), ..Default::default() };
        let endpoint = format!("{base}/info/lfs");
        let config = Config { remote: "https://example.invalid/notes.git".into(), lfs_url: endpoint, branch: "main".into(), ..Default::default() };
        let git_config = git2::Config::new().unwrap();
        upload.transfer(&config, &git_config, std::slice::from_ref(&pointer), true, &auth).unwrap();
        let download = Lfs::new(&root.join("receiver"));
        download.transfer(&config, &git_config, std::slice::from_ref(&pointer), false, &auth).unwrap();
        assert_eq!(download.read(&pointer).unwrap(), b"versioned picture");
        worker.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
