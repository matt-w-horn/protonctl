//! The export folder, set by `[export] folder` in the config. It is RFC R10's
//! one exception: tools write there only what a call asks for (a Drive file,
//! an attachment, a manifest), only at paths protonctl chooses, and protonctl
//! never cleans it. A folder connected to Cowork gives agents there reach to
//! these files, which they cannot get to in the private download folder.
//!
//! Whoever else can write the folder (an agent's VM it is shared with) can
//! swap any part of it for a symlink at any moment, say into the Proton Drive
//! folder, where whatever is made uploads. So nothing here goes by path: the
//! folder is opened one name at a time from `/`, never following a symlink,
//! each part below it is made and opened the same way relative to the one
//! above, and files are made and opened relative to the folder held open.

use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::{AtFlags, CWD, Mode, OFlags};
use rustix::io::Errno;

use crate::config::{ExportConfig, cache_dir};

#[derive(Debug)]
pub struct Export {
    folder: PathBuf,
}

/// Where `p` leads, for a folder that may not exist yet: the deepest part of
/// it that exists, resolved through symlinks and spelled as on disk, then the
/// parts still to be made. A missing part followed by `..` falls back to
/// resolving `p` by text.
fn resolved(p: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut at = p;
    loop {
        if let Ok(real) = at.canonicalize() {
            return missing
                .iter()
                .rev()
                .fold(real, |path, name| path.join(name));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                at = parent;
            }
            _ => return lexical(p),
        }
    }
}

/// `p` with `.` and `..` resolved by text.
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

/// Flags for opening a folder that must not be a symlink.
fn dir_flags() -> OFlags {
    OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC
}

/// The folder `name` in `dir`, made with mode 0700 when missing, and opened
/// without following a symlink. `shown` names it in errors.
fn step(dir: &OwnedFd, name: &OsStr, shown: &Path) -> Result<OwnedFd> {
    let open = || rustix::fs::openat(dir, name, dir_flags(), Mode::empty());
    let opened = match open() {
        Err(Errno::NOENT) => match rustix::fs::mkdirat(dir, name, Mode::RWXU) {
            Ok(()) | Err(Errno::EXIST) => open(),
            Err(e) => Err(e),
        },
        other => other,
    };
    opened.map_err(|e| match e {
        // O_NOFOLLOW meeting a symlink: ELOOP on Linux and macOS, though
        // some systems answer ENOTDIR when O_DIRECTORY is set as well.
        Errno::LOOP | Errno::NOTDIR => anyhow::anyhow!(
            "{} leads outside the export folder (a symlink, or not a folder); refusing to write there",
            shown.display()
        ),
        e => anyhow::Error::from(std::io::Error::from(e))
            .context(format!("cannot open {}", shown.display())),
    })
}

/// `name` when it is one name: not empty, `.` or `..`, and holding no '/'.
fn one_name(name: &str) -> Result<&str> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        bail!("{name:?} cannot be a name in the export folder");
    }
    Ok(name)
}

/// A folder in the export folder, held open, so a part swapped for a symlink
/// after it was opened cannot send a file elsewhere.
#[derive(Debug)]
pub struct ExportDir {
    fd: OwnedFd,
    path: PathBuf,
}

impl ExportDir {
    /// Where the folder was when it was opened, for results and messages.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A new file `name` here, never one already there, nor a symlink;
    /// opened to read back too, so a digest reads what was written.
    pub fn create(&self, name: &str) -> Result<(File, PathBuf)> {
        let path = self.path.join(one_name(name)?);
        let flags =
            OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::RDWR | OFlags::CLOEXEC;
        match rustix::fs::openat(&self.fd, name, flags, Mode::from_raw_mode(0o666)) {
            Ok(fd) => Ok((File::from(fd), path)),
            Err(Errno::EXIST) => bail!("{} already exists", path.display()),
            Err(e) => Err(std::io::Error::from(e))
                .with_context(|| format!("cannot create {}", path.display())),
        }
    }

    /// The file `name` here when it is a plain file with no other link to
    /// it, opened `flags` (one of read only, or read and write appending).
    /// `None` when there is no such file.
    fn open_plain(&self, name: &str, flags: OFlags) -> Result<Option<(File, PathBuf)>> {
        let path = self.path.join(one_name(name)?);
        // NONBLOCK: a named pipe put there must not hang the open; it means
        // nothing for a plain file.
        let flags = flags | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let file = match rustix::fs::openat(&self.fd, name, flags, Mode::empty()) {
            Ok(fd) => File::from(fd),
            Err(Errno::NOENT) => return Ok(None),
            Err(Errno::LOOP) => bail!("{} is a symlink; refusing it", path.display()),
            Err(e) => {
                return Err(std::io::Error::from(e))
                    .with_context(|| format!("cannot open {}", path.display()));
            }
        };
        // A hard link to a file elsewhere passes O_NOFOLLOW, so the link
        // count decides; checked on the file opened, not on the path.
        let meta = file.metadata()?;
        if !meta.file_type().is_file() || meta.nlink() != 1 {
            bail!(
                "{} is not a plain file of its own (a link?); refusing it",
                path.display()
            );
        }
        Ok(Some((file, path)))
    }

    /// The file `name` here, for writing more at its end and reading back.
    pub fn append(&self, name: &str) -> Result<(File, PathBuf)> {
        self.open_plain(name, OFlags::RDWR | OFlags::APPEND)?
            .with_context(|| {
                format!(
                    "{} is gone; start a new one",
                    self.path.join(name).display()
                )
            })
    }

    /// At most `max` bytes of the file `name` here, or `None` when there is
    /// no file by that name.
    pub fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>> {
        let Some((file, _)) = self.open_plain(name, OFlags::RDONLY)? else {
            return Ok(None);
        };
        let mut held = Vec::new();
        file.take(max).read_to_end(&mut held)?;
        Ok(Some(held))
    }

