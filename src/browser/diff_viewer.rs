//! Two-file diff viewer (Ctrl+D): full-screen side-by-side comparison with aligned rows,
//! patience line diff ([`similar`]), synchronized vertical scroll, and gap padding on the
//! opposite pane for insert/delete blocks.
//!
//! With **no** marks, Ctrl+D compares the **current directories** of both panels and shows
//! `C ` / `S ` / `X ` prefixes (like the `> ` mark column) until either panel changes folder.

use std::collections::HashMap;
use std::io;
use std::mem::take;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use similar::{Algorithm, ChangeTag, DiffOp, TextDiff};

use crate::app::events::AppAction;
use crate::app::state::AppState;
use crate::browser::panel::PanelOperations;
use crate::core::file_ops::{FileInfo, FileOperations};
use crate::core::location::PanelLocation;
use crate::core::panel_backend;
use crate::ui::theme::DiffViewerPalette;
use crate::util;

/// One marked file resolved for diff I/O: [`panel_backend::read_file`] needs `location` + `name`;
/// `display_path` is shown in the UI header.
#[derive(Clone)]
struct DiffMarkedSource {
    location: PanelLocation,
    name: String,
    display_path: String,
}

/// Exactly two marked non-directory files in diff order: **left** = old/first pane, **right** = new/second.
struct DiffPairSources {
    left: DiffMarkedSource,
    right: DiffMarkedSource,
}

pub enum DiffViewerState {
    Loading {
        left_path: String,
        right_path: String,
        rx: mpsc::Receiver<io::Result<DiffViewerReady>>,
        /// Set when closing during load so background reads stop cooperatively (same pattern as F3 viewer).
        cancel: Arc<AtomicBool>,
    },
    Ready(DiffViewerReady),
}

/// Panel-directory compare (Ctrl+X D with no marks): background work; Esc sets `cancel` and drops the receiver.
pub struct FolderComparePending {
    rx: mpsc::Receiver<FolderCompareState>,
    cancel: Arc<AtomicBool>,
    anchor_left: PanelLocation,
    anchor_right: PanelLocation,
}

impl FolderComparePending {
    fn stale_against(
        &self,
        left: &PanelLocation,
        right: &PanelLocation,
    ) -> bool {
        &self.anchor_left != left || &self.anchor_right != right
    }
}

pub struct DiffViewerReady {
    pub left_path: String,
    pub right_path: String,
    aligned: Vec<AlignedRow>,
    /// Column inner widths (full width for line number + separator + text).
    cached_widths: (u16, u16),
    /// Right-aligned source line numbers (1-based); width in columns.
    line_num_width: usize,
    left_display: Vec<Line<'static>>,
    right_display: Vec<Line<'static>>,
    pub scroll: usize,
    pub area: Rect,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Equal,
    OnlyLeft,
    OnlyRight,
    Changed,
}

#[derive(Clone)]
struct AlignedRow {
    left: String,
    right: String,
    kind: RowKind,
    /// 1-based line in left file; None when this side has no source line.
    old_line: Option<usize>,
    /// 1-based line in right file.
    new_line: Option<usize>,
}

pub fn close_diff_viewer(app: &mut AppState) {
    if let Some(DiffViewerState::Loading { cancel, .. }) = app.diff_viewer_screen.as_ref() {
        cancel.store(true, Ordering::Relaxed);
    }
    app.diff_viewer_screen = None;
}

/// Stop an in-flight panel-directory compare (Esc); background thread may still exit shortly.
pub fn cancel_folder_compare_pending(app: &mut AppState) {
    if let Some(p) = app.folder_compare_pending.take() {
        p.cancel.store(true, Ordering::Relaxed);
    }
}

fn read_panel_file_bytes_cancellable(
    loc: &PanelLocation,
    name: &str,
    cancel: &AtomicBool,
) -> io::Result<Vec<u8>> {
    match loc {
        PanelLocation::Fs(p) => {
            let path = FileOperations::join_path(p, name);
            util::read_path_chunked(&path, cancel)
        }
        _ => {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "compare cancelled",
                ));
            }
            panel_backend::read_file(loc, name)
        }
    }
}

/// Prefix column for folder compare (Ctrl+D with no marks): same size, different bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FolderDiffTag {
    ContentDiff,
    SizeDiff,
    /// Entry exists only on this panel’s side.
    AbsentOnOther,
}

/// Result of comparing left vs right panel listings; cleared when either [`PanelLocation`] changes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FolderCompareState {
    pub anchor_left: PanelLocation,
    pub anchor_right: PanelLocation,
    pub left_tags: HashMap<String, FolderDiffTag>,
    pub right_tags: HashMap<String, FolderDiffTag>,
}

impl FolderCompareState {
    pub fn tag_for_entry(
        &self,
        is_left: bool,
        file: &FileInfo,
    ) -> Option<FolderDiffTag> {
        if file.is_parent_dir() {
            return None;
        }
        let name = file.name.trim_end_matches('/');
        let map = if is_left {
            &self.left_tags
        } else {
            &self.right_tags
        };
        map.get(name).copied()
    }
}

/// Drop folder-compare prefixes if either panel is no longer at the directory where compare ran.
/// Also cancels an in-flight background compare when either panel cwd no longer matches the run.
pub fn clear_folder_compare_if_stale(app: &mut AppState) {
    let left = app.left_panel().current_location().clone();
    let right = app.right_panel().current_location().clone();
    if let Some(fc) = app.folder_compare.as_ref() {
        if fc.anchor_left != left || fc.anchor_right != right {
            app.folder_compare = None;
        }
    }
    if app
        .folder_compare_pending
        .as_ref()
        .is_some_and(|p| p.stale_against(&left, &right))
    {
        cancel_folder_compare_pending(app);
    }
}

