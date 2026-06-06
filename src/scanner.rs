use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub file_name: String,
    #[allow(dead_code)]
    pub hash: String,
    pub files: Vec<FileEntry>,
    pub wasted_bytes: u64,
}

#[derive(Debug, Clone)]
pub enum ScanProgress {
    Walking {
        found: usize,
    },
    QuickHash {
        current: usize,
        total: usize,
    },
    FullHash {
        current: usize,
        total: usize,
    },
    Done {
        groups: Vec<DuplicateGroup>,
        total_files: usize,
    },
}

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

pub fn scan_directory(root: &Path, min_size: u64, tx: mpsc::Sender<ScanProgress>) {
    // ---- Phase 1: walk & group by file size ---------------------------------
    let mut size_groups: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    let mut total = 0;

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let size = meta.len();
        if size < min_size {
            continue;
        }
        size_groups.entry(size).or_default().push(FileEntry {
            path: entry.path().to_path_buf(),
            size,
        });
        total += 1;
        if total % 100 == 0 {
            let _ = tx.send(ScanProgress::Walking { found: total });
        }
    }

    let _ = tx.send(ScanProgress::Walking { found: total });

    // ---- Phase 2: candidates (files that share a size) -----------------------
    let candidates: Vec<FileEntry> = size_groups
        .into_values()
        .filter(|v| v.len() > 1)
        .flatten()
        .collect();

    let total_candidates = candidates.len();
    if total_candidates == 0 {
        let _ = tx.send(ScanProgress::Done {
            groups: Vec::new(),
            total_files: total,
        });
        return;
    }

    // ---- Phase 3: quick hash (first + last 4 KB) ----------------------------
    //  This cheap filter eliminates the vast majority of non-duplicates
    //  without reading every byte of every file.
    let _ = tx.send(ScanProgress::QuickHash {
        current: 0,
        total: total_candidates,
    });

    let quick_hashed: Vec<_> = candidates
        .par_iter()
        .enumerate()
        .map(|(i, entry)| {
            let qh = quick_hash(&entry.path).unwrap_or(String::new());
            if i % 100 == 0 {
                let _ = tx.send(ScanProgress::QuickHash {
                    current: i,
                    total: total_candidates,
                });
            }
            (qh, entry.clone())
        })
        .collect();

    let _ = tx.send(ScanProgress::QuickHash {
        current: total_candidates,
        total: total_candidates,
    });

    // ---- Phase 4: group by quick hash, discard singletons --------------------
    let mut qh_groups: HashMap<String, Vec<FileEntry>> = HashMap::new();
    for (qh, entry) in quick_hashed {
        if !qh.is_empty() {
            qh_groups.entry(qh).or_default().push(entry);
        }
    }

    let full_candidates: Vec<FileEntry> = qh_groups
        .into_values()
        .filter(|v| v.len() > 1)
        .flatten()
        .collect();

    let total_full = full_candidates.len();
    let _ = tx.send(ScanProgress::FullHash {
        current: 0,
        total: total_full,
    });

    if total_full == 0 {
        let _ = tx.send(ScanProgress::Done {
            groups: Vec::new(),
            total_files: total,
        });
        return;
    }

    // ---- Phase 5: full SHA-256 ----------------------------------------------
    let full_hashed: Vec<_> = full_candidates
        .par_iter()
        .enumerate()
        .map(|(i, entry)| {
            let fh = sha256_file(&entry.path).unwrap_or(String::new());
            if i % 10 == 0 {
                let _ = tx.send(ScanProgress::FullHash {
                    current: i,
                    total: total_full,
                });
            }
            (fh, entry.clone())
        })
        .collect();

    let _ = tx.send(ScanProgress::FullHash {
        current: total_full,
        total: total_full,
    });

    // ---- Phase 6: group by full hash, build result --------------------------
    let mut fh_groups: HashMap<String, Vec<FileEntry>> = HashMap::new();
    for (fh, entry) in full_hashed {
        if !fh.is_empty() {
            fh_groups.entry(fh).or_default().push(entry);
        }
    }

    let mut groups: Vec<DuplicateGroup> = fh_groups
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(hash, mut files)| {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            let wasted = files.iter().skip(1).map(|f| f.size).sum();
            let name = files[0]
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            DuplicateGroup {
                file_name: name,
                hash: hash[..12.min(hash.len())].to_string(),
                files,
                wasted_bytes: wasted,
            }
        })
        .collect();

    groups.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes));

    let _ = tx.send(ScanProgress::Done {
        groups,
        total_files: total,
    });
}

// ---------------------------------------------------------------------------
// Hashing helpers
// ---------------------------------------------------------------------------

/// Read only the first 4 KB (and last 4 KB for files > 8 KB) to get a quick
/// fingerprint.  This is up to 10 000× faster than a full SHA-256 for large
/// files and catches the vast majority of non‑duplicates.
fn quick_hash(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 4096];

    // First 4 KB
    let n = file.read(&mut buf)?;
    hasher.update(&buf[..n]);

    // Last 4 KB (if the file is bigger than 8 KB)
    if size > 8192 {
        file.seek(SeekFrom::End(-4096))?;
        let n = file.read(&mut buf)?;
        hasher.update(&buf[..n]);
    }

    // Truncate to 8 hex chars for smaller maps
    Ok(format!("{:x}", hasher.finalize())[..8].to_string())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
