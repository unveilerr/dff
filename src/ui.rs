use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::*;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Gauge, Paragraph};

use crate::app::{App, format_bytes};

use std::sync::LazyLock;
use std::time::Duration;

// ── Theme-aware colour helpers ───────────────────────────────────────────────
// Uses only ANSI terminal colours so the tool automatically fits any
// terminal colour scheme (light / dark / …).

mod clr {
    use ratatui::prelude::Color;
    pub const BG: Color = Color::Reset;
    pub const TEXT: Color = Color::Reset;
    pub const ACCENT: Color = Color::Cyan;
    pub const HEADING: Color = Color::Yellow;
    pub const MARK: Color = Color::Red;
    pub const OK: Color = Color::Green;
    pub const DIM: Color = Color::DarkGray;
    pub const BRIGHT: Color = Color::White;
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    if area.height < 10 {
        let msg = Paragraph::new("Terminal too small — resize to at least 10 rows")
            .style(Style::default().fg(clr::MARK))
            .centered();
        frame.render_widget(msg, area);
        return;
    }

    let layout = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Min(1),    // main content
        Constraint::Length(1), // status bar
        Constraint::Length(1), // footer
    ]);
    let [header_area, content_area, status_area, footer_area] = layout.areas(area);

    render_header(frame, header_area, app);
    render_content(frame, content_area, app);
    render_status(frame, status_area, app);
    render_footer(frame, footer_area, app);

    if app.confirm_delete {
        render_confirm_overlay(frame, area, app);
    }
}

// ── Header ───────────────────────────────────────────────────────────────────

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = format!(" ⚡ DFF v{} ", env!("CARGO_PKG_VERSION"));

    let summary = if app.scanning {
        format!(" {} {} ", app.spinner_char(), app.scan_progress)
    } else if app.groups.is_empty() {
        "  ✓ No duplicates found  ".to_string()
    } else {
        let total = app.total_files_in_groups();
        let wasted = format_bytes(app.total_wasted());
        format!(
            "  {} groups · {} files · {} wasted  ",
            app.total_groups(),
            total,
            wasted,
        )
    };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            &title,
            Style::default()
                .fg(clr::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│", Style::default().fg(clr::DIM)),
        Span::styled(&summary, Style::default().fg(clr::TEXT)),
    ]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(clr::DIM)),
    );

    frame.render_widget(header, area);
}

// ── Content area ─────────────────────────────────────────────────────────────