    /// Delete the file `name` here, one this call made and could not finish.
    pub fn remove(&self, name: &str) {
        if let Ok(name) = one_name(name) {
            rustix::fs::unlinkat(&self.fd, name, AtFlags::empty()).ok();
        }
    }
}

impl Export {
    /// The configured folder, refused when it is relative, inside the Proton
    /// Drive app's folder (writing there uploads to Proton, which R11 keeps
    /// off), or inside protonctl's cache, which holds the download folders.
    /// `drive_folders` are the Proton Drive app's folders, resolved.
    pub fn new<'a>(
        cfg: &ExportConfig,
        drive_folders: impl IntoIterator<Item = &'a Path>,
    ) -> Result<Self> {
        let folder = &cfg.folder;
        if !folder.is_absolute() {
            bail!("[export] folder must be an absolute path");
        }
        // Resolved as far as they exist, so a symlink or another spelling of a
        // folder not made yet cannot lead inside either one unseen.
        let folder = resolved(folder);
        let cache = resolved(&cache_dir());
        if drive_folders
            .into_iter()
            .any(|root| folder.starts_with(root))
        {
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
    /// at every level protonctl creates, and held open. Each part is one
    /// name: a part that is empty, `.`, `..` or holds a '/' is refused.
    ///
    /// The export folder is reached one name at a time from `/` as checked
    /// in `new` (resolved there, so no part of it is a symlink), and every
    /// part below it the same way, so a symlink anywhere on the way, the
    /// export folder itself included, is refused before anything is made
    /// where it leads.
    pub fn dir(&self, parts: &[&str]) -> Result<ExportDir> {
        for part in parts {
            if one_name(part).is_err() {
                bail!("{part:?} cannot be a folder name in the export folder");
            }
        }
        let mut fd = rustix::fs::openat(CWD, "/", dir_flags(), Mode::empty())
            .map_err(std::io::Error::from)
            .context("cannot open /")?;
        let mut path = PathBuf::from("/");
        for c in self.folder.components() {
            match c {
                Component::RootDir => {}
                Component::Normal(name) => {
                    path.push(name);
                    fd = step(&fd, name, &path)?;
                }
                _ => bail!(
                    "{} is not a plain absolute path; spell [export] folder without . or ..",
                    self.folder.display()
                ),
            }
        }
        for part in parts {
            path.push(part);
            fd = step(&fd, OsStr::new(part), &path)?;
        }
        Ok(ExportDir { fd, path })
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
        assert!(Export::new(&cfg("exports".into()), [root.as_path()]).is_err());
        let inside = Export::new(&cfg(root.join("Exports")), [root.as_path()]);
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
        assert!(Export::new(&cfg(around), [root.as_path()]).is_err());
        let cache = cache_dir().join("exports");
        assert!(Export::new(&cfg(cache), [root.as_path()]).is_err());
        let outside = tempfile::tempdir().unwrap();
        let ok = Export::new(&cfg(outside.path().join("exports")), [root.as_path()]).unwrap();
        let dir = ok.dir(&["mail", "abc"]).unwrap();
        let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        for bad in ["", ".", "..", "a/b"] {
            assert!(ok.dir(&["drive", bad]).is_err(), "{bad:?}");
        }
        // A part swapped for a symlink leading out is refused, and nothing
        // is made where it leads: a folder made in the Drive folder uploads.
        std::os::unix::fs::symlink(&root, ok.folder().join("drive")).unwrap();
        let err = ok.dir(&["drive", "Projects"]).unwrap_err();
        assert!(
            err.to_string().contains("outside the export folder"),
            "{err}"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    /// RFC R10: a symlink at any part, or at the export folder itself after
    /// it was checked, is refused with nothing made where it leads.
    #[test]
    fn a_symlink_anywhere_on_the_way_is_refused_with_nothing_made_outside() {
        let drive = tempfile::tempdir().unwrap();
        let root = drive.path().canonicalize().unwrap();
        let cfg = |p: PathBuf| ExportConfig { folder: p };
        let outside = tempfile::tempdir().unwrap();
        let ok = Export::new(&cfg(outside.path().join("exports")), [root.as_path()]).unwrap();
        ok.dir(&["drive", "a"]).unwrap();
        // A deeper part.
        std::os::unix::fs::symlink(&root, ok.folder().join("drive/a/b")).unwrap();
        assert!(ok.dir(&["drive", "a", "b", "c"]).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        // The export folder itself, swapped for a symlink after the check.
        let moved = outside.path().join("moved");
        std::fs::rename(ok.folder(), &moved).unwrap();
        std::os::unix::fs::symlink(&root, ok.folder()).unwrap();
        let err = ok.dir(&["mail", "x"]).unwrap_err();
        assert!(
            err.to_string().contains("outside the export folder"),
            "{err}"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn a_folder_not_made_yet_is_checked_where_it_will_be() {
        let drive = tempfile::tempdir().unwrap();
        let root = drive.path().canonicalize().unwrap();
        let cfg = |p: PathBuf| ExportConfig { folder: p };
        // Reached through a symlink to the Drive folder.
        let elsewhere = tempfile::tempdir().unwrap();
        let link = elsewhere.path().join("Drive");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let err = Export::new(&cfg(link.join("exports/new")), [root.as_path()]).unwrap_err();
        assert!(err.to_string().contains("uploads to Proton"), "{err}");
        // Spelled in another case, which a case-insensitive disk ignores.
        let shouted = PathBuf::from(root.to_string_lossy().to_uppercase());
        if shouted.exists() {
            assert!(Export::new(&cfg(shouted.join("exports")), [root.as_path()]).is_err());
        }
    }
}
