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

## [0.2.0] - 2026-05-05

First changelog entry for the **0.2.0** release (crate version bumped from 0.1.x).

### Added

- **Raster image viewer (F3)** — Background load; terminal rendering via `ratatui-image` (Kitty, iTerm2, Sixel, half-blocks). Overrides: `OXIDE_IMAGE_SKIP_CAP_QUERY`, `OXIDE_IMAGE_PROTOCOL` (`halfblocks`, `iterm2`, `sixel`, `kitty`). iTerm2 uses a reliable OSC 1337 path when probing would pick a broken protocol.
- **F1 Actions dialog** — Scrollable, mouse-friendly shortcut/action list.
- **Ctrl+X chord handling** (`ctrl_x_chord.rs`) — Suspend and global chords coordinated with overlays.
- **`doc/PKGBUILD`** — Example Arch-style packaging metadata.
- Dependencies: `image`, `ratatui-image` for the above viewer.

### Changed

- **Shortcuts:** save panel state **Ctrl+X C** (was Ctrl+E in prior docs); size info **Ctrl+X S** (was Ctrl+G); editor in-file find **Ctrl+X F** (was Ctrl+F, to free Ctrl+F for global find).
- **F1** opens Actions instead of the old full-screen help dialog.
- **Subshell** — Deeper integration with the app loop, suspend/resume, and parent/subshell communication.
- **Root chrome in subshell** — The red menu/prompt indicator is now driven by **“any UID 0 process attached to the subshell PTY”** (tty-wide `ps -t … -o uid=`), instead of attempting to detect a “root shell session” specifically. This makes `sudo -s`/`sudo -i` reliably flip the chrome red across platforms, at the cost of also turning it red for any other root program on that PTY (by design).
- **Diff viewer** — Input and cancel behavior during long reads.
- **Renderer / overlays** — Layout updates for new dialogs and viewers.
- **Docs** — `doc/FEATURES.md` (and related) updated for shortcuts and editor caret behavior when scrolled off-screen.

### Removed

- **Legacy help dialog** (`help_dialog.rs`) — Replaced by the F1 Actions flow.

### Fixed

- Panel refresh when paths vanish or change; bottom bar under `/` after `sudo` and related state issues.
- Diff viewer: **Esc** can abandon or close during heavy operations.
- Dialog text inputs across settings, rename, pattern select, mkdir, new file, archive, and panel overlays.
- Settings/autosave aligned with new Ctrl+X chords.
- Cross-shell relay reliability.

### Commit index (0.2.0 development line)

Newest first; useful for archaeology, not a substitute for the sections above.

| Commit     | Message |
|------------|---------|
| `542972d` | fix(image view): iTerm2 compatibility |
| `67134e8` | feat(global): image viwer and save config fix |
| `16ffe92` | chore: minor |
| `294d4a7` | feature(global): actions menu and fixes |
| `c50a266` | fix(global): cross shell communication |
| `1c3c704` | fix(global): major fixes - panels refreshing and botton menu under root after sudo |
| `54be8a3` | fix(diff): abandon via esc on heavy operations |
| `4e5cf24` | fix(ui): dialogs inputs |
| `3faf18c` | fix(global): critical fixes |
| `d19c078` | fix(global): critical fixes |