fn render_content(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.scanning {
        render_scanning(frame, area, app);
        return;
    }

    if app.groups.is_empty() {
        let msg = Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("✓", Style::default().fg(clr::OK)),
            Span::raw(" No duplicate files found — the directory is clean"),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(clr::DIM)),
        );
        frame.render_widget(msg, area);
        return;
    }

    // ── Build all rendered lines ──────────────────────────────────────────────
    let max_path_w = area.width.saturating_sub(34) as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(app.total_rows().min(2000));
    let mut base_idx: usize = 0;

    for (gi, group) in app.groups.iter().enumerate() {
        let copies = group.files.len();
        let size_per = format_bytes(group.files[0].size);
        let wasted = format_bytes(group.wasted_bytes);
        let has_marked = (base_idx..base_idx + copies).any(|i| app.marked.contains(&i));

        // ── group header ─────────────────────────────────────────────────────
        let header = Line::from(vec![
            Span::styled(
                format!(" {:>2}. ", gi + 1),
                Style::default().fg(clr::DIM).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_mid(&group.file_name, max_path_w),
                Style::default()
                    .fg(clr::HEADING)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} × {}", copies, size_per),
                Style::default().fg(clr::DIM),
            ),
            Span::styled(
                format!("  ({} wasted)", wasted),
                Style::default().fg(if has_marked { clr::MARK } else { clr::DIM }),
            ),
        ]);
        lines.push(header);

        // ── file entries ─────────────────────────────────────────────────────
        for (fi, file) in group.files.iter().enumerate() {
            let flat_idx = base_idx + fi;
            let is_selected = flat_idx == app.selected_idx;
            let is_marked = app.marked.contains(&flat_idx);

            let display_path = truncate_path_lossy(&file.path, max_path_w);
            let fsize = format_bytes(file.size);

            let mut spans: Vec<Span> = Vec::new();

            // Marker / selection indicator
            if is_selected && is_marked {
                spans.push(Span::styled(
                    " ◉",
                    Style::default().fg(clr::MARK).add_modifier(Modifier::BOLD),
                ));
            } else if is_selected {
                spans.push(Span::styled(
                    " ▸",
                    Style::default()
                        .fg(clr::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ));
            } else if is_marked {
                spans.push(Span::styled(" ◉", Style::default().fg(clr::MARK)));
            } else {
                spans.push(Span::raw("  "));
            }

            // Path
            let path_style = if is_selected && is_marked {
                Style::default()
                    .fg(clr::BRIGHT)
                    .bg(clr::MARK)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(clr::BRIGHT)
                    .add_modifier(Modifier::REVERSED)
            } else if is_marked {
                Style::default().fg(clr::MARK).add_modifier(Modifier::DIM)
            } else {
                Style::default().fg(clr::TEXT)
            };
            spans.push(Span::styled(format!(" {}", display_path), path_style));

            // File size (always shown now)
            let size_style = if is_selected {
                Style::default().fg(clr::DIM).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(clr::DIM)
            };
            spans.push(Span::styled(format!("  {}", fsize), size_style));

            lines.push(Line::from(spans));
        }

        // Separator between groups
        if gi < app.groups.len() - 1 {
            lines.push(Line::from(Span::styled(
                "  ─────────────────────────────────────────────────────────────",
                Style::default().fg(clr::DIM),
            )));
        }

        base_idx += copies;
    }

    // ── Scroll indicator ─────────────────────────────────────────────────────
    let total = app.flat_count();
    let pct = if total > 0 {
        (app.selected_idx + 1) * 100 / total
    } else {
        0
    };
    let scroll_line = Line::from(Span::styled(
        format!(
            "  {}  {}% · file {} of {}",
            "─".repeat(30.min(area.width.saturating_sub(25) as usize)),
            pct,
            app.selected_idx + 1,
            total,
        ),
        Style::default().fg(clr::DIM),
    ));
    lines.push(scroll_line);

    // ── Scrolling ────────────────────────────────────────────────────────────
    let content_h = area.height.saturating_sub(4) as usize;
    let sel_row = app.selected_row.min(lines.len().saturating_sub(1));

    if sel_row < app.scroll_offset {
        app.scroll_offset = sel_row;
    } else if sel_row >= app.scroll_offset + content_h {
        app.scroll_offset = sel_row.saturating_add(1).saturating_sub(content_h);
    }
    let scroll = app.scroll_offset.min(lines.len().saturating_sub(1));

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(clr::DIM));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll as u16, 0));

    frame.render_widget(paragraph, area);
}

// ── Scanning screen ──────────────────────────────────────────────────────────

