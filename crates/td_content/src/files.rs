//! A pack's files, read from a directory or a zip, keyed by `/` paths.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

#[derive(Debug, Clone, Default)]
pub struct PackFiles {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl PackFiles {
    pub fn from_zip(bytes: &[u8]) -> Result<PackFiles, String> {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("bad zip: {e}"))?;
        let mut files = BTreeMap::new();
        for i in 0..archive.len() {
            let mut f = archive
                .by_index(i)
                .map_err(|e| format!("bad zip entry: {e}"))?;
            if f.is_dir() {
                continue;
            }
            let name = f
                .name()
                .map_err(|e| format!("bad zip entry name: {e}"))?
                .trim_start_matches("./")
                .to_string();
            let mut data = Vec::new();
            f.read_to_end(&mut data)
                .map_err(|e| format!("{name}: {e}"))?;
            files.insert(name, data);
        }
        // A zip of a directory has everything under one folder: strip it.
        if !files.contains_key("info.toml")
            && let Some(prefix) = files
                .keys()
                .find(|k| k.ends_with("/info.toml"))
                .map(|k| k.trim_end_matches("info.toml").to_string())
        {
            files = files
                .into_iter()
                .filter_map(|(k, v)| k.strip_prefix(&prefix).map(|k| (k.to_string(), v)))
                .collect();
        }
        Ok(PackFiles { files })
    }

    #[cfg(feature = "fs")]
    pub fn from_dir(dir: &std::path::Path) -> Result<PackFiles, String> {
        fn walk(
            root: &std::path::Path,
            dir: &std::path::Path,
            out: &mut BTreeMap<String, Vec<u8>>,
        ) -> Result<(), String> {
            let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, out)?;
                } else {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    let data =
                        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                    out.insert(rel, data);
                }
            }
            Ok(())
        }
        let mut files = BTreeMap::new();
        walk(dir, dir, &mut files)?;
        Ok(PackFiles { files })
    }

    /// A directory or a `.zip`.
    #[cfg(feature = "fs")]
    pub fn open(path: &std::path::Path) -> Result<PackFiles, String> {
        if path.is_dir() {
            Self::from_dir(path)
        } else {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Self::from_zip(&bytes)
        }
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn text(&self, path: &str) -> Result<&str, String> {
        let bytes = self.get(path).ok_or_else(|| format!("missing {path}"))?;
        std::str::from_utf8(bytes).map_err(|_| format!("{path}: not UTF-8"))
    }

    /// Files under `dir/` with extension `ext`, sorted.
    pub fn list<'a>(
        &'a self,
        dir: &'a str,
        ext: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a [u8])> + 'a {
        self.files.iter().filter_map(move |(k, v)| {
            let rest = k.strip_prefix(dir)?.strip_prefix('/')?;
            (!rest.contains('/') && rest.ends_with(ext)).then_some((k.as_str(), v.as_slice()))
        })
    }
}

/// `maps/crossroads.toml` → `crossroads`.
pub fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}
