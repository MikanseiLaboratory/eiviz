use eiviz_core::{AssetId, AssetRef, MissingMediaPolicy, Project, SCHEMA_VERSION};
use serde::Serialize;
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
            kind: eiviz_core::OutputKind::Rtmp {
                url: "rtmp://127.0.0.1/live/key".into(),
            },
            enabled: false,
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