fn render_scanning(frame: &mut Frame, area: Rect, app: &App) {
    let centered = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(7),
        Constraint::Fill(1),
    ])
    .areas::<3>(area);

    let block = Block::default()
        .title(" Scanning ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(clr::ACCENT));

    let inner = block.inner(centered[1]);
    frame.render_widget(block, centered[1]);

    // ── Spinner + message ─────────────────────────────────────────────────────
    let msg = Line::from(vec![
        Span::styled(
            format!(" {} ", app.spinner_char()),
            Style::default()
                .fg(clr::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(&app.scan_progress, Style::default().fg(clr::TEXT)),
    ]);

    let msg_area = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas::<4>(inner);

    frame.render_widget(Paragraph::new(msg).centered(), msg_area[1]);

    // ── Progress gauge ────────────────────────────────────────────────────────
    let gauge_area = msg_area[3];
    let g_inner = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas::<3>(gauge_area);

    // Try to extract a percentage from the progress string
    let pct = if let Some(pct_start) = app.scan_progress.find('(') {
        if let Some(pct_end) = app.scan_progress.find("%)") {
            app.scan_progress[pct_start + 1..pct_end]
                .parse::<f64>()
                .ok()
        } else {
            None
        }
    } else {
        None
    };

    if let Some(pct_val) = pct {
        let gauge = Gauge::default()
            .ratio(pct_val / 100.0)
            .label(format!(" {}% ", pct_val as u64))
            .gauge_style(Style::default().fg(clr::ACCENT).bg(clr::DIM));
        frame.render_widget(gauge, g_inner[1]);
    } else if app.scan_progress.starts_with("Walking") {
        let gauge = Gauge::default()
            .label(" scanning… ")
            .gauge_style(Style::default().fg(clr::ACCENT).bg(clr::DIM));
        frame.render_widget(gauge, g_inner[1]);
    }
}

// ── Status bar ───────────────────────────────────────────────────────────────

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let msg = app
        .status_message
        .as_ref()
        .map(|(m, _)| m.as_str())
        .unwrap_or("");

    let expired = app
        .status_message
        .as_ref()
        .map(|(_, t)| t.elapsed() > Duration::from_secs(8))
        .unwrap_or(true);

    // Build status text
    let status = if app.scanning {
        " Scanning…  [q] quit ".to_string()
    } else if !expired && !msg.is_empty() {
        msg.to_string()
    } else {
        let n = app.marked.len();
        format!(
            " {} marked · [Space] hold+scroll · [m] group · [a] all · [d] delete · [D] all+del · [q] quit ",
            n,
        )
    };

    let fg = if status.starts_with('✓') {
        clr::OK
    } else if status.starts_with('✗') {
        clr::MARK
    } else {
        clr::DIM
    };

    let status_ref: &str = Box::leak(status.into_boxed_str());

    let bar = Paragraph::new(Line::from(Span::styled(
        status_ref.trim(),
        Style::default().fg(fg),
    )))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(clr::DIM)),
    );

    frame.render_widget(bar, area);
}