/// Compare left and right panel directories on a background thread (Esc cancels; see [`poll_folder_compare_pending`]).
/// Clears marks and any previous [`AppState::folder_compare`] for this run; sets [`AppState::folder_compare_pending`].
pub fn start_compare_panel_directories(app: &mut AppState) {
    cancel_folder_compare_pending(app);
    clear_folder_compare_if_stale(app);
    app.folder_compare = None;
    app.left_panel_mut().clear_marks();
    app.right_panel_mut().clear_marks();

    let anchor_left = app.left_panel().current_location().clone();
    let anchor_right = app.right_panel().current_location().clone();
    let loc_left = anchor_left.clone();
    let loc_right = anchor_right.clone();
    let left_files = app.left_panel().get_files().to_vec();
    let right_files = app.right_panel().get_files().to_vec();

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_t = Arc::clone(&cancel);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let Some(state) = compute_folder_compare_state(
            loc_left,
            loc_right,
            &left_files,
            &right_files,
            &cancel_t,
        ) {
            let _ = tx.send(state);
        }
    });
    app.folder_compare_pending = Some(FolderComparePending {
        rx,
        cancel,
        anchor_left,
        anchor_right,
    });
}

fn compute_folder_compare_state(
    left_loc: PanelLocation,
    right_loc: PanelLocation,
    left_files: &[FileInfo],
    right_files: &[FileInfo],
    cancel: &AtomicBool,
) -> Option<FolderCompareState> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let left_map = collect_name_to_file(left_files);
    let right_map = collect_name_to_file(right_files);

    let mut left_tags = HashMap::with_capacity(left_map.len());
    let mut right_tags = HashMap::with_capacity(right_map.len());

    for (name, lf) in &left_map {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match right_map.get(name.as_str()) {
            None => {
                left_tags.insert(name.clone(), FolderDiffTag::AbsentOnOther);
            }
            Some(rf) => {
                if lf.is_dir && rf.is_dir {
                    continue;
                }
                if lf.is_dir != rf.is_dir {
                    left_tags.insert(name.clone(), FolderDiffTag::SizeDiff);
                    right_tags.insert(name.clone(), FolderDiffTag::SizeDiff);
                    continue;
                }
                if lf.size != rf.size {
                    left_tags.insert(name.clone(), FolderDiffTag::SizeDiff);
                    right_tags.insert(name.clone(), FolderDiffTag::SizeDiff);
                    continue;
                }
                let bl = read_panel_file_bytes_cancellable(&left_loc, &lf.name, cancel);
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                let br = read_panel_file_bytes_cancellable(&right_loc, &rf.name, cancel);
                match (bl, br) {
                    (Ok(a), Ok(b)) if a == b => {}
                    (Ok(_), Ok(_)) => {
                        left_tags.insert(name.clone(), FolderDiffTag::ContentDiff);
                        right_tags.insert(name.clone(), FolderDiffTag::ContentDiff);
                    }
                    _ => {
                        left_tags.insert(name.clone(), FolderDiffTag::SizeDiff);
                        right_tags.insert(name.clone(), FolderDiffTag::SizeDiff);
                    }
                }
            }
        }
    }

    for name in right_map.keys() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if !left_map.contains_key(name) {
            right_tags.insert(name.clone(), FolderDiffTag::AbsentOnOther);
        }
    }

    Some(FolderCompareState {
        anchor_left: left_loc,
        anchor_right: right_loc,
        left_tags,
        right_tags,
    })
}

/// Apply a completed folder compare from the background thread; returns true if UI should redraw.
pub fn poll_folder_compare_pending(app: &mut AppState) -> bool {
    let pending = match app.folder_compare_pending.take() {
        None => return false,
        Some(p) => p,
    };
    match pending.rx.try_recv() {
        Ok(state) => {
            let apply = app.left_panel().current_location() == &pending.anchor_left
                && app.right_panel().current_location() == &pending.anchor_right;
            if apply {
                app.folder_compare = Some(state);
            }
            true
        }
        Err(TryRecvError::Empty) => {
            app.folder_compare_pending = Some(pending);
            false
        }
        Err(TryRecvError::Disconnected) => true,
    }
}

fn collect_name_to_file(files: &[FileInfo]) -> HashMap<String, &FileInfo> {
    let mut m = HashMap::with_capacity(files.len());
    for f in files {
        if f.is_parent_dir() {
            continue;
        }
        m.insert(
            f.name.trim_end_matches('/').to_string(),
            f,
        );
    }
    m
}

