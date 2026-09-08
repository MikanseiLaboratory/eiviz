//! Still/Video path helpers used when a session is applied and when operators
//! rematch files by name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::session::{Document, InputDto, InputKind};

pub fn media_file_missing(input: &InputDto) -> bool {
    matches!(input.kind, InputKind::Still | InputKind::Video)
        && !path_is_file(input.path_or_address.as_deref())
}

pub fn missing_media_message(input: &InputDto) -> Option<String> {
    if !matches!(input.kind, InputKind::Still | InputKind::Video) {
        return None;
    }
    let Some(path) = input
        .path_or_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Some(format!("input {} is missing a file path", input.id));
    };
    if Path::new(path).is_file() {
        None
    } else {
        Some(format!("input {} file does not exist: {path}", input.id))
    }
}

/// Update Still/Video paths whose current file is missing when the filename
/// appears exactly once under `directories`. Zero or several matches leave the
/// stored path unchanged.
pub fn relink_missing_media(doc: &mut Document, directories: &[String]) -> usize {
    let index = unique_filenames(directories);
    let mut count = 0;
    for input in &mut doc.inputs {
        if !media_file_missing(input) {
            continue;
        }
        let Some(name) = input
            .path_or_address
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let Some(found) = index.get(name) else {
            continue;
        };
        input.path_or_address = Some(found.to_string_lossy().into_owned());
        count += 1;
    }
    count
}

fn path_is_file(path: Option<&str>) -> bool {
    path.map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| Path::new(value).is_file())
}

fn unique_filenames(directories: &[String]) -> HashMap<String, PathBuf> {
    let mut found: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for directory in directories {
        let trimmed = directory.trim();
        if trimmed.is_empty() {
            continue;
        }
        collect_files(Path::new(trimmed), &mut found, 0);
    }
    found
        .into_iter()
        .filter_map(|(name, mut paths)| {
            paths.sort();
            paths.dedup();
            if paths.len() == 1 {
                Some((name, paths.pop().expect("one path")))
            } else {
                None
            }
        })
        .collect()
}

fn collect_files(dir: &Path, out: &mut HashMap<String, Vec<PathBuf>>, depth: u32) {
    if depth > 16 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out, depth + 1);
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        out.entry(name.to_string()).or_default().push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::parse;

    fn still_doc(path: &str) -> Document {
        let mut doc = parse(
            br#"{
          "version": 2,
          "inputs": [{ "id": 2, "name": "Card", "kind": "Still" }],
          "scenes": [{ "id": 1, "name": "Scene 1", "layers": [{ "inputId": 2, "width": 1, "height": 1 }] }],
          "units": [{ "id": 1, "name": "MU 1" }]
        }"#,
        )
        .unwrap();
        doc.inputs[0].path_or_address = Some(path.to_string());
        doc
    }

    #[test]
    fn unique_filename_updates_path() {
        let root = std::env::temp_dir().join(format!("eiviz-relink-{}", std::process::id()));
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("logo.png");
        std::fs::write(&file, b"png").unwrap();
        let missing = root.join("missing").join("logo.png");
        let mut doc = still_doc(&missing.to_string_lossy());
        assert_eq!(
            relink_missing_media(&mut doc, &[root.to_string_lossy().into_owned()]),
            1
        );
        assert_eq!(
            doc.inputs[0].path_or_address.as_deref(),
            Some(file.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ambiguous_filename_keeps_path() {
        let root = std::env::temp_dir().join(format!("eiviz-relink-amb-{}", std::process::id()));
        let one = root.join("one");
        let two = root.join("two");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(one.join("logo.png"), b"a").unwrap();
        std::fs::write(two.join("logo.png"), b"b").unwrap();
        let original = root.join("old").join("logo.png");
        let mut doc = still_doc(&original.to_string_lossy());
        assert_eq!(
            relink_missing_media(&mut doc, &[root.to_string_lossy().into_owned()]),
            0
        );
        assert_eq!(
            doc.inputs[0].path_or_address.as_deref(),
            Some(original.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
