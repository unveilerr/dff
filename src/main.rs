use clap::Parser;
use std::path::Path;

mod app;
mod scanner;
mod ui;

#[derive(Parser)]
#[command(
    name = "dff",
    version,
    about = "🔍 Duplicate File Finder — scan directories for duplicate files",
    long_about = "\
DFF scans a directory recursively, finds duplicate files by comparing
SHA-256 hashes, and presents them in an interactive TUI.

Algorithm:
  1. Walk directory, group files by size
  2. Compute SHA-256 for same-size candidates (parallel, one thread per CPU)
  3. Group by hash, show duplicates sorted by wasted space
",
    after_help = "\
KEYBINDINGS:
  Navigation             ↑/↓  j/k  g/G  PageUp/PageDown  🖱 wheel
  Mark toggle           Space (hold + scroll to mark multiple)
  Mark group            m
  Unmark group          M
  Mark all              a
  Clear marks           u
  Delete marked         d
  Mark all + delete     D
  Open directory        Enter
  Open file             o
  Quit                  q  Esc  Ctrl+C
"
)]
struct Cli {
    /// Directory to scan
    #[arg(default_value = ".")]
    path: String,

    /// Minimum file size (e.g., 1KB, 5MB, 1GB). Default: 1 byte
    #[arg(short, long)]
    min_size: Option<String>,

    /// Number of threads for hashing (0 = auto-detect)
    #[arg(short = 'j', long, default_value = "0")]
    threads: usize,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let path = Path::new(&cli.path);

    if !path.exists() {
        anyhow::bail!("Error: path '{}' does not exist", cli.path);
    }
    if !path.is_dir() {
        anyhow::bail!("Error: '{}' is not a directory", cli.path);
    }

    let min_size = match &cli.min_size {
        Some(s) => parse_size(s).unwrap_or_else(|| {
            eprintln!(
                "Warning: could not parse size '{}', defaulting to 1 byte",
                s
            );
            1
        }),
        None => 1,
    };

    let threads = if cli.threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        cli.threads
    };

    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global();

    app::run(path, min_size)?;

    Ok(())
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }
    let split_at = s.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    let num: f64 = s[..split_at].parse().ok()?;
    let suffix = &s[split_at..];
    match suffix {
        "kb" | "k" => Some((num * 1000.0) as u64),
        "kib" => Some((num * 1024.0) as u64),
        "mb" | "m" => Some((num * 1000.0 * 1000.0) as u64),
        "mib" => Some((num * 1024.0 * 1024.0) as u64),
        "gb" | "g" => Some((num * 1000.0 * 1000.0 * 1000.0) as u64),
        "gib" => Some((num * 1024.0 * 1024.0 * 1024.0) as u64),
        "tb" | "t" => Some((num * 1000.0 * 1000.0 * 1000.0 * 1000.0) as u64),
        "tib" => Some((num * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64),
        "" => s.parse::<u64>().ok(),
        _ => None,
    }
}
