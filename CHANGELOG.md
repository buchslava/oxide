# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Version numbers follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html): **MAJOR** for incompatible API/behavior changes, **MINOR** for backward-compatible additions, **PATCH** for backward-compatible fixes.

## How to maintain this file

1. **While developing** — Under `[Unreleased]`, add bullets in the right subsection (`Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`). Write for readers (what changed and why), not only commit hashes.
2. **When cutting a release** — Rename `[Unreleased]` to `[version] - YYYY-MM-DD` (ISO date), then add a new empty `[Unreleased]` section at the top. Bump `version` in `Cargo.toml` to match.
3. **Avoid** — Rewriting past release sections (history is append-only). Dumping raw `git log` as the only narrative (optional “commit index” at the bottom is fine).

Package version for this crate is defined in `Cargo.toml` (`[package].version`).

## [0.1.0]

... start

## [0.2.0] - 2026-05-09

First changelog entry for the **0.2.0** release (crate version bumped from 0.1.x).

### Added

- **Raster image viewer (F3)** — Background load; terminal rendering via `ratatui-image` (Kitty, iTerm2, Sixel, half-blocks). Overrides: `OXIDE_IMAGE_SKIP_CAP_QUERY`, `OXIDE_IMAGE_PROTOCOL` (`halfblocks`, `iterm2`, `sixel`, `kitty`). iTerm2 uses a reliable OSC 1337 path when probing would pick a broken protocol.
- **F1 Actions dialog** — Scrollable, mouse-friendly shortcut list; keyboard focus on rows (**Tab** / **Shift+Tab**, **↑** / **↓**, **j** / **k**, **Home** / **End**, **PgUp** / **PgDn**); **Enter** runs the focused action (same as clicking the grey chord). While the dialog is open, **Ctrl+X** second keys, **Ctrl+O**, **Ctrl+R**, and **Ctrl+C** / **Ctrl+V** still apply (**Ctrl+C** / **Ctrl+V** target the command line), matching panel shortcuts without closing first.
- **Ctrl+X chord handling** (`ctrl_x_chord.rs`) — Suspend and global chords coordinated with overlays.
- **Size info (Ctrl+X S)** — MC-style **Directory scanning** dialog while totals are computed: live path (compact), directory and file counts, cumulative size in KiB with thousands separators, dimmed backdrop, and **[ Abort ]** / **Esc** / **A** to cancel (`AtomicBool` + cooperative walk in `FileOperations::size_of_path_recursive_cancellable`). **Done** state uses a centered **Size** dialog with selection summary, **Disk:** line from `statvfs`, and a dismiss hint; `format_disk_bytes` / `format_u64_with_commas` in `text_format.rs` render capacities in decimal SI (KB–PB) instead of raw integer “gigabytes”.
- **`doc/PKGBUILD`** — Example Arch-style packaging metadata.
- Dependencies: `image`, `ratatui-image` for the above viewer.
- **Terminal color depth (`OXIDE_COLOR_DEPTH`)** — Themes use 24-bit RGB by default; on older terminals that mishandle truecolor escapes, the UI can look wrong or glitchy. **Auto** mode uses `COLORTERM=truecolor` for full RGB, `TERM` names containing `256color` (and a few modern terminal hints) for xterm 256-color (`Color::Indexed`), otherwise maps RGB to the nearest 16 ANSI colors. Override anytime: `truecolor`, `256`, or `16` (also accepts `24bit`, `8bit`, `ansi`, etc.). The resolved palette is applied at startup and when the theme changes; the root-session menu strip and Crossterm main-buffer overlays use the same mapping so they stay consistent with Ratatui.

### Changed