// ── Footer (keybindings help) ────────────────────────────────────────────────

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    if app.scanning {
        return;
    }

    let keys = Line::from(vec![
        key_binding("↑/↓", "nav"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("Space", "hold+scroll"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("m/M", "mark/unmark group"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("a", "all"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("d", "delete"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("D", "all+del"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("Enter", "dir"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("o", "open"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("g/G", "↕"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("u", "unmark all"),
        Span::styled(" · ", Style::default().fg(clr::DIM)),
        key_binding("q", "quit"),
    ]);

    let footer = Paragraph::new(keys).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(clr::DIM)),
    );

    frame.render_widget(footer, area);
}

fn key_binding(key: &str, action: &str) -> Span<'static> {
    Span::styled(
        format!("[{}] {}", key, action),
        Style::default().fg(clr::DIM),
    )
}

// ── Confirm-delete overlay (opaque) ─────────────────────────────────────────

fn render_confirm_overlay(frame: &mut Frame, area: Rect, app: &App) {
    let total_marked_size: u64 = app
        .marked
        .iter()
        .filter_map(|&idx| app.flat_to_group_file(idx))
        .map(|(gi, fi)| app.groups[gi].files[fi].size)
        .sum();

    let count = app.marked.len();
    let has_primary = app.has_primary_marked();
    let overlay_w = 56.min(area.width.saturating_sub(4));
    let overlay_h = if has_primary { 11 } else { 9 };

    let vert = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(overlay_h),
        Constraint::Fill(1),
    ]);
    let horiz = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(overlay_w),
        Constraint::Fill(1),
    ]);
    let [_, vert_center, _] = vert.areas(area);
    let [_, overlay_area, _] = horiz.areas(vert_center);

    // ── Full-screen opaque fill ────────────────────────────────────────────
    //  Clear the ENTIRE screen area so content behind doesn't show through.
    frame.render_widget(Block::default().style(Style::default().bg(clr::BG)), area);

    // ── Overlay border ──────────────────────────────────────────────────────
    let block = Block::default()
        .title(" ⚠ Confirm Delete ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(clr::MARK))
        .style(Style::default().bg(clr::BG));

    let inner = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    // ── Inner layout ─────────────────────────────────────────────────────────
    let text_area = Layout::vertical([
        Constraint::Length(1), // 0: file count
        Constraint::Length(1), // 1: size + "cannot be undone"
        Constraint::Length(1), // 2: primary-copy warning (or blank)
        Constraint::Length(1), // 3: blank
        Constraint::Length(1), // 4: Yes / No
        Constraint::Length(1), // 5: hint
    ])
    .areas::<6>(inner);

    // File count
    let label1 = Line::from(vec![Span::styled(
        format!(
            " You are about to delete {} file{} ",
            count,
            if count == 1 { "" } else { "s" }
        ),
        Style::default().fg(clr::TEXT),
    )]);
    frame.render_widget(
        Paragraph::new(label1)
            .centered()
            .style(Style::default().bg(clr::BG)),
        text_area[0],
    );

    // Size
    let label2 = Line::from(vec![
        Span::styled(
            format!(" {} will be freed. ", format_bytes(total_marked_size)),
            Style::default().fg(clr::MARK),
        ),
        Span::styled("This cannot be undone!", Style::default().fg(clr::DIM)),
    ]);
    frame.render_widget(
        Paragraph::new(label2)
            .centered()
            .style(Style::default().bg(clr::BG)),
        text_area[1],
    );

    // Primary-copy warning (if any marked file is the sole remaining copy)
    if has_primary {
        let warn = Line::from(vec![
            Span::styled(
                " ⚠ ",
                Style::default().fg(clr::MARK).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Warning: ",
                Style::default().fg(clr::MARK).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "marked set includes the last remaining copy of a file ",
                Style::default().fg(clr::MARK),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(warn)
                .centered()
                .style(Style::default().bg(clr::BG)),
            text_area[2],
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from("")).style(Style::default().bg(clr::BG)),
            text_area[2],
        );
    }

    // Blank
    frame.render_widget(
        Paragraph::new(Line::from("")).style(Style::default().bg(clr::BG)),
        text_area[3],
    );

    // Options
    let label3 = Line::from(vec![
        Span::styled(
            "  [Y]es, delete  ",
            Style::default().fg(clr::MARK).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  [N]o, cancel  ",
            Style::default().fg(clr::OK).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  [Esc]  ", Style::default().fg(clr::DIM)),
    ]);
    frame.render_widget(
        Paragraph::new(label3)
            .centered()
            .style(Style::default().bg(clr::BG)),
        text_area[4],
    );

    // Hint
    let hint = Line::from(Span::styled(
        " You can also press Enter as a shortcut for Yes ",
        Style::default().fg(clr::DIM),
    ));
    frame.render_widget(
        Paragraph::new(hint)
            .centered()
            .style(Style::default().bg(clr::BG)),
        text_area[5],
    );
}

// ── Path truncation ──────────────────────────────────────────────────────────

static HOME_DIR: LazyLock<String> = LazyLock::new(|| std::env::var("HOME").unwrap_or_default());

fn truncate_path_lossy(path: &std::path::Path, max_len: usize) -> String {
    // Convert to string representation
    let s = path.to_string_lossy();
    if s.len() <= max_len || max_len < 15 {
        return s.chars().take(max_len).collect();
    }

    // Collapse $HOME → ~ using cached HOME
    let collapsed = s.replacen(&*HOME_DIR, "~", 1);
    if collapsed.len() <= max_len {
        return collapsed;
    }

    // Truncate middle
    let half = (max_len.saturating_sub(3)) / 2;
    if collapsed.len() > half * 2 + 3 {
        let bytes = collapsed.as_bytes();
        let mut start_end = half;
        while start_end > 0 && !bytes[start_end].is_ascii() {
            if bytes[start_end] & 0xC0 != 0x80 {
                break;
            }
            start_end -= 1;
        }
        let mut end_start = collapsed.len().saturating_sub(half);
        while end_start < collapsed.len() && !bytes[end_start].is_ascii() {
            if bytes[end_start] & 0xC0 != 0x80 {
                break;
            }
            end_start += 1;
        }
        format!("{}…{}", &collapsed[..start_end], &collapsed[end_start..])
    } else {
        collapsed
    }
}

fn truncate_mid(s: &str, max_len: usize) -> String {
    if s.len() <= max_len || max_len < 5 {
        return s.chars().take(max_len).collect();
    }
    let half = (max_len - 3) / 2;
    let a: String = s.chars().take(half).collect();
    let b: String = s.chars().rev().take(half).collect();
    format!("{}…{}", a, b.chars().rev().collect::<String>())
}
