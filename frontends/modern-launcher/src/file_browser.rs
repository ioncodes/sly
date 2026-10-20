use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::worker::LatestJob;

pub const MAX_DIRECTORY_ENTRIES: usize = 1024;
const MAX_WARNINGS: usize = 16;

#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_directory: bool,
}

#[derive(Debug)]
pub struct DirectoryListing {
    pub path: PathBuf,
    pub entries: Vec<DirectoryEntry>,
    pub warnings: Vec<String>,
    pub truncated: bool,
}

pub type DirectoryBrowser = LatestJob<PathBuf, Result<DirectoryListing, String>>;

impl DirectoryBrowser {
    pub fn new(start: PathBuf) -> io::Result<Self> {
        let mut browser = Self::spawn("launcher-folders", |path, cancelled| {
            read_directory(&path, cancelled)
        })?;

        browser.navigate(start);
        Ok(browser)
    }

    pub fn default_start(preferred: Option<&Path>) -> PathBuf {
        preferred
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .or_else(|| {
                std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                    .map(PathBuf::from)
            })
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR.to_string()))
    }

    pub fn navigate(&mut self, path: PathBuf) {
        self.request(path);
    }
}

/// Windows has no single filesystem root, so an empty path lists the available drives instead.
pub fn is_drive_list(path: &Path) -> bool {
    cfg!(windows) && path.as_os_str().is_empty()
}

fn read_drives() -> DirectoryListing {
    let entries = (b'A'..=b'Z')
        .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
        .filter(|path| path.is_dir())
        .map(|path| DirectoryEntry {
            name: path.display().to_string(),
            path,
            is_directory: true,
        })
        .collect();

    DirectoryListing {
        path: PathBuf::new(),
        entries,
        warnings: Vec::new(),
        truncated: false,
    }
}

fn read_directory(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<DirectoryListing, String> {
    if is_drive_list(path) {
        return Ok(read_drives());
    }

    let path = path
        .canonicalize()
        .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;

    let directory = std::fs::read_dir(&path)
        .map_err(|error| format!("Cannot list {}: {error}", path.display()))?;

    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .or_else(|| cfg!(windows).then(PathBuf::new))
        .map(|path| DirectoryEntry {
            name: "..".into(),
            path,
            is_directory: true,
        });

    let capacity = MAX_DIRECTORY_ENTRIES - usize::from(parent.is_some());
    let mut ordered = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut total = 0usize;
    let mut symlinks = 0usize;

    for item in directory {
        if cancelled() {
            return Err("Directory navigation cancelled".into());
        }

        let item = match item {
            Ok(item) => item,
            Err(error) => {
                if warnings.len() < MAX_WARNINGS {
                    warnings.push(error.to_string());
                }

                continue;
            }
        };

        let file_type = match item.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                if warnings.len() < MAX_WARNINGS {
                    warnings.push(format!("{}: {error}", item.path().display()));
                }

                continue;
            }
        };

        if file_type.is_symlink() {
            symlinks += 1;
            continue;
        }

        if !file_type.is_dir() && !file_type.is_file() {
            continue;
        }

        total += 1;
        let name = item.file_name().to_string_lossy().into_owned();

        let entry = DirectoryEntry {
            name,
            path: item.path(),
            is_directory: file_type.is_dir(),
        };

        let key = (
            !entry.is_directory,
            entry.name.to_lowercase(),
            entry.path.clone(),
        );

        if ordered.len() < capacity
            || ordered
                .last_key_value()
                .is_some_and(|(last, _)| key < *last)
        {
            ordered.insert(key, entry);

            if ordered.len() > capacity {
                ordered.pop_last();
            }
        }
    }

    if symlinks > 0 && warnings.len() < MAX_WARNINGS {
        warnings.push(format!("{symlinks} symbolic links hidden"));
    }

    let truncated = total > capacity;

    if truncated && warnings.len() < MAX_WARNINGS {
        warnings.push(format!(
            "Showing the first {capacity} entries of {total}; \
             choose a more specific configured root for additional folders"
        ));
    }

    let entries = parent.into_iter().chain(ordered.into_values()).collect();

    Ok(DirectoryListing {
        path,
        entries,
        warnings,
        truncated,
    })
}
