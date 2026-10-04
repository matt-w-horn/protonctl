//! The export folder, set by `[export] folder` in the config. It is RFC R10's
//! one exception: tools write there only what a call asks for (a Drive file,
//! an attachment, a manifest), only at paths protonctl chooses, and protonctl
//! never cleans it. A folder connected to Cowork gives agents there reach to
//! these files, which they cannot get to in the private download folder.

use std::os::unix::fs::DirBuilderExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{ExportConfig, cache_dir};

#[derive(Debug)]
pub struct Export {
    folder: PathBuf,
}

/// `p` with `.` and `..` resolved by text, for a folder that may not exist yet.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

impl Export {
    /// The configured folder, refused when it is relative, inside the Proton
    /// Drive app's folder (writing there uploads to Proton, which R11 keeps
    /// off), or inside protonctl's cache, which holds the download folders.
    pub fn new(cfg: &ExportConfig, drive_root: Option<&Path>) -> Result<Self> {
        let folder = &cfg.folder;
        if !folder.is_absolute() {
            bail!("[export] folder must be an absolute path");
        }
        let folder = folder.canonicalize().unwrap_or_else(|_| lexical(folder));
        let cache = cache_dir();
        let cache = cache.canonicalize().unwrap_or(cache);
        if drive_root.is_some_and(|root| folder.starts_with(root)) {
            bail!(
                "[export] folder must be outside the Proton Drive app's folder, where writing uploads to Proton"
            );
        }
        if folder.starts_with(&cache) {
            bail!(
                "[export] folder must be outside {}, which holds the download folders",
                cache.display()
            );
        }
        Ok(Self { folder })
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The folder `parts` names under the export folder, made with mode 0700
    /// at every level protonctl creates. Each part is one name: a part that
    /// is empty, `.`, `..` or holds a '/' is refused.
    pub fn dir(&self, parts: &[&str]) -> Result<PathBuf> {
        let mut dir = self.folder.clone();
        for part in parts {
            if part.is_empty() || *part == "." || *part == ".." || part.contains('/') {
                bail!("{part:?} cannot be a folder name in the export folder");
            }
            dir.push(part);
        }
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        // Whoever else can write the folder (an agent's VM it is shared with)
        // could swap a part for a symlink, say into the Proton Drive folder,
        // where writing uploads. So the folder must still resolve inside.
        let base = self.folder.canonicalize()?;
        if !dir.canonicalize()?.starts_with(&base) {
            bail!(
                "{} leads outside the export folder (a symlink?); refusing to write there",
                dir.display()
            );
        }
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_folder_must_be_absolute_and_outside_drive_and_the_cache() {
        let drive = tempfile::tempdir().unwrap();
        let root = drive.path().canonicalize().unwrap();
        let cfg = |p: PathBuf| ExportConfig { folder: p };
        assert!(Export::new(&cfg("exports".into()), Some(&root)).is_err());
        let inside = Export::new(&cfg(root.join("Exports")), Some(&root));
        assert!(
            inside
                .unwrap_err()
                .to_string()
                .contains("uploads to Proton")
        );
        // A path that reaches the Drive folder only through ".." counts too.
        let name = root.file_name().unwrap();
        let around = root
            .parent()
            .unwrap()
            .join("x/..")
            .join(name)
            .join("Exports");
        assert!(Export::new(&cfg(around), Some(&root)).is_err());
        let cache = cache_dir().join("exports");
        assert!(Export::new(&cfg(cache), Some(&root)).is_err());
        let outside = tempfile::tempdir().unwrap();
        let ok = Export::new(&cfg(outside.path().join("exports")), Some(&root)).unwrap();
        let dir = ok.dir(&["mail", "abc"]).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        for bad in ["", ".", "..", "a/b"] {
            assert!(ok.dir(&["drive", bad]).is_err(), "{bad:?}");
        }
        // A part swapped for a symlink leading out is refused.
        std::os::unix::fs::symlink(&root, ok.folder().join("drive")).unwrap();
        let err = ok.dir(&["drive", "Projects"]).unwrap_err();
        assert!(
            err.to_string().contains("outside the export folder"),
            "{err}"
        );
    }
}
