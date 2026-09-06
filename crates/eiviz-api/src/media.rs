//! Host-owned media staging. Upload bytes never enter MixerPort until commit.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eiviz_control::error::{ControlError, ControlResult};
use eiviz_control::session::InputKind;
use sha2::{Digest, Sha256};

pub const DEFAULT_CHUNK_SIZE: u32 = 256 * 1024;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_UPLOADS: usize = 4;
const STALE_AFTER: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone)]
pub struct MediaStorageConfig {
    pub root: PathBuf,
    pub max_file_bytes: u64,
    pub max_uploads: usize,
}

impl MediaStorageConfig {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            max_file_bytes: MAX_FILE_BYTES,
            max_uploads: MAX_UPLOADS,
        }
    }
}

pub trait MediaStorage: Send + Sync {
    fn begin(
        &self,
        file_name: &str,
        kind: InputKind,
        size_bytes: u64,
        sha256_hex: &str,
    ) -> ControlResult<String>;
    fn write_chunk(&self, upload_id: &str, offset: u64, data: &[u8]) -> ControlResult<()>;
    fn commit(&self, upload_id: &str) -> ControlResult<PathBuf>;
    fn abort(&self, upload_id: &str) -> ControlResult<()>;
}

#[derive(Debug)]
struct Upload {
    id: String,
    _kind: InputKind,
    size_bytes: u64,
    sha256_hex: String,
    received: u64,
    path: PathBuf,
    started: Instant,
}

#[derive(Debug, Clone)]
pub struct FileMediaStorage {
    inner: Arc<Mutex<FileInner>>,
}

#[derive(Debug)]
struct FileInner {
    config: MediaStorageConfig,
    uploads: Vec<Upload>,
}

impl FileMediaStorage {
    pub fn new(config: MediaStorageConfig) -> ControlResult<Self> {
        fs::create_dir_all(&config.root).map_err(|error| ControlError::io(error.to_string()))?;
        let staging = config.root.join(".staging");
        fs::create_dir_all(&staging).map_err(|error| ControlError::io(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(FileInner {
                config,
                uploads: Vec::new(),
            })),
        })
    }
}

impl MediaStorage for FileMediaStorage {
    fn begin(
        &self,
        file_name: &str,
        kind: InputKind,
        size_bytes: u64,
        sha256_hex: &str,
    ) -> ControlResult<String> {
        if !matches!(kind, InputKind::Still | InputKind::Video) {
            return Err(ControlError::invalid("media kind must be still or video"));
        }
        let ext = allowed_ext(file_name, kind)?;
        if size_bytes == 0 || size_bytes > MAX_FILE_BYTES {
            return Err(ControlError::invalid("file size out of range"));
        }
        if !is_hex_sha256(sha256_hex) {
            return Err(ControlError::invalid("sha256 required"));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ControlError::internal("media lock"))?;
        inner.sweep();
        if inner.uploads.len() >= inner.config.max_uploads {
            return Err(ControlError::unavailable("too many uploads"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let staging = inner.config.root.join(".staging").join(format!("{id}{ext}"));
        File::create(&staging).map_err(|error| ControlError::io(error.to_string()))?;
        inner.uploads.push(Upload {
            id: id.clone(),
            _kind: kind,
            size_bytes,
            sha256_hex: sha256_hex.to_ascii_lowercase(),
            received: 0,
            path: staging,
            started: Instant::now(),
        });
        Ok(id)
    }

    fn write_chunk(&self, upload_id: &str, offset: u64, data: &[u8]) -> ControlResult<()> {
        if data.is_empty() {
            return Err(ControlError::invalid("empty chunk"));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ControlError::internal("media lock"))?;
        inner.sweep();
        let upload = inner
            .uploads
            .iter_mut()
            .find(|item| item.id == upload_id)
            .ok_or_else(|| ControlError::not_found("upload"))?;
        if offset != upload.received {
            return Err(ControlError::invalid("chunk offset must be sequential"));
        }
        let next = upload
            .received
            .checked_add(data.len() as u64)
            .ok_or_else(|| ControlError::invalid("chunk overflow"))?;
        if next > upload.size_bytes {
            return Err(ControlError::invalid("chunk exceeds declared size"));
        }
        let mut file = OpenOptions::new()
            .write(true)
            .open(&upload.path)
            .map_err(|error| ControlError::io(error.to_string()))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| ControlError::io(error.to_string()))?;
        file.write_all(data)
            .map_err(|error| ControlError::io(error.to_string()))?;
        upload.received = next;
        Ok(())
    }

    fn commit(&self, upload_id: &str) -> ControlResult<PathBuf> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ControlError::internal("media lock"))?;
        inner.sweep();
        let index = inner
            .uploads
            .iter()
            .position(|item| item.id == upload_id)
            .ok_or_else(|| ControlError::not_found("upload"))?;
        let upload = inner.uploads.remove(index);
        if upload.received != upload.size_bytes {
            let _ = fs::remove_file(&upload.path);
            return Err(ControlError::invalid("upload incomplete"));
        }
        let digest = hash_file(&upload.path)?;
        if digest != upload.sha256_hex {
            let _ = fs::remove_file(&upload.path);
            return Err(ControlError::invalid("sha256 mismatch"));
        }
        let ext = upload
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("bin");
        let dest = unique_dest(&inner.config.root, &upload.id, ext)?;
        fs::rename(&upload.path, &dest).map_err(|error| ControlError::io(error.to_string()))?;
        Ok(dest)
    }