/// Number of marked non-directory entries across both panels (for Ctrl+D hints).
pub fn marked_non_dir_file_count(app: &AppState) -> usize {
    let mut n = 0;
    for panel in [app.left_panel(), app.right_panel()] {
        for idx in panel.iter_marked_indices() {
            if let Some(f) = panel.get_files().get(idx) {
                if !f.is_dir && !f.is_parent_dir() {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Two marked non-directory files in stable order: left panel first (by index), then right.
fn two_marked_files(app: &AppState) -> Option<DiffPairSources> {
    #[derive(Clone)]
    struct SortEntry {
        panel: u8,
        idx: usize,
        source: DiffMarkedSource,
    }
    let mut v: Vec<SortEntry> = Vec::new();
    for (panel_id, panel) in [(0u8, app.left_panel()), (1u8, app.right_panel())] {
        let loc = panel.current_location().clone();
        for idx in panel.iter_marked_indices() {
            if let Some(f) = panel.get_files().get(idx) {
                if !f.is_dir && !f.is_parent_dir() {
                    let display_path = panel_backend::join_path_display(&loc, &f.name);
                    v.push(SortEntry {
                        panel: panel_id,
                        idx,
                        source: DiffMarkedSource {
                            location: loc.clone(),
                            name: f.name.clone(),
                            display_path,
                        },
                    });
                }
            }
        }
    }
    if v.len() != 2 {
        return None;
    }
    v.sort_by(|a, b| (a.panel, a.idx).cmp(&(b.panel, b.idx)));
    let e0 = v[0].source.clone();
    let e1 = v[1].source.clone();
    Some(DiffPairSources {
        left: e0,
        right: e1,
    })
}

/// Reads both files, builds line lists and patience diff on a worker thread so the UI thread stays responsive to Esc.
fn diff_worker_build_ready(
    loc_l: PanelLocation,
    name_l: String,
    loc_r: PanelLocation,
    name_r: String,
    left_path: String,
    right_path: String,
    cancel: &AtomicBool,
) -> io::Result<DiffViewerReady> {
    let left_bytes = read_panel_file_bytes_cancellable(&loc_l, &name_l, cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "diff cancelled",
        ));
    }
    let right_bytes = read_panel_file_bytes_cancellable(&loc_r, &name_r, cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "diff cancelled",
        ));
    }
    let old_lines = logical_lines_from_bytes_cancellable(&left_bytes, cancel)?;
    let new_lines = logical_lines_from_bytes_cancellable(&right_bytes, cancel)?;
    let aligned = build_aligned_rows_cancellable(&old_lines, &new_lines, cancel)?;
    let line_num_width = line_number_column_width(&aligned);
    Ok(DiffViewerReady {
        left_path,
        right_path,
        aligned,
        cached_widths: (0, 0),
        line_num_width,
        left_display: Vec::new(),
        right_display: Vec::new(),
        scroll: 0,
        area: Rect::default(),
    })
}

/// Open diff viewer when exactly two marked files exist. Returns `false` if not opened (caller shows toast).
pub fn try_open_diff(app: &mut AppState) -> bool {
    let n = marked_non_dir_file_count(app);
    if n != 2 {
        return false;
    }
    let Some(pair) = two_marked_files(app) else {
        return false;
    };
    let loc_l = pair.left.location.clone();
    let name_l = pair.left.name.clone();
    let loc_r = pair.right.location.clone();
    let name_r = pair.right.name.clone();
    let left_path = pair.left.display_path;
    let right_path = pair.right.display_path;
    let left_path_worker = left_path.clone();
    let right_path_worker = right_path.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = Arc::clone(&cancel);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = diff_worker_build_ready(
            loc_l,
            name_l,
            loc_r,
            name_r,
            left_path_worker,
            right_path_worker,
            &cancel_thread,
        );
        let _ = tx.send(res);
    });
    app.diff_viewer_screen = Some(DiffViewerState::Loading {
        left_path,
        right_path,
        rx,
        cancel,
    });
    true
}

pub fn poll_diff_loading(app: &mut AppState) -> bool {
    let taken = take(&mut app.diff_viewer_screen);
    let Some(DiffViewerState::Loading {
        left_path,
        right_path,
        rx,
        cancel,
    }) = taken
    else {
        app.diff_viewer_screen = taken;
        return false;
    };
    match rx.try_recv() {
        Ok(Ok(ready)) => {
            let _ = (left_path, right_path);
            app.diff_viewer_screen = Some(DiffViewerState::Ready(ready));
            true
        }
        Ok(Err(_)) => {
            app.diff_viewer_screen = None;
            true
        }
        Err(TryRecvError::Empty) => {
            app.diff_viewer_screen = Some(DiffViewerState::Loading {
                left_path,
                right_path,
                rx,
                cancel,
            });
            false
        }
        Err(TryRecvError::Disconnected) => {
            app.diff_viewer_screen = None;
            true
        }
    }
}

fn safe_text_char(c: char) -> bool {
    c == '\n' || (c.is_ascii() && c >= ' ' && c <= '~')
}

fn sanitize_text_for_display_cancellable(
    s: &str,
    cancel: &AtomicBool,
) -> io::Result<String> {
    let mut out = String::with_capacity(s.len());
    let mut n = 0usize;
    for c in s.chars() {
        if n % 65_536 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "diff cancelled",
            ));
        }
        n += 1;
        match c {
            '\r' => {}
            '\t' => out.push_str("    "),
            '\n' => out.push('\n'),
            _ if safe_text_char(c) => out.push(c),
            _ => out.push('.'),
        }
    }
    Ok(out)
}

fn logical_lines_from_bytes_cancellable(
    content: &[u8],
    cancel: &AtomicBool,
) -> io::Result<Vec<String>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "diff cancelled",
        ));
    }
    let s = String::from_utf8_lossy(content);
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "diff cancelled",
        ));
    }
    let sanitized = sanitize_text_for_display_cancellable(&s, cancel)?;
    let mut lines: Vec<String> = Vec::new();
    let mut line_idx = 0usize;
    for line in sanitized.lines() {
        if line_idx % 8192 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "diff cancelled",
            ));
        }
        lines.push(line.to_string());
        line_idx += 1;
    }
    if !content.is_empty() && !s.ends_with('\n') {
        if lines.is_empty() {
            lines.push(String::new());
        }
    } else if content.ends_with(b"\n") && s.ends_with('\n') {
        lines.push(String::new());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    Ok(lines)
}

