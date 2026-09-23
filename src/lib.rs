//! Portable order archiving for Sir Zips-a-Lot.

use std::{
    fs::{self, File},
    io,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result, ensure};
use tempfile::Builder;
use walkdir::WalkDir;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

pub mod watcher;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EntryState {
    path: PathBuf,
    directory: bool,
    size: u64,
    modified: SystemTime,
}

pub(crate) fn snapshot(source: &Path) -> Result<Vec<EntryState>> {
    WalkDir::new(source)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .map(|entry| {
            let entry = entry.context("Cannot read an order entry")?;
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                !is_link(&metadata),
                "Symbolic links and junctions are unsupported: {}",
                entry.path().display()
            );
            ensure!(
                metadata.is_dir() || metadata.is_file(),
                "Unsupported file type: {}",
                entry.path().display()
            );
            Ok(EntryState {
                path: entry.path().to_path_buf(),
                directory: metadata.is_dir(),
                size: metadata.len(),
                modified: metadata.modified()?,
            })
        })
        .collect()
}

/// Package a single file or order directory into a completed ZIP in `destination`.
///
/// A directory's contents live at the ZIP root, so extracting into a folder named
/// after the ZIP restores the order without nesting a second order folder.
/// Source files are preserved. Existing archives are never overwritten. Callers
/// must establish that the source is ready before starting this operation.
pub fn archive_order(source: &Path, destination: &Path) -> Result<PathBuf> {
    archive_checked(source, destination, None)
}

pub(crate) fn archive_checked(
    source: &Path,
    destination: &Path,
    expected: Option<&[EntryState]>,
) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Cannot read order {}", source.display()))?;
    ensure!(
        !is_link(&metadata),
        "Order must not be a symbolic link or junction"
    );
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Order must be a file or directory"
    );

    let source = source.canonicalize().context("Cannot resolve order path")?;
    let before = snapshot(&source)?;
    if let Some(expected) = expected {
        ensure!(
            before == expected,
            "Order changed before packaging; waiting for it to settle"
        );
    }
    fs::create_dir_all(destination).context("Cannot create destination folder")?;
    let destination = destination
        .canonicalize()
        .context("Cannot resolve destination path")?;
    ensure!(
        !destination.starts_with(&source),
        "Destination must be outside the order folder"
    );

    let name = source
        .file_name()
        .context("Cannot archive a filesystem root")?;
    let mut archive_name = name.to_os_string();
    archive_name.push(".zip");
    let target = destination.join(archive_name);
    ensure!(
        !target.try_exists()?,
        "Archive already exists: {}",
        target.display()
    );

    // Stage on the destination filesystem so publication works across drives.
    let mut staged = Builder::new()
        .prefix(".sir-zips-a-lot-")
        .suffix(".partial")
        .tempfile_in(&destination)
        .context("Cannot create temporary archive")?;
    let mut writer = ZipWriter::new(staged.as_file_mut());
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    let archive_root = if metadata.is_dir() {
        source.as_path()
    } else {
        source.parent().context("Order has no parent directory")?
    };

    for entry in &before {
        let relative = entry.path.strip_prefix(archive_root)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let zip_name = archive_path(relative)?;
        if entry.directory {
            writer.add_directory(format!("{zip_name}/"), options)?;
        } else {
            writer.start_file(zip_name, options)?;
            let mut input = File::open(&entry.path)
                .with_context(|| format!("Cannot open {}", entry.path.display()))?;
            io::copy(&mut input, &mut writer).context("Cannot write file into archive")?;
        }
    }
    writer.finish().context("Cannot finish archive")?;
    ensure!(
        snapshot(&source)? == before,
        "Order changed during packaging; please retry after it settles"
    );
    staged
        .as_file()
        .sync_all()
        .context("Cannot flush archive")?;
    staged
        .persist_noclobber(&target)
        .with_context(|| format!("Cannot publish archive {}", target.display()))?;
    Ok(target)
}

fn archive_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            anyhow::bail!("Archive paths must be relative");
        };
        let part = part
            .to_str()
            .context("Order filenames must be valid Unicode")?;
        ensure!(
            !part.contains('\\'),
            "Backslashes in filenames are unsupported"
        );
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reparse points include directory junctions as well as symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
