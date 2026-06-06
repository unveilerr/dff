use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::scanner::{self, DuplicateGroup, FileEntry, ScanProgress};
use crate::ui;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct App {
    pub groups: Vec<DuplicateGroup>,
    pub total_scanned: usize,
    pub selected_idx: usize,
    pub selected_row: usize,
    pub scroll_offset: usize,
    pub marked: HashSet<usize>,
    pub status_message: Option<(String, Instant)>,
    pub scanning: bool,
    pub scan_progress: String,
    pub confirm_delete: bool,
    pub space_held: bool,
    pub space_at: Option<Instant>,
    pub should_quit: bool,
    pub last_tick: Instant,
    spinner_idx: usize,
    /// Precomputed (group_idx, file_idx) for each flat index — O(1) lookup.
    /// Rebuilt whenever groups change (scan done / after delete).
    flat_lookup: Vec<(usize, usize)>,
}

impl App {
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            total_scanned: 0,
            selected_idx: 0,
            selected_row: 0,
            scroll_offset: 0,
            marked: HashSet::new(),
            status_message: None,
            scanning: true,
            scan_progress: "Initializing…".to_string(),
            confirm_delete: false,
            space_held: false,
            space_at: None,
            should_quit: false,
            last_tick: Instant::now(),
            spinner_idx: 0,
            flat_lookup: Vec::new(),
        }
    }

    // -- convenience queries --------------------------------------------------

    pub fn total_files_in_groups(&self) -> usize {
        self.groups.iter().map(|g| g.files.len()).sum()
    }

    pub fn total_wasted(&self) -> u64 {
        self.groups.iter().map(|g| g.wasted_bytes).sum()
    }

    pub fn total_groups(&self) -> usize {
        self.groups.len()
    }

    pub fn flat_count(&self) -> usize {
        self.groups.iter().map(|g| g.files.len()).sum()
    }

    // -- index mapping --------------------------------------------------------

    /// Convert flat index → (group_index, file_index) — O(1).
    pub fn flat_to_group_file(&self, flat_idx: usize) -> Option<(usize, usize)> {
        self.flat_lookup.get(flat_idx).copied()
    }

    /// Rebuild `flat_lookup` from current groups.
    pub fn rebuild_flat_lookup(&mut self) {
        self.flat_lookup.clear();
        for (gi, group) in self.groups.iter().enumerate() {
            for fi in 0..group.files.len() {
                self.flat_lookup.push((gi, fi));
            }
        }
    }

    /// Convert flat index → row index in the rendered list
    pub fn file_to_row(&self, flat_idx: usize) -> Option<usize> {
        let (gi, fi) = self.flat_to_group_file(flat_idx)?;
        let mut row = 0;
        for g in 0..gi {
            row += 1; // header
            row += self.groups[g].files.len();
            row += 1; // separator
        }
        row += 1; // header of current group
        row += fi;
        Some(row)
    }

    pub fn update_selected_row(&mut self) {
        self.selected_row = self.file_to_row(self.selected_idx).unwrap_or(0);
    }

    pub fn total_rows(&self) -> usize {
        let mut rows = 0;
        for (gi, group) in self.groups.iter().enumerate() {
            rows += 1; // group header
            rows += group.files.len();
            if gi < self.groups.len() - 1 {
                rows += 1; // separator
            }
        }
        rows
    }

    pub fn current_file(&self) -> Option<(usize, usize, &FileEntry)> {
        self.flat_to_group_file(self.selected_idx)
            .map(|(gi, fi)| (gi, fi, &self.groups[gi].files[fi]))
    }

    // -- navigation -----------------------------------------------------------

    pub fn move_selection(&mut self, delta: i32) {
        let total = self.flat_count();
        if total == 0 {
            return;
        }
        let new = (self.selected_idx as i32 + delta).clamp(0, total as i32 - 1) as usize;
        self.selected_idx = new;
        self.update_selected_row();
    }

    pub fn go_top(&mut self) {
        if self.flat_count() > 0 {
            self.selected_idx = 0;
            self.update_selected_row();
        }
    }

    pub fn go_bottom(&mut self) {
        let total = self.flat_count();
        if total > 0 {
            self.selected_idx = total - 1;
            self.update_selected_row();
        }
    }

    // -- marking --------------------------------------------------------------

    pub fn toggle_mark(&mut self) {
        if self.marked.contains(&self.selected_idx) {
            self.marked.remove(&self.selected_idx);
        } else {
            self.marked.insert(self.selected_idx);
        }
    }

    /// Mark all duplicates in the current group (keep the first copy)
    pub fn mark_duplicates(&mut self) {
        if let Some((gi, _, _)) = self.current_file() {
            let mut base = 0;
            for g in 0..gi {
                base += self.groups[g].files.len();
            }
            for fi in 1..self.groups[gi].files.len() {
                self.marked.insert(base + fi);
            }
        }
    }

    /// Unmark all files in the current group (opposite of mark_duplicates)
    pub fn unmark_group(&mut self) {
        if let Some((gi, _, _)) = self.current_file() {
            let mut base = 0;
            for g in 0..gi {
                base += self.groups[g].files.len();
            }
            for fi in 0..self.groups[gi].files.len() {
                self.marked.remove(&(base + fi));
            }
        }
    }

    /// Returns `true` if any marked file is the first copy in its group.
    /// This means deleting would destroy the last remaining copy of that file.
    pub fn has_primary_marked(&self) -> bool {
        self.marked.iter().any(|&idx| {
            self.flat_to_group_file(idx)
                .map(|(_, fi)| fi == 0)
                .unwrap_or(false)
        })
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    /// Mark all duplicates across ALL groups (keep one copy per group)
    pub fn mark_all_duplicates(&mut self) {
        let mut base = 0;
        for group in &self.groups {
            for fi in 1..group.files.len() {
                self.marked.insert(base + fi);
            }
            base += group.files.len();
        }
    }

    /// Map rendered row number → flat file index
    pub fn row_to_file(&self, row: usize) -> Option<usize> {
        let mut current = 0;
        for (gi, group) in self.groups.iter().enumerate() {
            // header
            if current == row {
                return None;
            }
            current += 1;

            for fi in 0..group.files.len() {
                if current == row {
                    let mut flat = 0;
                    for g in 0..gi {
                        flat += self.groups[g].files.len();
                    }
                    flat += fi;
                    return Some(flat);
                }
                current += 1;
            }

            // separator between groups
            if gi < self.groups.len() - 1 {
                if current == row {
                    return None;
                }
                current += 1;
            }
        }
        None
    }

    // -- deletion -------------------------------------------------------------

    pub fn delete_marked(&mut self) -> Result<(usize, u64)> {
        let to_delete: Vec<(usize, String, u64)> = self
            .marked
            .iter()
            .filter_map(|&idx| {
                self.flat_to_group_file(idx).map(|(gi, fi)| {
                    let entry = &self.groups[gi].files[fi];
                    (idx, entry.path.to_string_lossy().to_string(), entry.size)
                })
            })
            .collect();

        let mut deleted = 0;
        let mut freed = 0;

        for (_, path_str, size) in &to_delete {
            if std::fs::remove_file(path_str).is_ok() {
                deleted += 1;
                freed += size;
            }
        }

        // Remove entries from groups (reverse order to keep indices valid)
        let mut to_remove: Vec<(usize, usize)> = self
            .marked
            .iter()
            .filter_map(|&idx| self.flat_to_group_file(idx))
            .collect();
        to_remove.sort_by(|a, b| b.cmp(a));

        for (gi, fi) in to_remove {
            if fi < self.groups[gi].files.len() {
                self.groups[gi].files.remove(fi);
            }
        }

        // Drop groups that no longer have duplicates
        self.groups.retain(|g| g.files.len() > 1);
        for group in &mut self.groups {
            group.wasted_bytes = group.files.iter().skip(1).map(|f| f.size).sum();
        }

        self.rebuild_flat_lookup();
        self.marked.clear();
        let total = self.flat_count();
        self.selected_idx = self.selected_idx.min(total.saturating_sub(1));
        self.update_selected_row();

        Ok((deleted, freed))
    }

    // -- open ----------------------------------------------------------------

    pub fn open_dir(&self) {
        if let Some((_, _, file)) = self.current_file() {
            if let Some(parent) = file.path.parent() {
                let _ = opener::open(parent);
            }
        }
    }

    pub fn open_file(&self) {
        if let Some((_, _, file)) = self.current_file() {
            let _ = opener::open(&file.path);
        }
    }

    // -- helpers -------------------------------------------------------------

    pub fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }

    pub fn tick_spinner(&mut self) {
        self.spinner_idx = (self.spinner_idx + 1) % 10;
    }

    pub fn spinner_char(&self) -> char {
        const SPINNERS: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        SPINNERS[self.spinner_idx]
    }
}