fn wrap_line(
    line: &str,
    width: usize,
) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut s = line;
    while !s.is_empty() {
        let chunk_char_count = s.chars().take(width).count();
        let (chunk, rest) = if chunk_char_count < s.chars().count() {
            let idx = s
                .char_indices()
                .nth(chunk_char_count)
                .map(|(i, _)| i)
                .unwrap_or(s.len());
            s.split_at(idx)
        } else {
            (s, "")
        };
        out.push(chunk.to_string());
        s = rest;
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// If `ops[delete_idx]` is followed by an `Insert`, returns that insert’s `(new_index, new_len)`.
fn insert_run_following_delete(
    ops: &[DiffOp],
    delete_idx: usize,
) -> Option<(usize, usize)> {
    match ops.get(delete_idx + 1)? {
        DiffOp::Insert {
            new_index, new_len, ..
        } => Some((*new_index, *new_len)),
        _ => None,
    }
}

fn push_equal_run(
    out: &mut Vec<AlignedRow>,
    old: &[String],
    new: &[String],
    old_base: usize,
    new_base: usize,
    len: usize,
) {
    for j in 0..len {
        let ol = old_base + j + 1;
        let nl = new_base + j + 1;
        out.push(AlignedRow {
            left: old[old_base + j].clone(),
            right: new[new_base + j].clone(),
            kind: RowKind::Equal,
            old_line: Some(ol),
            new_line: Some(nl),
        });
    }
}

/// Left-only deleted lines (gap on the right column).
fn push_delete_only_run(
    out: &mut Vec<AlignedRow>,
    old: &[String],
    old_base: usize,
    old_len: usize,
) {
    for j in 0..old_len {
        let ol = old_base + j + 1;
        out.push(AlignedRow {
            left: old[old_base + j].clone(),
            right: String::new(),
            kind: RowKind::OnlyLeft,
            old_line: Some(ol),
            new_line: None,
        });
    }
}

/// Right-only inserted lines (gap on the left column).
fn push_insert_only_run(
    out: &mut Vec<AlignedRow>,
    new: &[String],
    new_base: usize,
    new_len: usize,
) {
    for j in 0..new_len {
        let nl = new_base + j + 1;
        out.push(AlignedRow {
            left: String::new(),
            right: new[new_base + j].clone(),
            kind: RowKind::OnlyRight,
            old_line: None,
            new_line: Some(nl),
        });
    }
}

/// Extends `aligned_rows` with one side-by-side block: paired lines become [`RowKind::Changed`];
/// any extra lines on one side become [`RowKind::OnlyLeft`] / [`RowKind::OnlyRight`] (gaps).
/// Used for `Replace`, and for adjacent `Delete`+`Insert` (Meld-style alignment).
fn extend_aligned_rows_with_paired_sides(
    aligned_rows: &mut Vec<AlignedRow>,
    old: &[String],
    new: &[String],
    old_base: usize,
    old_len: usize,
    new_base: usize,
    new_len: usize,
) {
    let pairs = old_len.min(new_len);
    for j in 0..pairs {
        let ol = old_base + j + 1;
        let nl = new_base + j + 1;
        aligned_rows.push(AlignedRow {
            left: old[old_base + j].clone(),
            right: new[new_base + j].clone(),
            kind: RowKind::Changed,
            old_line: Some(ol),
            new_line: Some(nl),
        });
    }
    for j in pairs..old_len {
        let ol = old_base + j + 1;
        aligned_rows.push(AlignedRow {
            left: old[old_base + j].clone(),
            right: String::new(),
            kind: RowKind::OnlyLeft,
            old_line: Some(ol),
            new_line: None,
        });
    }
    for j in pairs..new_len {
        let nl = new_base + j + 1;
        aligned_rows.push(AlignedRow {
            left: String::new(),
            right: new[new_base + j].clone(),
            kind: RowKind::OnlyRight,
            old_line: None,
            new_line: Some(nl),
        });
    }
}

#[rustfmt::skip]
fn build_aligned_rows_cancellable(
    old: &[String],
    new: &[String],
    cancel: &AtomicBool,
) -> io::Result<Vec<AlignedRow>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "diff cancelled",
        ));
    }
    let old_r: Vec<&str> = old.iter().map(String::as_str).collect();
    let new_r: Vec<&str> = new.iter().map(String::as_str).collect();
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .diff_slices(&old_r, &new_r);
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "diff cancelled",
        ));
    }
    let mut out = Vec::new();
    let ops = diff.ops();
    let mut i = 0usize;
    let mut step = 0usize;
    while i < ops.len() {
        if step % 2048 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "diff cancelled",
            ));
        }
        step += 1;
        match ops[i] {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                push_equal_run(
                    &mut out,
                    old,
                    new,
                    old_index,
                    new_index,
                    len,
                );
                i += 1;
            }
            DiffOp::Delete {
                old_index,
                old_len,
                ..
            } => {
                if let Some((new_index, new_len)) = insert_run_following_delete(ops, i) {
                    extend_aligned_rows_with_paired_sides(
                        &mut out,
                        old,
                        new,
                        old_index,
                        old_len,
                        new_index,
                        new_len,
                    );
                    i += 2;
                } else {
                    push_delete_only_run(
                        &mut out,
                        old,
                        old_index,
                        old_len,
                    );
                    i += 1;
                }
            }
            DiffOp::Insert {
                new_index,
                new_len,
                ..
            } => {
                push_insert_only_run(
                    &mut out,
                    new,
                    new_index,
                    new_len,
                );
                i += 1;
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                extend_aligned_rows_with_paired_sides(
                    &mut out,
                    old,
                    new,
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                );
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Largest 1-based line index on one side (minimum 1 so width is never zero).
fn max_line_index(
    aligned: &[AlignedRow],
    line: impl Fn(&AlignedRow) -> Option<usize>,
) -> usize {
    let mut m = 1usize;
    for row in aligned {
        if let Some(n) = line(row) {
            m = m.max(n);
        }
    }
    m
}

fn line_number_column_width(aligned: &[AlignedRow]) -> usize {
    let max_old = max_line_index(aligned, |r| r.old_line);
    let max_new = max_line_index(aligned, |r| r.new_line);
    let merged = max_old.max(max_new);
    let m = merged.max(1);
    let log_digits = (m as f64).log10().floor() as usize;
    let w = log_digits + 1;
    let at_least_3 = w.max(3);
    at_least_3.min(8)
}

fn style_for_cell(
    kind: RowKind,
    side_left: bool,
    p: DiffViewerPalette,
) -> Style {
    let base = Style::default().bg(p.background).fg(p.text);
    match (kind, side_left) {
        (RowKind::Equal, _) => base,
        (RowKind::OnlyLeft, true) => Style::default().bg(p.removed_bg).fg(p.removed_fg),
        (RowKind::OnlyLeft, false) => Style::default().bg(p.gap_bg).fg(p.gap_fg),
        (RowKind::OnlyRight, false) => Style::default().bg(p.added_bg).fg(p.added_fg),
        (RowKind::OnlyRight, true) => Style::default().bg(p.gap_bg).fg(p.gap_fg),
        // Same highlight on both panes for modified lines (reference-style side-by-side diff).
        (RowKind::Changed, _) => Style::default().bg(p.changed_old_bg).fg(p.changed_old_fg),
    }
}

/// Padding for the opposite column when one side has no text: spaces + [`DiffViewerPalette::gap_bg`]
/// so rows stay aligned without a visible dash stripe.
fn gap_padding_spaces(width: usize) -> String {
    " ".repeat(width.max(1))
}

fn char_diff_segment_pairs(
    left: &str,
    right: &str,
    p: DiffViewerPalette,
) -> (Vec<(Style, String)>, Vec<(Style, String)>) {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .diff_chars(left, right);
    let base_l = style_for_cell(RowKind::Changed, true, p);
    let base_r = style_for_cell(RowKind::Changed, false, p);
    let del = Style::default().bg(p.char_removed_bg).fg(p.char_removed_fg);
    let ins = Style::default().bg(p.char_added_bg).fg(p.char_added_fg);
    let mut l = Vec::new();
    let mut r = Vec::new();
    for ch in diff.iter_all_changes() {
        let v = ch.value().to_string();
        if v.is_empty() {
            continue;
        }
        match ch.tag() {
            ChangeTag::Equal => {
                l.push((base_l, v.clone()));
                r.push((base_r, v));
            }
            ChangeTag::Delete => l.push((del, v)),
            ChangeTag::Insert => r.push((ins, v)),
        }
    }
    if l.is_empty() {
        l.push((base_l, String::new()));
    }
    if r.is_empty() {
        r.push((base_r, String::new()));
    }
    (l, r)
}

fn wrap_segments_to_lines(
    segments: Vec<(Style, String)>,
    width: usize,
) -> Vec<Line<'static>> {
    if width == 0 {
        let spans: Vec<Span> = segments
            .into_iter()
            .map(|(st, s)| Span::styled(s, st))
            .collect();
        return vec![Line::from(spans)];
    }
    let mut rows: Vec<Vec<Span<'static>>> = vec![];
    let mut cur: Vec<Span<'static>> = vec![];
    let mut cur_chars = 0usize;
    for (st, part) in segments {
        let mut rest: &str = part.as_str();
        while !rest.is_empty() {
            let room = width.saturating_sub(cur_chars);
            if room == 0 {
                rows.push(std::mem::take(&mut cur));
                cur_chars = 0;
                continue;
            }
            let take = rest.chars().count().min(room);
            let byte_end = rest
                .char_indices()
                .nth(take)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());
            let chunk = &rest[..byte_end];
            rest = &rest[byte_end..];
            cur.push(Span::styled(chunk.to_string(), st));
            cur_chars += chunk.chars().count();
            if cur_chars >= width && !rest.is_empty() {
                rows.push(std::mem::take(&mut cur));
                cur_chars = 0;
            }
        }
    }
    if !cur.is_empty() || rows.is_empty() {
        rows.push(cur);
    }
    rows.into_iter().map(Line::from).collect()
}

