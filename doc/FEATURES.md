# Features and Key Bindings

## Features

- **Two-panel layout** — Single- or double-column view modes
- **Full keyboard navigation** — Arrow keys, Tab, Enter, F-keys
- **File operations** — Copy, move, delete with overwrite and error handling
- **Viewer (F3)** — Text and hex modes, scroll. Large files are read in a background thread so Esc closes immediately; a "Loading…" screen is shown until the read completes. In text mode, binary and non-printable characters are shown as `.` to avoid terminal corruption
- **Embedded editor (F4)** — Syntax highlighting, Ctrl+F search, save/discard
- **Create directory (F7)**
- **Rename / Attributes (F2)** — Change name, permissions, owner/group (Unix)
- **Size info (Ctrl+G)** — Total size of selected files/folders shown in the panel bottom bar. Requires at least one selected item. Shows progress during calculation, then final total. Any key or mouse click dismisses (Ctrl+O spawns shell instead).
- **Hidden files (Ctrl+H)** — Shown by default; Ctrl+H toggles visibility in both panels
- **Shell relay (Ctrl+O)** — Spawn subshell, run commands, return to panels
- **Command line** — Run shell commands; F12 inserts the current (selected) file name at the cursor without running (Enter runs the command)
- **Mouse support** — Clicks, scroll
- **Disk space display** — Shows usage on Unix

## Key Bindings

### Panels

| Keys | Action |
|------|--------|
| ↑↓ PgUp/PgDn | Navigate |
| Tab | Switch panel |
| ← → | Move column |
| Enter | Open dir / run |
| Space | Mark |
| * | Invert selection |

### F-keys

| Keys | Action |
|------|--------|
| F1 | Settings / Help |
| F2 | Rename / Attributes |
| F3 | View |
| F4 | Edit |
| F5 | Copy |
| F6 | Move |
| F7 | New directory |
| F8 | Delete |
| F10 | Quit |
| F12 | (command line) Insert current file at cursor |

### Command line

When focus is on the command line (e.g. after typing a character or F6):

| Keys | Action |
|------|--------|
| Enter | Run command |
| F12 | Insert current (selected) file name at cursor (no run) |
| Tab / Esc | Return focus to panel |

### Shortcuts

| Keys | Action |
|------|--------|
| Ctrl+G | Size info |
| Ctrl+H | Toggle hidden files |
| Ctrl+O | Shell |
| Ctrl+R | Refresh |
| Ctrl+T | View mode |
| Type char | Focus command line and insert character |
| Tab/Esc | Panel focus |

### Editor (F4)

| Keys | Action |
|------|--------|
| Shift+←→↑↓ | Select |
| F3 | Line numbers |
| Ctrl+C/V | Copy/Paste |
| F2 | Save |
| Ctrl+F | Find |
| Esc | Exit |

### Viewer (F3)

| Keys | Action |
|------|--------|
| H | Hex/text toggle |
| ↑↓ PgUp/PgDn | Scroll |
| Esc | Close |

In text mode, control and binary characters are displayed as `.` (same convention as the hex dump ASCII column) so the terminal is not corrupted.
