# 🔍 DFF — Duplicate File Finder

**DFF** is a terminal user interface (TUI) tool for finding and removing duplicate files,
written in Rust. It scans a directory recursively, identifies duplicates by comparing
SHA-256 hashes, and lets you delete them right from the terminal.

[![Crates.io][crates-badge]][crates-url]
[![MIT License][license-badge]][license-url]
![Platform][platform-badge]
![Rust][rust-badge]

[crates-badge]: https://img.shields.io/badge/crates.io-v0.1.0-orange
[crates-url]: https://crates.io/crates/dff
[license-badge]: https://img.shields.io/badge/license-MIT-blue
[license-url]: #license
[platform-badge]: https://img.shields.io/badge/platform-linux%20%7C%20macOS%20%7C%20windows-lightgrey
[rust-badge]: https://img.shields.io/badge/rust-1.85+-orange

## Features

- ⚡ **Fast** — parallel SHA-256 hashing via `rayon` (one thread per CPU by default)
- 🔍 **Two-pass hash** — quick header/footer fingerprint first, full SHA-256 only for matches
- 🖱️ **Mouse support** — scroll wheel and click to select
- 🏷️ **Hold‑to‑mark** — press Space then scroll to mark multiple files in one go
- 🗑️ **Bulk delete** — mark all duplicates at once (`a`) or mark+delete in one action (`D`)
- 🎨 **Theme-aware** — uses only standard ANSI colours (works in light & dark terminals)
- 📁 **Open & explore** — press Enter to open a file's directory or `o` to open the file
- 📊 **Progress bars** — live progress while walking and hashing

## Installation

### From source

```bash
git clone https://github.com/unveilerr/dff.git
cd dff
cargo build --release
cp target/release/dff ~/.local/bin/
```

> **Requirements:** Rust 1.85+, any UTF-8-capable terminal.

### Run without installing

```bash
cargo run -- ~/Downloads
```

## Usage

```bash
# Scan the current directory
dff

# Scan a specific directory
dff ~/Downloads

# Skip files smaller than 1 MB
dff ~/photos --min-size 1MB

# Use 8 threads for hashing
dff /data --threads 8

# Show help
dff --help
```

### `--min-size`

Accepts formats: `1KB`, `5MB`, `1GB`, `500Kib`, `2MiB`, `1GiB`, etc.
Both decimal (KB/MB/GB) and binary (KiB/MiB/GiB) prefixes are supported.

## Keybindings

| Key | Action |
|---|---|
| `↑/↓` `j/k` | Navigate between files |
| `g` / `G` | Go to top / bottom |
| `PageUp` / `PageDown` | Jump 10 lines |
| `Space` | Toggle mark on current file |
| `Space` + `↑/↓` | **Hold Space** + scroll — marks every file you pass |
| `m` | Mark all duplicates in the current group |
| `M` | **Unmark** all files in the current group |
| `a` | Mark **ALL** duplicates in every group |
| `d` | Delete marked files (with confirmation) |
| `D` (`Shift+d`) | Mark all duplicates + prompt delete immediately |
| `u` | Clear all marks |
| `Enter` | Open the directory containing the file |
| `o` | Open the file in the default application |
| `q` / `Esc` / `Ctrl+C` | Quit |
| 🖱️ Scroll wheel | Scroll through files |
| 🖱️ Left click | Select file under cursor |

### Safety

- Commands `m`, `a`, `D` **never** mark the first copy in a group — at least one
  copy always survives.
- `Space` allows marking *any* file, but the confirmation dialog warns if the
  **last remaining copy** of a file is about to be deleted.
- Every deletion requires an explicit `[Y]es` confirmation.

## How it works

```
1. Walk directory        → group files by size
2. Quick hash            → SHA-256 of first + last 4 KB (fast filter)
3. Full SHA-256          → definitive hash for remaining candidates (parallel)
4. Group by hash         → keep groups with >1 file
5. Sort by wasted space  → largest waste first
```

Files are compared in three stages for maximum speed:
1. **Size** — files with unique sizes are immediately skipped (fast filter)
2. **Quick hash** — reads only 8 KB per file (first + last 4 KB), eliminates the vast majority of non‑duplicates without touching the full file
3. **Full SHA-256** — only for files that passed the quick-hash check

## TUI layout

```
┌─ ⚡ DFF v0.1.0 │ 5 groups · 23 files · 1.2 GB wasted ─────┐
│                                                             │
│   1.  IMG_2022.JPG      4 × 12 MB  (36 MB wasted)          │
│     ◉ ./photos/IMG_2022.JPG              12 MB              │
│     ▸ ./backup/photos/IMG_2022.JPG       12 MB              │
│     ◉ ./Downloads/IMG_2022(1).JPG        12 MB              │
│     ◉ ./Desktop/IMG_2022.JPG             12 MB              │
│                                                             │
│   2.  project-backup.tar.gz   3 × 800 MB  (1.6 GB wasted)   │
│     ○ ./archive/project-backup-2024.tar.gz  800 MB          │
│     ○ ./old/project-backup.tar.gz            800 MB         │
│     ○ ./tmp/project-backup.tar.gz            800 MB         │
│                                                             │
│  ──────────────────────────── 60% · file 8 of 23            │
├─ 3 marked · [Space] hold+scroll · [m/M] group · [a] all … ─┤
│ [↑/↓] nav · [Space] mark · [m] mark group · [a] all · …    │
└─────────────────────────────────────────────────────────────┘
```

## Colour scheme

DFF uses only **standard ANSI colours** (no hardcoded RGB), so it automatically
adapts to your terminal theme — whether light, dark, or anything in between.

- Selection: `Modifier::REVERSED` (terminal inversion)
- Accents: `Cyan`, `Yellow` (follow your terminal palette)
- Marks: `Red`
- Success: `Green`
- Dim / separators: `DarkGray`

## Dependencies

| Crate | Purpose |
|---|---|
| [`ratatui`](https://crates.io/crates/ratatui) | TUI rendering framework |
| [`crossterm`](https://crates.io/crates/crossterm) | Terminal backend & event handling |
| [`walkdir`](https://crates.io/crates/walkdir) | Recursive directory walking |
| [`sha2`](https://crates.io/crates/sha2) | SHA-256 hashing |
| [`rayon`](https://crates.io/crates/rayon) | Parallel hash computation |
| [`clap`](https://crates.io/crates/clap) | CLI argument parsing |
| [`anyhow`](https://crates.io/crates/anyhow) | Error handling |
| [`opener`](https://crates.io/crates/opener) | Opening files/directories in desktop |

## License

MIT