fn pad_line_to_cell_width(
    mut line: Line<'static>,
    target: usize,
    fill: Style,
) -> Line<'static> {
    let w = line.width();
    if w < target {
        line.push_span(Span::styled(" ".repeat(target - w), fill));
    }
    line
}

fn prepend_line_gutter(
    content: Line<'static>,
    num: Option<usize>,
    num_width: usize,
    continuation: bool,
    text_target_width: usize,
    fill_style: Style,
    p: DiffViewerPalette,
) -> Line<'static> {
    let padded = pad_line_to_cell_width(content, text_target_width, fill_style);
    let num_style = Style::default().bg(p.background).fg(p.line_number_fg);
    let sep_style = Style::default().bg(p.background).fg(p.column_border);
    let label = if continuation {
        " ".repeat(num_width)
    } else {
        match num {
            Some(n) => format!("{:>nw$}", n, nw = num_width),
            None => " ".repeat(num_width),
        }
    };
    let mut spans = vec![Span::styled(label, num_style), Span::styled("│", sep_style)];
    spans.extend(padded);
    Line::from(spans)
}

/// Usable text width inside one pane: full column width minus line-number gutter and the `│` column.
fn diff_column_text_width(
    column_inner_width: usize,
    line_num_width: usize,
) -> usize {
    const SEPARATOR_COLS: usize = 1;
    column_inner_width
        .saturating_sub(line_num_width + SEPARATOR_COLS)
        .max(1)
}