- **Panel file list** — Dropped the left permission column and the size column next to each name (single-column rows are mark + name + mtime; double-column rows are mark + MC-style name only, e.g. `*file` / `/dir`).
- **Shortcuts:** save panel state **Ctrl+X C** (was Ctrl+E in prior docs); size info **Ctrl+X S** (was Ctrl+G); editor in-file find **Ctrl+X F** (was Ctrl+F, to free Ctrl+F for global find).
- **Size info** — Filesystem free/total for the active path is shown only in the **Size** result dialog, not in the panel bottom bar (the bar no longer reserves a disk column while size info is open).
- **Panel bottom bar** — The selected file’s Unix permission string (same `rwxr-xr-x` style as listings) appears to the left of the size on the right side; directories show the mode when there is no size. If the bar is too narrow, the meta text is truncated with a suffix ellipsis so the size tail stays visible when possible.
- **F1** opens Actions instead of the old full-screen help dialog.
- **F1 Actions dialog** — **↑** / **↓** / **j** / **k** move the focused shortcut row (and scroll to keep it in view), not the viewport alone; mouse wheel and scrollbar still scroll the list. Intro and footer describe focus and **Enter**.
- **F1 Actions dialog** — Single static row layout with a lazily built hit map (`OnceLock`), one full line build per draw, and a linear scan for the focused line index (no per-keystroke hit vec allocation).
- **Subshell** — Deeper integration with the app loop, suspend/resume, and parent/subshell communication.
- **Post-command countdown** — When the main-buffer countdown runs before panels return (after a subshell command with auto-reopen), **Esc** abandons the wait and restores the panel TUI immediately.
- **Root chrome in subshell** — The red menu/prompt indicator is now driven by **“any UID 0 process attached to the subshell PTY”** (tty-wide `ps -t … -o uid=`), instead of attempting to detect a “root shell session” specifically. This makes `sudo -s`/`sudo -i` reliably flip the chrome red across platforms, at the cost of also turning it red for any other root program on that PTY (by design).
- **Diff viewer** — Input and cancel behavior during long reads.
- **Renderer / overlays** — Layout updates for new dialogs and viewers.
- **Docs** — `doc/FEATURES.md` (and related) updated for shortcuts and editor caret behavior when scrolled off-screen.

### Removed

- **Legacy help dialog** (`help_dialog.rs`) — Replaced by the F1 Actions flow.

### Fixed

- **Disk space line (`statvfs`)** — Byte totals use **`f_frsize`** (fragment size) with `f_blocks` / `f_bfree`, not **`f_bsize`**, fixing POSIX-correct math and wildly inflated volumes on macOS APFS (previously ~256× too large).
- **Size dialog (Done)** — **Esc** closes the dialog only and is not passed through to the panel handler that moves focus to the command line.
- **Archives (ZIP / tar.gz)** — Virtual listings use stored Unix modes (and symlinks for tar) so executable styling matches metadata when the archive records it. Extract (F5 / copy-out) applies stored permission bits on Unix (`chmod`) after writing files and directories.
- **Create archive (Ctrl+X A)** — New `.zip` members get `.unix_permissions()` from each source path on Unix (no longer everything 0644). `.tar.gz` creation uses each directory’s real mode for GNU headers instead of a fixed `0755`. Adding to an existing ZIP, delete/move rewrite, mkdir-in-zip, and single-file write preserve each kept entry’s previous Unix mode when rebuilding the central directory.
- **tar.gz rewrite** — In-memory `TarEntry` now carries the header mode from the archive (or from disk when adding from a filesystem panel) so round-trips do not force `755`/`644` on every member.
- **Recursive copy (F5)** — On Unix, destination directories created during tree copy get the same permission bits as the matching source directories (`fs::copy` already preserved file modes).
- Panel refresh when paths vanish or change; bottom bar under `/` after `sudo` and related state issues.
- Diff viewer: **Esc** can abandon or close during heavy operations.
- Dialog text inputs across settings, rename, pattern select, mkdir, new file, archive, and panel overlays.
- Settings/autosave aligned with new Ctrl+X chords.
- Cross-shell relay reliability.
- **F4 embedded editor** — Line-number gutter background followed a hardcoded gray whenever `chrome.main_background` was not `Color::Rgb` (for example after palette adaptation for 256-color or 16-color terminals, common on Linux without truecolor). The gutter is now derived from the same approximate RGB as the chrome background, then remapped with the active color depth so it matches the theme.
