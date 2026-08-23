use eiviz_core::{AssetRef, Project, SCHEMA_VERSION};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::{ZipArchive, ZipWriter};

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("io: {0}")]
    Io(String),
    #[error("schema {0} is newer than supported {SCHEMA_VERSION}")]
    FutureSchema(u32),
    #[error("invalid package: {0}")]
    Package(String),
    #[error(transparent)]
    Domain(#[from] eiviz_core::DomainError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ProjectError>;

pub fn migrate(mut project: Project) -> Result<Project> {
    if project.schema_version > SCHEMA_VERSION {
        return Err(ProjectError::FutureSchema(project.schema_version));
    }
    // v0 (missing) -> v1 is a no-op besides stamping.
    if project.schema_version == 0 {
        project.schema_version = SCHEMA_VERSION;
    }
    project.validate()?;
    Ok(project)
}

pub fn save_atomic(project: &Project, path: &Path) -> Result<()> {
    project.validate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| ProjectError::Io(e.to_string()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(project)?;
    {
        let mut f = File::create(&tmp).map_err(|e| ProjectError::Io(e.to_string()))?;
        f.write_all(&json)
            .map_err(|e| ProjectError::Io(e.to_string()))?;
        f.sync_all().map_err(|e| ProjectError::Io(e.to_string()))?;
    }
    fs::rename(&tmp, path).map_err(|e| ProjectError::Io(e.to_string()))?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Project> {
    let data = fs::read(path).map_err(|e| ProjectError::Io(e.to_string()))?;
    let project: Project = serde_json::from_slice(&data)?;
    migrate(project)
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

pub fn export_portable(project: &Project, dest: &Path, asset_root: &Path) -> Result<()> {
    project.validate()?;
    let file = File::create(dest).map_err(|e| ProjectError::Io(e.to_string()))?;
    let mut zip = ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("project.json", opts)
        .map_err(|e| ProjectError::Package(e.to_string()))?;
    zip.write_all(&serde_json::to_vec_pretty(project)?)
        .map_err(|e| ProjectError::Io(e.to_string()))?;
    for asset in project.assets.values() {
        if asset.missing {
            continue;
        }
        let src = asset_root.join(&asset.relative_path);
        let bytes =
            fs::read(&src).map_err(|e| ProjectError::Io(format!("missing asset {src:?}: {e}")))?;
        let actual = hash_bytes(&bytes);
        if actual != asset.sha256_hex {
            return Err(ProjectError::Package(format!(
                "hash mismatch for {}",
                asset.original_name
            )));
        }
        zip.start_file(format!("assets/{}", asset.sha256_hex), opts)
            .map_err(|e| ProjectError::Package(e.to_string()))?;
        zip.write_all(&bytes)
            .map_err(|e| ProjectError::Io(e.to_string()))?;
    }
    zip.finish()
        .map_err(|e| ProjectError::Package(e.to_string()))?;
    Ok(())
}

pub fn import_portable(package: &Path, dest_dir: &Path) -> Result<Project> {
    fs::create_dir_all(dest_dir).map_err(|e| ProjectError::Io(e.to_string()))?;
    let file = File::open(package).map_err(|e| ProjectError::Io(e.to_string()))?;
    let mut zip = ZipArchive::new(file).map_err(|e| ProjectError::Package(e.to_string()))?;
    let mut project: Option<Project> = None;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| ProjectError::Package(e.to_string()))?;
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| ProjectError::Io(e.to_string()))?;
        if name == "project.json" {
            project = Some(migrate(serde_json::from_slice(&buf)?)?);
        } else if let Some(hash) = name.strip_prefix("assets/") {
            let path = dest_dir.join("assets").join(hash);
            fs::create_dir_all(path.parent().unwrap())
                .map_err(|e| ProjectError::Io(e.to_string()))?;
            fs::write(&path, buf).map_err(|e| ProjectError::Io(e.to_string()))?;
        }
    }
    let mut project = project.ok_or_else(|| ProjectError::Package("no project.json".into()))?;
    for asset in project.assets.values_mut() {
        let p = dest_dir.join("assets").join(&asset.sha256_hex);
        asset.relative_path = format!("assets/{}", asset.sha256_hex);
        asset.missing = !p.exists();
    }
    project.validate()?;
    save_atomic(&project, &dest_dir.join("project.json"))?;
    Ok(project)
}

pub fn ingest_asset(project: &mut Project, file: &Path, dest_root: &Path) -> Result<AssetRef> {
    let bytes = fs::read(file).map_err(|e| ProjectError::Io(e.to_string()))?;
    let hash = hash_bytes(&bytes);
    let rel = format!("assets/{hash}");
    let dest = dest_root.join(&rel);
    fs::create_dir_all(dest.parent().unwrap()).map_err(|e| ProjectError::Io(e.to_string()))?;
    fs::write(&dest, &bytes).map_err(|e| ProjectError::Io(e.to_string()))?;
    let asset = AssetRef {
        id: eiviz_core::AssetId::new(),
        original_name: file
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        sha256_hex: hash,
        relative_path: rel,
        missing: false,
    };
    project.assets.insert(asset.id, asset.clone());
    Ok(asset)
}

pub fn project_dir_name(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eiviz_core::Project;

    #[test]
    fn round_trip_and_reject_future_schema() {
        let dir = std::env::temp_dir().join(format!("eiviz-proj-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = Project::new("alpha");
        let path = dir.join("project.json");
        save_atomic(&p, &path).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.name, "alpha");
        assert_eq!(loaded.id, p.id);

        let mut future = p.clone();
        future.schema_version = 99;
        let future_path = dir.join("future.json");
        fs::write(&future_path, serde_json::to_vec(&future).unwrap()).unwrap();
        match load(&future_path) {
            Err(ProjectError::FutureSchema(99)) => {}
            other => panic!("{other:?}"),
        }

        let pkg = dir.join("pack.eiviz");
        export_portable(&p, &pkg, &dir).unwrap();
        let imported = import_portable(&pkg, &dir.join("imported")).unwrap();
        assert_eq!(imported.id, p.id);
    }
}