fn append_guttered_side_by_side_lines(
    left_display: &mut Vec<Line<'static>>,
    right_display: &mut Vec<Line<'static>>,
    left_content: Line<'static>,
    right_content: Line<'static>,
    old_line: Option<usize>,
    new_line: Option<usize>,
    line_num_width: usize,
    continuation_line: bool,
    text_lw: usize,
    text_rw: usize,
    left_fill: Style,
    right_fill: Style,
    p: DiffViewerPalette,
) {
    left_display.push(prepend_line_gutter(
        left_content,
        old_line,
        line_num_width,
        continuation_line,
        text_lw,
        left_fill,
        p,
    ));
    right_display.push(prepend_line_gutter(
        right_content,
        new_line,
        line_num_width,
        continuation_line,
        text_rw,
        right_fill,
        p,
    ));
}

/// One screen row of the opposite column should show the gap stripe (insert/delete alignment).
fn wrapped_fragment_needs_gap_stripe(
    for_left_column: bool,
    past_end_of_this_side_wrap: bool,
    this_fragment: &str,
    other_fragment: &str,
    row_kind: RowKind,
) -> bool {
    if past_end_of_this_side_wrap {
        return true;
    }
    if for_left_column {
        this_fragment.is_empty()
            && !other_fragment.is_empty()
            && matches!(row_kind, RowKind::OnlyRight)
    } else {
        this_fragment.is_empty()
            && !other_fragment.is_empty()
            && matches!(row_kind, RowKind::OnlyLeft)
    }
}

fn style_for_gap_stripe_or_row_cell(
    use_gap_stripe: bool,
    row_kind: RowKind,
    side_left: bool,
    p: DiffViewerPalette,
) -> Style {
    if use_gap_stripe {
        Style::default().bg(p.gap_bg).fg(p.gap_fg)
    } else {
        style_for_cell(row_kind, side_left, p)
    }
}

/// Fixed visual width: invisible gap padding, or fragment padded / clipped to `text_width` grapheme columns.
fn padded_or_gap_cell_string(
    use_gap_stripe: bool,
    fragment: &str,
    text_width: usize,
) -> String {
    if use_gap_stripe {
        gap_padding_spaces(text_width)
    } else if fragment.chars().count() < text_width {
        format!(
            "{}{}",
            fragment,
            " ".repeat(text_width.saturating_sub(fragment.chars().count()))
        )
    } else {
        fragment.chars().take(text_width).collect()
    }
}

/// Builds wrapped display lines for a [`RowKind::Changed`] row (intra-line char diff + gutters).
fn extend_diff_display_for_intraline_changed_row(
    row: &AlignedRow,
    left_display: &mut Vec<Line<'static>>,
    right_display: &mut Vec<Line<'static>>,
    text_lw: usize,
    text_rw: usize,
    line_num_width: usize,
    p: DiffViewerPalette,
) {
    let (segl, segr) = char_diff_segment_pairs(&row.left, &row.right, p);
    let fill_l = style_for_cell(RowKind::Changed, true, p);
    let fill_r = style_for_cell(RowKind::Changed, false, p);
    let ll: Vec<Line> = wrap_segments_to_lines(segl, text_lw);
    let rr: Vec<Line> = wrap_segments_to_lines(segr, text_rw);
    let n = ll.len().max(rr.len());
    for i in 0..n {
        let l_cont = ll
            .get(i)
            .cloned()
            .unwrap_or_else(|| Line::from(Span::styled(" ".repeat(text_lw), fill_l)));
        let r_cont = rr
            .get(i)
            .cloned()
            .unwrap_or_else(|| Line::from(Span::styled(" ".repeat(text_rw), fill_r)));
        append_guttered_side_by_side_lines(
            left_display,
            right_display,
            l_cont,
            r_cont,
            row.old_line,
            row.new_line,
            line_num_width,
            i > 0,
            text_lw,
            text_rw,
            fill_l,
            fill_r,
            p,
        );
    }
}

/// Wraps equal / only-left / only-right logical lines and appends guttered screen lines (gap stripes).
fn extend_diff_display_for_wrapped_non_changed_row(
    row: &AlignedRow,
    left_display: &mut Vec<Line<'static>>,
    right_display: &mut Vec<Line<'static>>,
    text_lw: usize,
    text_rw: usize,
    line_num_width: usize,
    p: DiffViewerPalette,
) {
    let wl = wrap_line(&row.left, text_lw);
    let wr = wrap_line(&row.right, text_rw);
    let n = wl.len().max(wr.len());
    for i in 0..n {
        let ls = wl.get(i).map(String::as_str).unwrap_or("");
        let rs = wr.get(i).map(String::as_str).unwrap_or("");
        let pad_left = i >= wl.len();
        let pad_right = i >= wr.len();
        let l_gap = wrapped_fragment_needs_gap_stripe(true, pad_left, ls, rs, row.kind);
        let r_gap = wrapped_fragment_needs_gap_stripe(false, pad_right, rs, ls, row.kind);
        let lstyle = style_for_gap_stripe_or_row_cell(l_gap, row.kind, true, p);
        let rstyle = style_for_gap_stripe_or_row_cell(r_gap, row.kind, false, p);
        let ltext = padded_or_gap_cell_string(l_gap, ls, text_lw);
        let rtext = padded_or_gap_cell_string(r_gap, rs, text_rw);
        let l_line = Line::from(Span::styled(ltext, lstyle));
        let r_line = Line::from(Span::styled(rtext, rstyle));
        append_guttered_side_by_side_lines(
            left_display,
            right_display,
            l_line,
            r_line,
            row.old_line,
            row.new_line,
            line_num_width,
            i > 0,
            text_lw,
            text_rw,
            lstyle,
            rstyle,
            p,
        );
    }
}

