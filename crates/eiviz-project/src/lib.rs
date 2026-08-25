use eiviz_core::{AssetId, AssetRef, MissingMediaPolicy, Project, SCHEMA_VERSION};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Component;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetDiagnosticKind {
    AssetRootUnavailable,
    Missing,
    HashMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AssetDiagnostic {
    pub asset_id: AssetId,
    pub original_name: String,
    pub path: String,
    pub expected_sha256: String,
    pub actual_sha256: Option<String>,
    pub policy: MissingMediaPolicy,
    pub kind: AssetDiagnosticKind,
}

/// Resolve and hash every persisted asset reference. A failure is reported
/// against the configured missing-media policy; no alternate path is searched.
pub fn inspect_assets(project: &Project, asset_root: Option<&Path>) -> Vec<AssetDiagnostic> {
    project
        .assets
        .values()
        .filter_map(|asset| {
            let Some(root) = asset_root else {
                return Some(AssetDiagnostic {
                    asset_id: asset.id,
                    original_name: asset.original_name.clone(),
                    path: format!("<asset-root>/{}", asset.relative_path),
                    expected_sha256: asset.sha256_hex.clone(),
                    actual_sha256: None,
                    policy: project.missing_media,
                    kind: AssetDiagnosticKind::AssetRootUnavailable,
                });
            };
            let path = root.join(&asset.relative_path);
            let path_display = path.display().to_string();
            match fs::read(&path) {
                Ok(bytes) => {
                    let actual = hash_bytes(&bytes);
                    (actual != asset.sha256_hex).then(|| AssetDiagnostic {
                        asset_id: asset.id,
                        original_name: asset.original_name.clone(),
                        path: path_display,
                        expected_sha256: asset.sha256_hex.clone(),
                        actual_sha256: Some(actual),
                        policy: project.missing_media,
                        kind: AssetDiagnosticKind::HashMismatch,
                    })
                }
                Err(_) => Some(AssetDiagnostic {
                    asset_id: asset.id,
                    original_name: asset.original_name.clone(),
                    path: path_display,
                    expected_sha256: asset.sha256_hex.clone(),
                    actual_sha256: None,
                    policy: project.missing_media,
                    kind: AssetDiagnosticKind::Missing,
                }),
            }
        })
        .collect()
}

/// Mark missing or hash-mismatched assets so Runtime applies only the explicit
/// project policy. This never changes a path or substitutes another file.
pub fn reconcile_assets(project: &mut Project, asset_root: Option<&Path>) -> Vec<AssetDiagnostic> {
    let diagnostics = inspect_assets(project, asset_root);
    for asset in project.assets.values_mut() {
        asset.missing = diagnostics
            .iter()
            .any(|diagnostic| diagnostic.asset_id == asset.id);
    }
    diagnostics
}

pub fn migrate(mut project: Project) -> Result<Project> {
    if project.schema_version > SCHEMA_VERSION {
        return Err(ProjectError::FutureSchema(project.schema_version));
    }
    // Distribution profiles became mandatory in v2. Never invent a codec or
    // transport profile for a legacy streaming output.
    if project.schema_version < 2
        && let Some(output) = project.outputs.values().find(|output| {
            matches!(
                output.kind,
                eiviz_core::OutputKind::Rtmp { .. }
                    | eiviz_core::OutputKind::Srt { .. }
                    | eiviz_core::OutputKind::Mp4 { .. }
            ) && output.distribution.is_none()
        })
    {
        return Err(ProjectError::Package(format!(
            "legacy distribution output {} requires an explicit codec and transport profile",
            output.id
        )));
    }
    // v3 persists the audio resampling policy. Serde defaults every older
    // project to ExactRate; migration never opts a project into ASRC.
    // v4 persists auxiliary load shedding. Its serde default is Disabled, so
    // migration never silently authorizes Preview/Multiview degradation.
    // v5 adds field order and color-conversion policy. Serde defaults preserve
    // progressive scan and Exact color handling; migration never enables a
    // conversion, tone map, or extended profile.
    // v6 makes each Output's video source explicit. Older outputs retain the
    // owning Mixing Unit's Program feed; migration never selects a Multiview.
    // v7 persists per-output color format. Missing values keep the historical
    // kind default and never switch a window output onto CPU pixels.
    if project.schema_version < 7 {
        for output in project.outputs.values_mut() {
            if output.color_format.is_none() {
                output.color_format = eiviz_core::OutputColorFormat::legacy_for(&output.kind);
            }
        }
    }
    project.schema_version = SCHEMA_VERSION;
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
    let write_result = (|| -> Result<()> {
        let mut f = File::create(&tmp).map_err(|e| ProjectError::Io(e.to_string()))?;
        f.write_all(&json)
            .map_err(|e| ProjectError::Io(e.to_string()))?;
        f.sync_all().map_err(|e| ProjectError::Io(e.to_string()))?;
        fs::rename(&tmp, path).map_err(|e| ProjectError::Io(e.to_string()))?;
        if let Some(parent) = path.parent() {
            sync_directory(parent).map_err(|error| ProjectError::Io(error.to_string()))?;
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result?;
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
        let relative = safe_relative_path(&asset.relative_path).ok_or_else(|| {
            ProjectError::Package(format!(
                "asset {} has unsafe relative path {:?}",
                asset.original_name, asset.relative_path
            ))
        })?;
        let src = asset_root.join(relative);
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
    import_portable_with_writer(package, dest_dir, |path, bytes| fs::write(path, bytes))
}

fn import_portable_with_writer(
    package: &Path,
    dest_dir: &Path,
    mut write_asset: impl FnMut(&Path, &[u8]) -> std::io::Result<()>,
) -> Result<Project> {
    let file = File::open(package).map_err(|e| ProjectError::Io(e.to_string()))?;
    let mut zip = ZipArchive::new(file).map_err(|e| ProjectError::Package(e.to_string()))?;
    let mut project: Option<Project> = None;
    let mut assets = BTreeMap::<String, Vec<u8>>::new();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| ProjectError::Package(e.to_string()))?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            return Err(ProjectError::Package(format!(
                "unexpected directory entry {name:?}"
            )));
        }
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| ProjectError::Io(e.to_string()))?;
        if name == "project.json" {
            if project.is_some() {
                return Err(ProjectError::Package("duplicate project.json entry".into()));
            }
            project = Some(migrate(serde_json::from_slice(&buf)?)?);
        } else if let Some(hash) = portable_asset_hash(&name) {
            if hash_bytes(&buf) != hash {
                return Err(ProjectError::Package(format!(
                    "portable asset {hash} content hash mismatch"
                )));
            }
            if assets.insert(hash.to_owned(), buf).is_some() {
                return Err(ProjectError::Package(format!(
                    "duplicate portable asset {hash}"
                )));
            }
        } else {
            return Err(ProjectError::Package(format!(
                "unsafe or unexpected package entry {name:?}"
            )));
        }
    }
    let mut project = project.ok_or_else(|| ProjectError::Package("no project.json".into()))?;
    let expected = project
        .assets
        .values()
        .filter(|asset| !asset.missing)
        .map(|asset| asset.sha256_hex.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(hash) = expected
        .iter()
        .find(|hash| !assets.contains_key(**hash))
        .copied()
    {
        return Err(ProjectError::Package(format!(
            "portable package is missing asset {hash}"
        )));
    }
    if let Some(hash) = assets.keys().find(|hash| !expected.contains(hash.as_str())) {
        return Err(ProjectError::Package(format!(
            "portable package contains unreferenced asset {hash}"
        )));
    }
    fs::create_dir_all(dest_dir).map_err(|e| ProjectError::Io(e.to_string()))?;
    let asset_dir = dest_dir.join("assets");
    fs::create_dir_all(&asset_dir).map_err(|e| ProjectError::Io(e.to_string()))?;
    for (hash, bytes) in &assets {
        write_asset(&asset_dir.join(hash), bytes).map_err(|e| ProjectError::Io(e.to_string()))?;
    }
    for asset in project.assets.values_mut() {
        asset.relative_path = format!("assets/{}", asset.sha256_hex);
        asset.missing = !assets.contains_key(&asset.sha256_hex);
    }
    project.validate()?;
    save_atomic(&project, &dest_dir.join("project.json"))?;
    Ok(project)
}

fn portable_asset_hash(name: &str) -> Option<&str> {
    let hash = name.strip_prefix("assets/")?;
    (hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(hash)
}

fn safe_relative_path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

pub fn ingest_asset(project: &mut Project, file: &Path, dest_root: &Path) -> Result<AssetRef> {
    let asset = stage_asset(file, dest_root)?;
    project.assets.insert(asset.id, asset.clone());
    Ok(asset)
}

/// Copy an asset into the content-addressed store without mutating Project.
/// The returned reference must be committed through the Command sequencer.
pub fn stage_asset(file: &Path, dest_root: &Path) -> Result<AssetRef> {
    let bytes = fs::read(file).map_err(|e| ProjectError::Io(e.to_string()))?;
    let hash = hash_bytes(&bytes);
    let rel = format!("assets/{hash}");
    let dest = dest_root.join(&rel);
    fs::create_dir_all(dest.parent().unwrap()).map_err(|e| ProjectError::Io(e.to_string()))?;
    fs::write(&dest, &bytes).map_err(|e| ProjectError::Io(e.to_string()))?;
    Ok(AssetRef {
        id: eiviz_core::AssetId::new(),
        original_name: file
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        sha256_hex: hash,
        relative_path: rel,
        missing: false,
    })
}

pub fn autosave_path(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    p.set_extension("autosave.json");
    p
}

pub fn save_autosave(project: &Project, path: &Path) -> Result<()> {
    save_atomic(project, &autosave_path(path))
}

pub fn recover_autosave(path: &Path) -> Result<Option<Project>> {
    let auto = autosave_path(path);
    if auto.exists() {
        Ok(Some(load(&auto)?))
    } else {
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub struct AutosaveCandidate {
    pub path: PathBuf,
    pub project: Project,
    pub project_hash: String,
    pub newer_than_project: bool,
}

#[derive(Clone, Debug)]
pub enum AutosaveInspection {
    Missing,
    Current,
    Recoverable(Box<AutosaveCandidate>),
    Corrupt { path: PathBuf, error: String },
}

/// Inspect startup autosave state without mutating or replacing the project.
pub fn inspect_autosave(path: &Path) -> AutosaveInspection {
    let autosave = autosave_path(path);
    if !autosave.exists() {
        return AutosaveInspection::Missing;
    }
    let project = match load(&autosave) {
        Ok(project) => project,
        Err(error) => {
            return AutosaveInspection::Corrupt {
                path: autosave,
                error: error.to_string(),
            };
        }
    };
    let project_hash = serde_json::to_vec(&project)
        .map(|bytes| hash_bytes(&bytes))
        .unwrap_or_default();
    if let Ok(saved) = load(path)
        && serde_json::to_vec(&saved)
            .map(|bytes| hash_bytes(&bytes))
            .is_ok_and(|hash| hash == project_hash)
    {
        return AutosaveInspection::Current;
    }
    let newer_than_project = fs::metadata(&autosave)
        .and_then(|metadata| metadata.modified())
        .ok()
        .zip(
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok(),
        )
        .is_none_or(|(autosave_modified, project_modified)| autosave_modified >= project_modified);
    AutosaveInspection::Recoverable(Box::new(AutosaveCandidate {
        path: autosave,
        project,
        project_hash,
        newer_than_project,
    }))
}

pub fn discard_autosave(path: &Path) -> Result<()> {
    let autosave = autosave_path(path);
    match fs::remove_file(&autosave) {
        Ok(()) => {
            if let Some(parent) = autosave.parent() {
                sync_directory(parent).map_err(|error| ProjectError::Io(error.to_string()))?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProjectError::Io(error.to_string())),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

pub fn append_journal(path: &Path, revision: u64, hash: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| ProjectError::Io(e.to_string()))?;
    }
    let line = format!("{revision} {hash}\n");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| ProjectError::Io(e.to_string()))?;
    f.write_all(line.as_bytes())
        .map_err(|e| ProjectError::Io(e.to_string()))?;
    Ok(())
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

        save_autosave(&p, &path).unwrap();
        let recovered = recover_autosave(&path).unwrap().unwrap();
        assert_eq!(recovered.id, p.id);
        let journal = dir.join("journal.jsonl");
        append_journal(&journal, 1, "abc").unwrap();
        let j = fs::read_to_string(&journal).unwrap();
        assert!(j.contains("abc"));
    }

    #[test]
    fn v1_migration_never_invents_distribution_profiles() {
        let mut plain = Project::new("legacy");
        plain.schema_version = 1;
        assert_eq!(migrate(plain).unwrap().schema_version, SCHEMA_VERSION);

        let mut distribution = Project::new("legacy distribution");
        distribution.schema_version = 1;
        let owner = *distribution.mixing_units.keys().next().unwrap();
        let output = eiviz_core::Output {
            id: eiviz_core::OutputId::new(),
            name: "legacy RTMP".into(),
            owner,
            video_source: eiviz_core::OutputVideoSource::Program,
            kind: eiviz_core::OutputKind::Rtmp {
                url: "rtmp://127.0.0.1/live/key".into(),
            },
            enabled: false,
            color_format: None,
            distribution: None,
        };
        distribution.outputs.insert(output.id, output.clone());
        distribution
            .mixing_units
            .get_mut(&owner)
            .unwrap()
            .outputs
            .push(output.id);
        let error = migrate(distribution).unwrap_err();
        assert!(error.to_string().contains("requires an explicit codec"));
    }

    #[test]
    fn v2_migration_selects_exact_rate_without_inventing_asrc() {
        let project = Project::new("legacy audio");
        let mut json = serde_json::to_value(project).unwrap();
        json["schema_version"] = serde_json::json!(2);
        json["audio"].as_object_mut().unwrap().remove("resampling");
        let loaded: Project = serde_json::from_value(json).unwrap();
        let migrated = migrate(loaded).unwrap();
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
        assert_eq!(
            migrated.audio.resampling,
            eiviz_core::AudioResamplingPolicy::ExactRate
        );
    }

    #[test]
    fn v3_migration_disables_auxiliary_load_shedding() {
        let project = Project::new("legacy shedding");
        let mut json = serde_json::to_value(project).unwrap();
        json["schema_version"] = serde_json::json!(3);
        json.as_object_mut()
            .unwrap()
            .remove("auxiliary_load_shedding");
        let loaded: Project = serde_json::from_value(json).unwrap();
        let migrated = migrate(loaded).unwrap();
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
        assert_eq!(
            migrated.auxiliary_load_shedding,
            eiviz_core::AuxiliaryLoadSheddingPolicy::Disabled
        );
    }

    #[test]
    fn v4_migration_preserves_progressive_exact_baseline() {
        let project = Project::new("legacy video");
        let mut json = serde_json::to_value(project).unwrap();
        json["schema_version"] = serde_json::json!(4);
        json["video"].as_object_mut().unwrap().remove("field_order");
        json["video"]
            .as_object_mut()
            .unwrap()
            .remove("color_conversion");
        let loaded: Project = serde_json::from_value(json).unwrap();
        let migrated = migrate(loaded).unwrap();
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
        assert!(migrated.video.is_baseline_1080p5994());
    }

    #[test]
    fn v6_migration_fills_legacy_output_color_formats() {
        let mut project = Project::new("legacy color");
        let owner = *project.mixing_units.keys().next().unwrap();
        let ndi = eiviz_core::Output {
            id: eiviz_core::OutputId::new(),
            name: "ndi".into(),
            owner,
            video_source: eiviz_core::OutputVideoSource::Program,
            kind: eiviz_core::OutputKind::Ndi {
                name: "legacy".into(),
            },
            enabled: true,
            color_format: None,
            distribution: None,
        };
        project.outputs.insert(ndi.id, ndi.clone());
        project
            .mixing_units
            .get_mut(&owner)
            .unwrap()
            .outputs
            .push(ndi.id);
        let mut json = serde_json::to_value(&project).unwrap();
        json["schema_version"] = serde_json::json!(6);
        json["outputs"][ndi.id.to_string()]
            .as_object_mut()
            .unwrap()
            .remove("color_format");
        let loaded: Project = serde_json::from_value(json).unwrap();
        let migrated = migrate(loaded).unwrap();
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);
        assert_eq!(
            migrated.outputs[&ndi.id].color_format,
            Some(eiviz_core::OutputColorFormat::Rgba8)
        );
    }

    #[test]
    fn corrupt_autosave_is_reported_without_replacing_saved_project() {
        let root =
            std::env::temp_dir().join(format!("eiviz-corrupt-autosave-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("project.json");
        let saved = Project::new("saved");
        save_atomic(&saved, &path).unwrap();
        fs::write(autosave_path(&path), b"{not valid json").unwrap();

        match inspect_autosave(&path) {
            AutosaveInspection::Corrupt {
                path: corrupt,
                error,
            } => {
                assert_eq!(corrupt, autosave_path(&path));
                assert!(!error.is_empty());
            }
            other => panic!("expected corrupt autosave, got {other:?}"),
        }
        assert_eq!(load(&path).unwrap().id, saved.id);
        discard_autosave(&path).unwrap();
        assert!(matches!(
            inspect_autosave(&path),
            AutosaveInspection::Missing
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn atomic_write_error_keeps_destination_and_cleans_temporary_file() {
        let root =
            std::env::temp_dir().join(format!("eiviz-project-write-error-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let error = save_atomic(&Project::new("cannot replace directory"), &root).unwrap_err();
        assert!(matches!(error, ProjectError::Io(_)));
        assert!(root.is_dir());
        assert!(!root.with_extension("json.tmp").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_corruption_and_truncation_are_deterministic_and_never_panic() {
        let root = std::env::temp_dir().join(format!("eiviz-portable-fuzz-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let valid = root.join("valid.eiviz");
        export_portable(&Project::new("fuzz seed"), &valid, &root).unwrap();
        let seed = fs::read(&valid).unwrap();

        for (case, end) in [0, 1, seed.len() / 4, seed.len() / 2, seed.len() - 1]
            .into_iter()
            .enumerate()
        {
            let path = root.join(format!("truncate-{case}.eiviz"));
            fs::write(&path, &seed[..end]).unwrap();
            let result = std::panic::catch_unwind(|| {
                import_portable(&path, &root.join(format!("truncate-{case}")))
            });
            assert!(result.is_ok(), "truncation case {case} panicked");
            assert!(
                result.unwrap().is_err(),
                "truncation case {case} was accepted"
            );
        }

        for offset in (0..seed.len()).step_by(97) {
            let mut mutated = seed.clone();
            mutated[offset] ^= 0xa5;
            let path = root.join(format!("mutate-{offset}.eiviz"));
            fs::write(&path, mutated).unwrap();
            assert!(
                std::panic::catch_unwind(|| {
                    let _ = import_portable(&path, &root.join(format!("mutate-{offset}")));
                })
                .is_ok(),
                "mutation at byte {offset} panicked"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_import_rejects_path_traversal_and_hash_spoofing() {
        let root =
            std::env::temp_dir().join(format!("eiviz-portable-traversal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let package = root.join("traversal.eiviz");
        let file = File::create(&package).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("project.json", options).unwrap();
        zip.write_all(&serde_json::to_vec(&Project::new("safe")).unwrap())
            .unwrap();
        zip.start_file("assets/../../escaped", options).unwrap();
        zip.write_all(b"owned").unwrap();
        zip.finish().unwrap();

        let error = import_portable(&package, &root.join("dest")).unwrap_err();
        assert!(error.to_string().contains("unsafe or unexpected"));
        assert!(!root.join("escaped").exists());

        let mut project = Project::new("unsafe export");
        let asset = AssetRef {
            id: eiviz_core::AssetId::new(),
            original_name: "escape".into(),
            sha256_hex: hash_bytes(b"owned"),
            relative_path: "../escaped".into(),
            missing: false,
        };
        project.assets.insert(asset.id, asset);
        fs::write(root.join("escaped"), b"owned").unwrap();
        assert!(
            export_portable(&project, &root.join("unsafe.eiviz"), &root)
                .unwrap_err()
                .to_string()
                .contains("unsafe relative path")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_import_propagates_injected_disk_write_failure_without_manifest() {
        let root =
            std::env::temp_dir().join(format!("eiviz-portable-disk-full-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("assets")).unwrap();
        let bytes = b"asset bytes";
        let hash = hash_bytes(bytes);
        fs::write(root.join("assets/source"), bytes).unwrap();
        let mut project = Project::new("disk failure");
        let asset = AssetRef {
            id: eiviz_core::AssetId::new(),
            original_name: "source".into(),
            sha256_hex: hash,
            relative_path: "assets/source".into(),
            missing: false,
        };
        project.assets.insert(asset.id, asset);
        let package = root.join("portable.eiviz");
        export_portable(&project, &package, &root).unwrap();
        let destination = root.join("destination");

        let error = import_portable_with_writer(&package, &destination, |_path, _bytes| {
            Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "injected disk full",
            ))
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected disk full"));
        assert!(!destination.join("project.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn asset_diagnostics_include_exact_path_hash_and_policy_without_substitution() {
        let root =
            std::env::temp_dir().join(format!("eiviz-asset-diagnostics-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("assets")).unwrap();
        let mut project = Project::new("diagnostics");
        project.missing_media = MissingMediaPolicy::Fail;
        let asset = AssetRef {
            id: eiviz_core::AssetId::new(),
            original_name: "clip.mp4".into(),
            sha256_hex: hash_bytes(b"expected"),
            relative_path: "assets/exact-path".into(),
            missing: false,
        };
        project.assets.insert(asset.id, asset.clone());
        fs::write(root.join(&asset.relative_path), b"different").unwrap();

        let diagnostics = reconcile_assets(&mut project, Some(&root));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, AssetDiagnosticKind::HashMismatch);
        assert_eq!(diagnostics[0].policy, MissingMediaPolicy::Fail);
        assert_eq!(diagnostics[0].expected_sha256, hash_bytes(b"expected"));
        assert_eq!(
            diagnostics[0].actual_sha256.as_deref(),
            Some(hash_bytes(b"different").as_str())
        );
        assert_eq!(
            diagnostics[0].path,
            root.join("assets/exact-path").display().to_string()
        );
        assert!(project.assets[&asset.id].missing);
        assert!(!root.join("clip.mp4").exists());
        let _ = fs::remove_dir_all(root);
    }
}