// ---------------------------------------------------------------------------
// Main entry point (terminal setup + event loop)
// ---------------------------------------------------------------------------

pub fn run(root: &Path, min_size: u64) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, event::EnableMouseCapture,)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    // Start scanner in background thread
    let (scan_tx, scan_rx) = mpsc::channel();
    let scan_root = root.to_path_buf();
    std::thread::spawn(move || {
        scanner::scan_directory(&scan_root, min_size, scan_tx);
    });

    // Event loop
    loop {
        // Drain scan progress messages
        while let Ok(progress) = scan_rx.try_recv() {
            match progress {
                ScanProgress::Walking { found } => {
                    app.scan_progress = format!("Walking … {} files found", found);
                }
                ScanProgress::QuickHash { current, total } => {
                    let pct = if total > 0 { current * 100 / total } else { 0 };
                    app.scan_progress = format!("Quick hash … {}/{} ({}%)", current, total, pct);
                }
                ScanProgress::FullHash { current, total } => {
                    let pct = if total > 0 { current * 100 / total } else { 0 };
                    app.scan_progress = format!("Full hash … {}/{} ({}%)", current, total, pct);
                }
                ScanProgress::Done {
                    groups,
                    total_files,
                } => {
                    app.groups = groups;
                    app.rebuild_flat_lookup();
                    app.total_scanned = total_files;
                    app.scanning = false;
                    app.update_selected_row();

                    let fg = app.total_files_in_groups();
                    let gc = app.total_groups();
                    let ws = app.total_wasted();
                    if gc > 0 {
                        app.set_status(format!(
                            "✓ Scan complete · {} groups · {} files · {} wasted",
                            gc,
                            fg,
                            format_bytes(ws),
                        ));
                    } else {
                        app.set_status("✓ No duplicates found · clean directory!".to_string());
                    }
                }
            }
        }

        // Poll for events
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key)
                    if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat =>
                {
                    handle_key(&mut app, key);
                }
                Event::Mouse(m) => handle_mouse(&mut app, m),
                _ => {}
            }
        }

        // Animation & space timeout tick
        if app.last_tick.elapsed() >= Duration::from_millis(80) {
            app.tick_spinner();
            app.last_tick = Instant::now();
        }

        // Auto-expire space-held mode after 300 ms of inactivity
        if app.space_held {
            if let Some(t) = app.space_at {
                if t.elapsed() > Duration::from_millis(300) {
                    app.space_held = false;
                    app.space_at = None;
                }
            }
        }

        // Render
        terminal.draw(|f| ui::draw(f, &mut app))?;

        if app.should_quit {
            break;
        }
    }

    // Cleanup
    execute!(
        terminal.backend_mut(),
        event::DisableMouseCapture,
        LeaveAlternateScreen,
    )?;
    disable_raw_mode()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Key dispatch