fn rebuild_display_cache(
    ready: &mut DiffViewerReady,
    col_lw: usize,
    col_rw: usize,
    p: DiffViewerPalette,
) {
    let line_num_width = ready.line_num_width;
    let text_lw = diff_column_text_width(col_lw, line_num_width);
    let text_rw = diff_column_text_width(col_rw, line_num_width);

    let cache_l = col_lw as u16;
    let cache_r = col_rw as u16;
    if ready.cached_widths == (cache_l, cache_r) && !ready.left_display.is_empty() {
        return;
    }
    ready.left_display.clear();
    ready.right_display.clear();

    for row in &ready.aligned {
        if row.kind == RowKind::Changed {
            extend_diff_display_for_intraline_changed_row(
                row,
                &mut ready.left_display,
                &mut ready.right_display,
                text_lw,
                text_rw,
                line_num_width,
                p,
            );
        } else {
            extend_diff_display_for_wrapped_non_changed_row(
                row,
                &mut ready.left_display,
                &mut ready.right_display,
                text_lw,
                text_rw,
                line_num_width,
                p,
            );
        }
    }
    ready.cached_widths = (cache_l, cache_r);
}

impl DiffViewerReady {
    fn total_rows(&self) -> usize {
        self.left_display.len()
    }

    /// Rows visible in the diff body (full frame minus title and status lines).
    fn visible_height(&self) -> usize {
        self.area.height.saturating_sub(2).max(1) as usize
    }
}

/// Lines to move per mouse wheel notch (matches common editor / browser feel).
const DIFF_MOUSE_SCROLL_LINES: usize = 3;

fn diff_scroll_by_lines(
    d: &mut DiffViewerReady,
    delta_down: isize,
) {
    let h = d.visible_height();
    let total = d.total_rows();
    let max_scroll = total.saturating_sub(h).max(0);
    let s = d.scroll as isize + delta_down;
    d.scroll = s.clamp(0, max_scroll as isize) as usize;
}

/// When the diff viewer is open, capture mouse so panels do not scroll underneath.
/// Wheel: scroll the diff (`DIFF_MOUSE_SCROLL_LINES` per notch, same direction as ↑↓).
pub fn handle_diff_mouse(
    app: &mut AppState,
    mouse_event: MouseEvent,
) -> bool {
    if app.diff_viewer_screen.is_none() {
        return false;
    }
    if let Some(DiffViewerState::Ready(d)) = app.diff_viewer_screen.as_mut() {
        let n = DIFF_MOUSE_SCROLL_LINES as isize;
        match mouse_event.kind {
            MouseEventKind::ScrollUp => diff_scroll_by_lines(d, -n),
            MouseEventKind::ScrollDown => diff_scroll_by_lines(d, n),
            _ => {}
        }
    }
    true
}

pub fn handle_diff_key(
    app: &mut AppState,
    key: KeyEvent,
) -> Option<AppAction> {
    match app.diff_viewer_screen.as_mut()? {
        DiffViewerState::Loading { .. } => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
                return Some(AppAction::DiffViewerClose);
            }
            Some(AppAction::Continue)
        }
        DiffViewerState::Ready(d) => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
                return Some(AppAction::DiffViewerClose);
            }
            let h = d.visible_height();
            let total = d.total_rows();
            let max_scroll = total.saturating_sub(h).max(0);
            match key.code {
                KeyCode::Up => d.scroll = d.scroll.saturating_sub(1),
                KeyCode::Down => d.scroll = (d.scroll + 1).min(max_scroll),
                KeyCode::PageUp => d.scroll = d.scroll.saturating_sub(h),
                KeyCode::PageDown => d.scroll = (d.scroll + h).min(max_scroll),
                KeyCode::Home => d.scroll = 0,
                KeyCode::End => d.scroll = max_scroll,
                _ => {}
            }
            Some(AppAction::Continue)
        }
    }
}

/// Horizontal layout shared by the diff header row and body: left pane, right pane.
/// The divider is the left pane’s right [`Block`] border (no separate gutter column), so the
/// terminal default background cannot show through on “empty” lines beside a one-line widget.
fn diff_viewer_column_constraints() -> [Constraint; 2] {
    [Constraint::Percentage(50), Constraint::Min(1)]
}

/// One header row inside a pane: path (truncated) flush left, `trailing` flush right, padded to `pane_width_cells`.
fn diff_pane_header_line(
    path: &str,
    pane_width_cells: usize,
    trailing: &str,
    p: DiffViewerPalette,
) -> Line<'static> {
    let pad_style = Style::default().bg(p.background).fg(p.text);
    let path_style = Style::default().bg(p.background).fg(p.header_path);
    let trail_style = Style::default().bg(p.background).fg(p.muted);
    if pane_width_cells == 0 {
        return Line::from(Span::styled("", pad_style));
    }
    let tw = trailing.chars().count();
    let sep = if tw > 0 { 1 } else { 0 };
    let max_path = pane_width_cells.saturating_sub(tw + sep);
    let path_shown = if max_path == 0 {
        String::new()
    } else if path.chars().count() <= max_path {
        path.to_string()
    } else {
        truncate_middle(path, max_path.max(1))
    };
    let used = path_shown.chars().count() + tw;
    let fill = pane_width_cells.saturating_sub(used);
    if tw == 0 {
        return Line::from(Span::styled(path_shown, path_style));
    }
    Line::from(vec![
        Span::styled(path_shown, path_style),
        Span::styled(" ".repeat(fill), pad_style),
        Span::styled(trailing.to_string(), trail_style),
    ])
}

pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    let Some(state) = app.diff_viewer_screen.as_mut() else {
        return;
    };
    let area = f.area();
    let p = app.ui_palette.diff_viewer;
    let base = Style::default().bg(p.background).fg(p.text);
    f.render_widget(
        Block::default().style(Style::default().bg(p.background)),
        area,
    );

    match state {
        DiffViewerState::Loading {
            left_path,
            right_path,
            ..
        } => {
            let msg = "Reading files — Esc to cancel";
            let bottom = Line::from(Span::styled(
                " Esc: close ",
                Style::default().fg(p.muted),
            ));
            let chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);
            let header_cols = Layout::horizontal(diff_viewer_column_constraints()).split(chunks[0]);
            let left_header_area = header_cols[0];
            let right_header_area = header_cols[1];
            let header_left_block = Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(p.column_border).bg(p.background))
                .style(Style::default().bg(p.background));
            let left_header_inner = header_left_block.inner(left_header_area);
            let lw = left_header_inner.width.max(1) as usize;
            let rw = right_header_area.width.max(1) as usize;
            f.render_widget(header_left_block, left_header_area);
            f.render_widget(
                Paragraph::new(diff_pane_header_line(
                    left_path.as_str(),
                    lw,
                    "",
                    p,
                ))
                .style(base),
                left_header_inner,
            );
            f.render_widget(
                Paragraph::new(diff_pane_header_line(
                    right_path.as_str(),
                    rw,
                    "",
                    p,
                ))
                .style(base),
                right_header_area,
            );
            f.render_widget(Paragraph::new(msg).style(base), chunks[1]);
            f.render_widget(
                Paragraph::new(bottom).style(base),
                chunks[2],
            );
        }
        DiffViewerState::Ready(d) => {
            d.area = area;
            let chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

            let content_area = chunks[1];
            let cols = Layout::horizontal(diff_viewer_column_constraints()).split(content_area);

            let left_area = cols[0];
            let right_area = cols[1];
            let left_block = Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(p.column_border).bg(p.background))
                .style(Style::default().bg(p.background));
            let left_inner = left_block.inner(left_area);
            let lw = left_inner.width.max(1) as usize;
            let rw = right_area.width.max(1) as usize;
            rebuild_display_cache(d, lw, rw, p);

            let total = d.total_rows();
            let h = content_area.height.max(1) as usize;
            let max_scroll = total.saturating_sub(h).max(0);
            d.scroll = d.scroll.min(max_scroll);
            let start = d.scroll;
            let end = (start + h).min(total);

            let pct = if total == 0 {
                100u16
            } else {
                (((start + h).min(total) * 100) / total.max(1)) as u16
            };
            let header_cols = Layout::horizontal(diff_viewer_column_constraints()).split(chunks[0]);
            let left_header_area = header_cols[0];
            let right_header_area = header_cols[1];
            let header_left_block = Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(p.column_border).bg(p.background))
                .style(Style::default().bg(p.background));
            let left_header_inner = header_left_block.inner(left_header_area);
            let hlw = left_header_inner.width.max(1) as usize;
            let hrw = right_header_area.width.max(1) as usize;
            // Paths only: line count / scroll % stay on the bottom bar (avoids a crowded number by the divider).
            f.render_widget(header_left_block, left_header_area);
            f.render_widget(
                Paragraph::new(diff_pane_header_line(
                    d.left_path.as_str(),
                    hlw,
                    "",
                    p,
                ))
                .style(base),
                left_header_inner,
            );
            f.render_widget(
                Paragraph::new(diff_pane_header_line(
                    d.right_path.as_str(),
                    hrw,
                    "",
                    p,
                ))
                .style(base),
                right_header_area,
            );

            let left_slice: Vec<Line> = d.left_display.get(start..end).unwrap_or(&[]).to_vec();
            let right_slice: Vec<Line> = d.right_display.get(start..end).unwrap_or(&[]).to_vec();

            let left_para = Paragraph::new(Text::from(left_slice))
                .style(base)
                .wrap(Wrap { trim: false });
            let right_para = Paragraph::new(Text::from(right_slice))
                .style(base)
                .wrap(Wrap { trim: false });

            f.render_widget(left_block, left_area);
            f.render_widget(left_para, left_inner);

            f.render_widget(right_para, right_area);

            let bar = Line::from(vec![
                Span::styled(" Esc ", Style::default().fg(p.muted)),
                Span::styled(
                    "close  ",
                    Style::default().fg(p.text).bg(p.background),
                ),
                Span::styled(
                    "↑↓ PgUp/PgDn wheel",
                    Style::default().fg(p.muted),
                ),
                Span::styled(
                    format!("  {} lines  {}%", total, pct),
                    Style::default().fg(p.muted),
                ),
            ]);
            f.render_widget(Paragraph::new(bar).style(base), chunks[2]);
        }
    }
}

fn truncate_middle(
    s: &str,
    max_chars: usize,
) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return "…".to_string();
    }
    let keep = max_chars - 1;
    let half = keep / 2;
    let start: String = s.chars().take(half).collect();
    let end: String = s
        .chars()
        .rev()
        .take(keep - half)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{}…{}", start, end)
}
