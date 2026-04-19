# Features and Key Bindings

## Features

- **Two-panel layout** — Single- or double-column view modes
- **Resilient refresh** — **Ctrl+R** and other refreshes re-read the current listing. If the folder you are in was removed (for example deleted from the other panel or from a subshell), the panel moves to the nearest existing parent directory instead of failing with a missing-path error. The same applies if an archive file opened as a virtual folder was deleted on disk.
- **Full keyboard navigation** — Arrow keys, Tab, Enter, F-keys
- **File operations** — Copy, move, delete with overwrite and error handling
- **Archives** — Browse **`.zip`**, **`.tar.gz`**, and **`.tgz`** as virtual folders (same operations as on disk where supported). **Ctrl+A** creates an archive; use a **`.zip`** or **`.tar.gz`** / **`.tgz`** file name to pick the format
- **Safe delete (F9 → General)** — When **on** (default), **F8** delete moves filesystem files and folders to the **OS trash** where supported (macOS; Linux when a standard trash location is writable). When **off**, or when trash is not available on the platform, delete is **permanent**. Deleting entries inside a **ZIP** or **tar.gz** panel still removes them from the archive only (not the system trash). Stored in `~/.oxide/settings.json`.
- **Viewer (F3)** — Text and hex modes; scroll with **↑↓**, **PgUp/PgDn**, and **mouse wheel**. Large files are read in a background thread so Esc closes immediately; a "Loading…" screen is shown until the read completes. In text mode, binary and non-printable characters are shown as `.` to avoid terminal corruption
- **Compare files (Ctrl+D)** — Mark **exactly two** non-directory files with **Space** (they can be on one panel or split across left and right). Opens a full-screen **side-by-side diff**: line numbers in each gutter, patience line diff, synchronized scrolling, wrapped long lines, and **intra-line** highlights on changed lines (insert/delete runs). **↑↓**, **PgUp/PgDn**, **Home**/**End**, and **mouse wheel** scroll; **Esc** closes. Reads both files in the background (like F3). If the mark count is not two, a short **toast** explains what to do. Works on normal directories and inside **ZIP** / **tar.gz** panels (same read path as the viewer)
- **Embedded editor (F4)** — Syntax highlighting, **Ctrl+X** then **F** in-file find, save/discard; **mouse wheel** scrolls in the editor surface. The terminal caret is hidden while the edit position is scrolled out of view and reappears when you move the caret back into the viewport
- **Create directory (F7)**
- **Rename / Attributes (F2)** — Change name, permissions, owner/group (Unix)
- **Size info (Ctrl+X then S)** — Total size of selected files/folders shown in the panel bottom bar. Requires at least one selected item. Shows progress during calculation, then final total. Any key or mouse click dismisses (Ctrl+O spawns shell instead).
- **Hidden files (Ctrl+H)** — Shown by default; Ctrl+H toggles visibility in both panels
- **Shell relay (Ctrl+O)** — Spawn subshell, run commands, return to panels
- **Command line** — Run shell commands; F12 inserts the current (selected) file name at the cursor without running (Enter runs the command)
- **Mouse support** — Clicks and wheel on the panels; when **F3**, **F4**, or **Ctrl+D diff** is open, the wheel scrolls that view instead of the file list
- **Disk space display** — Shows usage on Unix
- **Find file (Ctrl+F)** — Search under a start directory by file name pattern (and optional text-in-file). **File pattern mode** (wildcards vs regex) is set in **F9 → General**. **Wildcards:** `*` and `?`; use **`|`** to give several alternative globs in one field (e.g. `Screenshot*|*.zip|file*`). **`|`** is only split into multiple globs in wildcard mode — in **regex** mode, `|` is normal regex alternation. **Ignore pattern** (optional) uses the same wildcard/regex rules as the file pattern, but is matched against the **path relative to the start directory** (forward slashes); if it matches, that hit is skipped. Example: file pattern `*.zip`, ignore `*node_modules*` finds zip files but not under any `node_modules` segment.

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
| Ctrl+X C | Save panel paths & active panel to `~/.oxide/settings.json` (configuration snapshot; same as **Autosave latest state**) |
| Ctrl+X S | Size info |
| Ctrl+H | Toggle hidden files |
| Ctrl+O | Shell |
| Ctrl+R | Refresh |
| Ctrl+T | View mode |
| Ctrl+D | Compare two marked files (full-screen diff viewer) |
| Type char | Focus command line and insert character |
| Tab/Esc | Panel focus |

### Find file (Ctrl+F)

- **Fields:** start directory; **file pattern**; optional **ignore pattern**; optional **content** search string.
- **Pattern mode (F9 → General):** **Wildcards** (`*`, `?`) or **regex**. In wildcard mode, **several globs in one field** are written **pipe-separated** — a file name matches if **any** segment matches (e.g. `Screenshot*|*.zip|file*`). In regex mode, `|` is ordinary regex alternation (the pattern is **not** split).
- **Ignore pattern** — Optional; **same wildcard/regex rules** as the file pattern, but matched against the **path relative to the start directory** (with `/` separators). If it matches, that hit is **excluded** (e.g. file pattern `*.zip` with ignore `*node_modules*` lists zip files but skips paths that contain `node_modules`).

While the dialog is open: **Tab / ↑↓** move between fields and options; **Enter** starts the search or (on a result) changes directory; **Esc** closes. **F3** / **F4** on a result view or edit; dialog stays open.

### Settings (F9)

- **General → Safe delete** — Toggles whether **F8** delete uses the **system trash** (when supported) or **immediate removal**. Default is on. If the OS trash is not usable (unsupported OS or trash location not writable), the control is shown as unavailable and deletes are permanent.
- Other sections: panel view/sort/hidden, autosave, shell sync, file pattern mode (wildcards vs regex for Find and **+**/**−**), app info.

### Editor (F4)

| Keys | Action |
|------|--------|
| Shift+←→↑↓ | Select |
| F3 | Line numbers |
| Ctrl+C/V | Copy/Paste |
| F2 | Save |
| Ctrl+X F | Find in file |
| Mouse wheel | Scroll |
| Esc | Exit |

### Viewer (F3)

| Keys | Action |
|------|--------|
| H | Hex/text toggle |
| ↑↓ PgUp/PgDn | Scroll |
| Home / End | Jump to start / end of file |
| Mouse wheel | Scroll |
| Esc | Close |

In text mode, control and binary characters are displayed as `.` (same convention as the hex dump ASCII column) so the terminal is not corrupted.

### Diff viewer (Ctrl+D)

Requires **exactly two marked** non-directory files (**Space**). Stable order: marks on the **left** panel first (by list index), then the **right** panel—so one file per panel compares as left vs right.

| Keys | Action |
|------|--------|
| ↑↓ PgUp/PgDn | Scroll (both sides stay aligned) |
| Home / End | Top / bottom of diff |
| Mouse wheel | Scroll |
| Esc | Close |

Side-by-side columns show **old** (first marked file) and **new** (second). Insert-only and delete-only rows pad the opposite column so rows line up. **Changed** lines use a character-level diff for finer highlighting on top of the line-level colors.