// ---------------------------------------------------------------------------

fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use KeyCode::*;

    // —— Confirmation dialog ——
    if app.confirm_delete {
        match key.code {
            Char('y') | Enter => {
                match app.delete_marked() {
                    Ok((count, freed)) => {
                        if count > 0 {
                            app.set_status(format!(
                                "✓ Deleted {} files ({} freed)",
                                count,
                                format_bytes(freed),
                            ));
                        } else {
                            app.set_status("✗ Could not delete any files".to_string());
                        }
                    }
                    Err(e) => app.set_status(format!("✗ Delete error: {}", e)),
                }
                app.confirm_delete = false;
            }
            Char('n') | Esc => {
                app.confirm_delete = false;
                app.set_status("Deletion cancelled".to_string());
            }
            _ => {}
        }
        return;
    }

    // —— Ctrl+C always quits ——
    if key.code == Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }

    // —— During scanning only q works ——
    if app.scanning {
        match key.code {
            Char('q') | Esc => app.should_quit = true,
            _ => {}
        }
        return;
    }

    // ──────────────────────────────────────────────────────────────────────
    //  Space handling — Press toggles mark and starts a 300 ms window.
    //  Any movement within the window also toggles the target file.
    //  When movement stops (>300 ms) the window closes automatically.
    //  Pressing Space again also closes it.
    // ──────────────────────────────────────────────────────────────────────
    if key.code == Char(' ') {
        if key.kind == KeyEventKind::Press {
            if app.space_held {
                // Second press while still held → disable without toggling
                app.space_held = false;
                app.space_at = None;
            } else {
                app.toggle_mark();
                app.space_held = true;
                app.space_at = Some(Instant::now());
            }
        }
        // Ignore Space repeat events
        return;
    }

    // —— Navigation (with possible space-held marking) ——
    let moved = match key.code {
        Char('j') | Down => {
            app.move_selection(1);
            true
        }
        Char('k') | Up => {
            app.move_selection(-1);
            true
        }
        Char('G') => {
            app.go_bottom();
            false
        }
        Char('g') => {
            app.go_top();
            false
        }
        PageDown => {
            app.move_selection(10);
            false
        }
        PageUp => {
            app.move_selection(-10);
            false
        }
        Home => {
            app.go_top();
            false
        }
        End => {
            app.go_bottom();
            false
        }
        _ => false,
    };

    if moved && app.space_held {
        app.toggle_mark();
        app.space_at = Some(Instant::now()); // refresh timeout
    }
    if moved {
        return;
    }

    // —— Not Space, not movement → exit held mode ——
    app.space_held = false;

    // —— Actions ——
    match key.code {
        Char('q') | Esc => app.should_quit = true,

        Char('m') => {
            app.mark_duplicates();
            let n = app.marked.len();
            app.set_status(format!("Marked {} files in group", n));
        }
        Char('M') => {
            app.unmark_group();
            let n = app.marked.len();
            app.set_status(format!("Unmarked files in group · {} remain marked", n));
        }
        Char('u') => {
            app.clear_marks();
            app.set_status("Cleared all marks".to_string());
        }
        Char('a') => {
            app.mark_all_duplicates();
            let n = app.marked.len();
            app.set_status(format!("Marked {} files in all groups", n));
        }

        Char('d') => {
            if !app.marked.is_empty() {
                app.confirm_delete = true;
            } else {
                app.set_status("No files marked — hold Space & scroll or use [m]/[a]".to_string());
            }
        }
        Char('D') => {
            app.mark_all_duplicates();
            if !app.marked.is_empty() {
                app.confirm_delete = true;
            } else {
                app.set_status("No duplicates to delete".to_string());
            }
        }

        Enter => app.open_dir(),
        Char('o') => app.open_file(),
        Char('r') => {
            app.set_status("Re-scan not yet implemented — restart the app".to_string());
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Mouse dispatch
// ---------------------------------------------------------------------------

fn handle_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    match m.kind {
        MouseEventKind::ScrollDown => app.move_selection(1),
        MouseEventKind::ScrollUp => app.move_selection(-1),
        MouseEventKind::Down(button) => {
            // Left click — attempt to select the file at the clicked row.
            // The terminal reports absolute row — we need to subtract the header
            // and border offsets, then account for scroll.
            // Layout: header (2 lines), content (top border 1 line), then text.
            if matches!(button, crossterm::event::MouseButton::Left) && m.row >= 2 {
                let content_line = (m.row as usize)
                    .saturating_sub(2) // skip header
                    .saturating_sub(1); // skip content top border
                let text_row = content_line + app.scroll_offset;
                if let Some(flat_idx) = app.row_to_file(text_row) {
                    app.selected_idx = flat_idx;
                    app.update_selected_row();
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

pub fn format_bytes(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", size)
    } else {
        format!("{:.1} {}", value, UNITS[unit_idx])
    }
}