    fn abort(&self, upload_id: &str) -> ControlResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ControlError::internal("media lock"))?;
        if let Some(index) = inner.uploads.iter().position(|item| item.id == upload_id) {
            let upload = inner.uploads.remove(index);
            let _ = fs::remove_file(&upload.path);
        }
        Ok(())
    }
}

impl FileInner {
    fn sweep(&mut self) {
        let now = Instant::now();
        self.uploads.retain(|upload| {
            if now.duration_since(upload.started) > STALE_AFTER {
                let _ = fs::remove_file(&upload.path);
                false
            } else {
                true
            }
        });
    }
}

fn allowed_ext(file_name: &str, kind: InputKind) -> ControlResult<String> {
    if file_name.contains("..")
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains('\0')
        || Path::new(file_name).is_absolute()
    {
        return Err(ControlError::invalid("file name rejected"));
    }
    let name = Path::new(file_name)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ControlError::invalid("file name required"))?;
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ok = match kind {
        InputKind::Still => matches!(ext.as_str(), "png" | "jpg" | "jpeg"),
        InputKind::Video => matches!(ext.as_str(), "mp4" | "mov" | "m4v" | "mkv"),
        _ => false,
    };
    if !ok {
        return Err(ControlError::invalid("file extension not allowed"));
    }
    Ok(format!(".{ext}"))
}

fn is_hex_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn hash_file(path: &Path) -> ControlResult<String> {
    let mut file = File::open(path).map_err(|error| ControlError::io(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|error| ControlError::io(error.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn unique_dest(root: &Path, id: &str, ext: &str) -> ControlResult<PathBuf> {
    let dest = root.join(format!("{id}.{ext}"));
    let canon_root = fs::canonicalize(root).map_err(|error| ControlError::io(error.to_string()))?;
    if dest.exists() {
        return Err(ControlError::conflict("media already exists"));
    }
    if let Some(parent) = dest.parent() {
        let canon_parent =
            fs::canonicalize(parent).map_err(|error| ControlError::io(error.to_string()))?;
        if !canon_parent.starts_with(&canon_root) {
            return Err(ControlError::permission("media path escaped root"));
        }
    }
    Ok(dest)
}

pub fn parse_kind(kind: &str) -> ControlResult<InputKind> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "still" | "image" | "png" | "jpg" | "jpeg" => Ok(InputKind::Still),
        "video" | "mp4" | "mov" => Ok(InputKind::Video),
        _ => Err(ControlError::invalid("unknown media kind")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (FileMediaStorage, PathBuf) {
        let root = std::env::temp_dir().join(format!("eiviz-media-{}", uuid::Uuid::new_v4()));
        (
            FileMediaStorage::new(MediaStorageConfig::new(root.clone())).unwrap(),
            root,
        )
    }

    #[test]
    fn rejects_path_traversal_name() {
        let (store, root) = temp_store();
        let err = store
            .begin("../x.png", InputKind::Still, 4, &"a".repeat(64))
            .unwrap_err();
        assert!(matches!(err, ControlError::InvalidArgument { .. }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_requires_hash_and_size() {
        let (store, root) = temp_store();
        let bytes = b"\x89PNG";
        let sha = format!("{:x}", Sha256::digest(bytes));
        let id = store
            .begin("logo.png", InputKind::Still, bytes.len() as u64, &sha)
            .unwrap();
        store.write_chunk(&id, 0, bytes).unwrap();
        let path = store.commit(&id).unwrap();
        assert!(path.starts_with(&root));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hash_mismatch_leaves_no_final_file() {
        let (store, root) = temp_store();
        let id = store
            .begin("logo.png", InputKind::Still, 4, &"b".repeat(64))
            .unwrap();
        store.write_chunk(&id, 0, b"1234").unwrap();
        assert!(store.commit(&id).is_err());
        let files: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_file())
            .collect();
        assert!(files.is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
