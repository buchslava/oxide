# Reference

## Midnight Commander (MC) source

Useful for matching behavior (command line, shell, panels, key bindings).

- **Local path:** `../mc` (Midnight Commander code next to this project).

- **Clone (optional):**  
  `git clone https://github.com/MidnightCommander/mc.git`  
  Put it next to this project (e.g. `../mc`) or inside as `mc/` or `reference/mc/` (those paths are in `.gitignore`).

- **Relevant areas in MC:**  
  - Command line / input line: `src/consaver.c`, `src/command.c`, `src/keybind-defaults.c`  
  - Panels: `src/panel.c`, `src/filemanager.c`  
  - Shell / subshell: `src/subshell.c`, `src/execute.c`  
  - Main loop / layout: `src/main.c`, `src/layout.c`

When changing morning-commander behavior (Ctrl+O, command line, redraw after return), compare with MC’s handling in these files.

---

## Best practices from MC (from `../mc`)

Digging into `../mc/src/execute.c`, `../mc/src/subshell/common.c`, `../mc/src/filemanager/command.c`:

### Running a command (`execute.c`)

1. **Before running (pre_exec / edition_pre_exec):**
   - Optionally clear screen (`clear_before_exec`) or print `\n\n`.
   - Leave alternate screen (`tty_reset_screen` / `tty_exit_ca_mode`) so the command runs on the main terminal.
   - Switch to shell mode: disable mouse, reset keypad, reset terminal mode.

2. **Run the command:** `my_systemv_flags()` or, with subshell, `invoke_subshell()`.

3. **After command returns:**
   - **Pause (optional):** If `pause_after_run` is set: print `"Press any key to continue..."`, `tty_raw_mode()`, `get_key_code(0)` (wait for one key), then newline. So MC *can* show a prompt and wait; it’s configurable.
   - **Return to TUI (edition_post_exec):** `tty_enter_ca_mode()` (enter alternate screen), `tty_reset_prog_mode()`, **`tty_flush_input()`** (drain input so the next key isn’t a leftover), `tty_raw_mode()`, enable mouse, etc.
   - **Panels:** `update_panels(UP_OPTIMIZE, UP_KEEPSEL)` — refresh panels but **keep selection** (same idea as our `refresh_files_restore_selection`).
   - `do_refresh()`, `use_dash(TRUE)`.

4. **Takeaways for morning-commander:**
   - Hide panels before command (leave alternate screen); show panels after (enter alternate).
   - After return: drain input (we use `process_queued_events()`; MC uses `tty_flush_input()`) so the first keypress isn’t lost.
   - Refresh panels after command but restore selection (we do this).
   - Optional “wait for key” after command is a configurable MC behavior; we do it without the message so the user can read the result.

### Ctrl+O (subshell toggle) — MC vs Ratatui

**MC flow** (`execute.c` `toggle_subshell()`, `subshell/common.c` `feed_subshell()`):

1. **Before handing terminal to shell**
   - `channels_down()`, `disable_mouse()`, `disable_bracketed_paste()`
   - Optionally `tty_clear_screen()` (config)
   - `tty_reset_shell_mode()` (ncurses: cooked/echo for shell), `tty_keypad(FALSE)`, **`tty_reset_screen()`** (= `endwin()` in ncurses), **`tty_exit_ca_mode()`** (switch to main screen, restore cursor)
   - `tty_raw_mode()` so MC can read keys and detect Ctrl+O
   - Console restore if needed
2. **Relay loop** (`feed_subshell(VISIBLY)`): `select()` on stdin + subshell PTY; stdin → PTY, PTY → stdout. On **Ctrl+O** in the byte stream: stop forwarding that key, set `subshell_state = INACTIVE`, optionally sync command line from subshell, then return so MC redraws.
3. **After return**
   - Console save if needed
   - **`tty_enter_ca_mode()`** first (switch back to alternate screen)
   - `tty_reset_prog_mode()`, `tty_keypad(TRUE)`, **`tty_flush_input()`** (drain typeahead so next key isn’t eaten), `enable_mouse()`, etc.
   - Optional CD from subshell CWD; `update_panels()`, refresh.

**Ratatui-specific points:**

- **Alternate screen**: We use crossterm `EnterAlternateScreen` / `LeaveAlternateScreen`. Same idea as MC’s `tty_enter_ca_mode` / `tty_exit_ca_mode`: leave so the shell runs on the “real” terminal (scrollback, no TUI), then re-enter so our UI is the only thing on screen.
- **No clear when leaving**: We do *not* clear the main screen when leaving (like MC when `clear_before_exec` is off), so the user sees their shell and its history.
- **Raw mode**: We keep raw mode on for the whole app; when we leave alternate we don’t switch to cooked mode for the relay. Our relay reads stdin in raw mode and forwards to the PTY; the subshell runs in a PTY so it has its own termios. So we don’t need `tty_reset_shell_mode` / `tty_raw_mode` like MC (they’re driving the same terminal as the shell when not using a PTY in some code paths; we always use a PTY for Ctrl+O).
- **Flush after leave**: We `terminal.flush()` and `backend_mut().flush()` after LeaveAlternateScreen so the switch is visible before we read in the relay.
- **Return order (critical for Ratatui)**: Enter alternate **first**, then drain input (`process_queued_events`), then `terminal.clear()` and `terminal.draw()`. **Never** call `terminal.flush()` after `draw()`: Ratatui’s `draw()` already flushes and swaps buffers; an extra flush would diff against the wrong buffer and can paint the screen black.
- **Drain input**: Like MC’s `tty_flush_input()`, we drain the event queue right after entering alternate so the key that triggered Ctrl+O (or any typeahead) doesn’t trigger an action in the TUI.

**PTY window size (avoids uglified ls/command output):**

- MC: `lib/tty/tty.c` `tty_resize(fd)` gets winsize from `STDOUT_FILENO` (TIOCGWINSZ), sets it on the given fd (TIOCSWINSZ). Copy full struct (including ws_xpixel/ws_ypixel). In `subshell/common.c`, `init_subshell_child()` calls `tty_resize(subshell_pty_slave)` **before** dup2/exec so the shell sees correct dimensions. On SIGWINCH, MC calls `tty_resize(mc_global.tty.subshell_pty)` (master).
- We: resize PTY **before fork** (after openpty, set size on master so the child never sees wrong size), and again at start of `run_relay_until_ctrl_o()` so each relay run uses current terminal size. Use 4K relay buffer (MC uses PTY_BUFFER_SIZE 512) so long lines aren't fragmented and we don't lose output.
- **PTY slave termios (MC: `init_subshell_child`):** MC sets the subshell PTY slave with `tcsetattr(slave, &shell_mode)` so the shell sees a normal cooked terminal (ICANON, ECHO, ONLCR, etc.). If the slave is left at default/raw, output can be wrong (gaps, missing chars). We set the slave to cooked in the child via `set_pty_slave_cooked_mode()` and disable VDISCARD there so Ctrl+O never toggles kernel output discard.
- **Prompt reappear (MC `invoke_subshell`):** When toggling to the shell, MC sends `" \b"` (when `subshell_ready`) so the prompt redraws. We send `" \b"` from inside `run_relay_until_ctrl_o()` right before flushing PTY output, so the shell’s response is read in the same run. We then flush PTY→stdout for up to 500ms so the prompt is visible (MC inits the subshell at startup so the prompt is always ready; we create it lazily so we need a longer window for the first prompt).
- **Ratatui/cursor:** After `LeaveAlternateScreen` the main buffer cursor may be left where the TUI had it. We write `\r\n` to stdout before the relay so the prompt appears on a new line (MC does `endwin()` before `tty_exit_ca_mode()` which resets the terminal state).

### Ctrl+O: MC vs morning-commander (side-by-side)

| Step | MC (`../mc`) | morning-commander |
|------|----------------|-------------------|
| **1. Before handing terminal to shell** | `channels_down()`, `disable_mouse()`, `disable_bracketed_paste()` | Do not use backend. Flush → `prepare_for_relay()` → `write_relay_reset_sequence(stdout)` (leave alternate + show + disable mouse + reset). See "Solution: Second Ctrl+O uglification". |
| | Optional `tty_clear_screen()` | (no clear; keep scrollback) |
| | **`tty_reset_shell_mode()`** — real tty → cooked (ncurses `reset_shell_mode`) | *(we stay in raw; no cooked step)* |
| | `tty_noecho()`, `tty_keypad(FALSE)`, **`tty_reset_screen()`** (= `endwin()`) | — |
| | **`tty_exit_ca_mode()`** — leave alternate screen | Leave alternate sent in `write_relay_reset_sequence` to stdout (single writer). |
| | **`tty_raw_mode()`** — real tty → raw (ncurses `raw()`/`cbreak()`) | *(already raw from app start)* |
| | `invoke_subshell(NULL, VISIBLY)` → **`tcsetattr(STDOUT, &raw_mode)`** — subshell’s custom raw (OPOST off, etc.) | *(we never set real tty to “relay” termios)* |
| **2. Relay** | `feed_subshell(VISIBLY)`: `select()` stdin + PTY; stdin→PTY, PTY→stdout; **`peek_subshell_switch_key()`** for Ctrl+O (raw 0x0F + kitty CSI … u) | `run_relay_until_ctrl_o()`: `poll()` stdin + PTY; same; **`relay_stdin_chunk()`** for 0x0F, kitty, modifyOtherKeys |
| **3. On Ctrl+O** | Write bytes before key to PTY; set state; optionally sync cmdline; **return** (no explicit flush of PTY) | Write bytes before key to PTY; **`drain_pty_output()`** then return |
| **4. After return** | **`tty_enter_ca_mode()`** first | **`EnterAlternateScreen`** first |
| | `tty_reset_prog_mode()`, `tty_keypad(TRUE)`, **`tty_flush_input()`** | `process_queued_events()` (drain) |
| | `enable_mouse()`, etc. | `terminal.clear()`, `draw()` |

**Differences that can affect display:**

- **Real tty mode during relay:** MC sets the real terminal to a **known** `raw_mode` (OPOST off, ICANON off, etc.) via `tcsetattr(STDOUT, &raw_mode)` at the start of `invoke_subshell`. We leave the real tty in whatever state crossterm left (raw, but possibly different flags). If the real tty has OPOST or other output processing on, relayed bytes can be double-processed or mis-displayed.
- **Reset before relay:** MC goes through `tty_reset_shell_mode()` and `tty_reset_screen()` (endwin) before `tty_exit_ca_mode()`, so the terminal is in a defined state before the first byte is shown. We **reset display state** (G0/G1, SGR, wrap) **before** the relay (right after LeaveAlternateScreen) so TUI leftover state doesn’t corrupt shell output.
- **Real tty during relay:** We set the real tty to MC-style raw at relay start (`set_real_tty_relay_raw`: OPOST off, ICANON off, etc.) and restore on exit, matching MC’s `tcsetattr(STDOUT, &raw_mode)` in `invoke_subshell`. We use **fd 1 (STDOUT)** for tcgetattr/tcsetattr like MC so we affect the same device we write PTY output to.
- **Non-blocking PTY read (MC: read_nonblock):** MC uses non-blocking read on the PTY master so that “between select() and read() the slave can do tcflush(), revoking the data” does not cause a lockup or lost/corrupt output. We use `read_pty_nonblock()` in the relay loop and in drain/flush so we never block on the PTY and avoid that race.

**Morning-commander Ctrl+O implementation** (see `main.rs` `AppAction::Suspend` and `subshell.rs` `run_relay_until_ctrl_o`):

- **Handing to shell:** Do **not** use the backend for leave alternate; use the single-writer flow (see **“Solution: Second Ctrl+O uglification”** below): flush, `prepare_for_relay()`, `write_relay_reset_sequence(stdout)`, `\r\n`, then relay with `Some(prepared)`.
- Create subshell (PTY) with cwd from active panel; run relay: stdin ↔ PTY until Ctrl+O byte (0x0F); do not clear main screen.
- On return: Enter alternate (backend), hide cursor, enable mouse → drain queue → clear → draw → drain again (handle any FocusGained). No `flush()` after `draw()`.

### Terminal state and “second Ctrl+O” uglification (wider picture)

The problem is not only the second attempt: it is **terminal state** when we switch from TUI (alternate screen) back to the main screen and start writing relay bytes. That state can be wrong on any run after the first.

**Crossterm alternate screen:** We use `?1049h` / `?1049l` (EnterAlternateScreen / LeaveAlternateScreen). With `?1049`, the terminal saves the cursor on the main screen when entering alternate, clears the alternate buffer, and restores the cursor when leaving. So main-screen cursor position is restored; the issue is not cursor position alone.

**Escape parser state (VT100/ECMA-48):** The terminal has a state machine (ground, escape, CSI entry, CSI param, etc.). If the TUI sent a partial sequence (e.g. `ESC [` and then we left alternate), the parser can be mid-sequence. Our next bytes (e.g. `(B)` for G0) can then be consumed as part of that sequence and misparsed, causing garbled output. **Fix:** Send CAN (0x18) at the start of the relay; CAN cancels any escape/control sequence in progress and returns the parser to ground (see vt100.net “A parser for DEC’s ANSI-compatible video terminals”). We send CAN once right after `set_real_tty_relay_raw()`, then our reset (G0/G1, SGR, wrap, keypad, scroll, cursor).

**Writer ordering:** Main and relay both write to the same underlying fd 1 (stdout). We flush the backend and then `std::io::stdout().flush()` in main before calling the relay so backend output is on the wire before relay writes; avoids reorder or double buffering.

**Termios:** We cache the “relay raw” termios from the first run and reuse it on every run so we always apply the same raw state (OPOST off, etc.) regardless of what the TUI did in between.

**Other measures:** Numeric keypad (`ESC >`), scroll region reset (`ESC [ r`), move cursor to bottom (`ESC [ 999 ; 999 H \r`) so we append below previous session; no screen clear.

**DECSTR (soft reset):** To fix block character (█) and partial prompt (`]`) from alternate character set / leftover SGR, we send **DECSTR** (`ESC [ ! p`) at relay start (after CAN). DECSTR resets character sets (G0/G1/GL/GR) to default, SGR to normal, margins, and cursor to home; it does **not** clear the screen. DECSTR disables autowrap, so we send `ESC [ ? 7 h` immediately after to re-enable wrap. Then G0/G1 and SGR again for terminals that don’t support DECSTR.

**macOS Terminal.app:** On macOS we skip DECSTR (`ESC [ ! p`) in the relay reset; Terminal.app does not reliably support it. We use only well-supported sequences: CAN, wrap, G0/G1, SGR, keypad, scroll region, cursor to bottom. Alternate screen uses `?1049`. TIOCGWINSZ on macOS uses `c_ulong` for ioctl.

**Single-writer flow:** To avoid reorder or corruption from mixing backend and direct fd 1, we use one writer for everything after LeaveAlternateScreen: (1) **prepare_for_relay()** sets relay raw and returns saved termios. (2) Main sends LeaveAlternateScreen (backend), flush. (3) Main writes the full relay reset sequence and `\r\n` **to stdout** via `Subshell::write_relay_reset_sequence(&mut stdout)`. (4) Main calls **run_relay_until_ctrl_o(..., Some(prepared))** so the relay skips set_raw and reset and only runs the PTY loop; it restores termios from `prepared` on return. All bytes after the switch (reset, \r\n, PTY output) go through stdout in strict order.

---

## Solution: Second Ctrl+O uglification (macOS Terminal.app)

**Symptom:** On the second (and later) Ctrl+O toggle to the subshell, output was uglified: partial prompt (`]`), block character (█), column misalignment, truncated filenames (e.g. `ls`). First Ctrl+O worked; returning to TUI and toggling again broke the display.

**Root cause:** Using the **crossterm backend** for the “leave alternate screen + show cursor + disable mouse” transition, then writing the reset sequence and PTY output to **stdout**, split the transition across two writers and two termios states. On macOS Terminal.app (and possibly others), that led to wrong terminal state on the second run: parser mid-sequence, wrong character set, or buffer/order quirks.

**Working solution (do not change without testing on macOS Terminal.app):**

1. **Do not use the backend for the switch.** Do **not** call `execute!(terminal.backend_mut(), LeaveAlternateScreen, Show, DisableMouseCapture)` when handing the terminal to the shell.

2. **Single writer for the whole transition.**  
   - Flush backend and stdout (so any pending TUI output is sent).  
   - Call **`prepare_for_relay()`** (sets relay raw on fd 1, returns saved termios).  
   - Write **all** of the following to **stdout** via **`Subshell::write_relay_reset_sequence(&mut stdout)`**:  
     - Leave alternate: `\x1b[?1049l\x1b[?47l`  
     - Show cursor: `\x1b[?25h`  
     - Disable mouse: `\x1b[?1002l\x1b[?1006l`  
     - CAN: `\x18`  
     - On non-macOS only: DECSTR `\x1b[!p` (skip on macOS Terminal.app)  
     - Wrap, G0/G1, SGR: `\x1b[?7h\x1b(B\x1b)B\x1b[0m`  
     - Numeric keypad: `\x1b>`  
     - Scroll region + cursor to bottom: `\x1b[r\x1b[999;999H\r`  
   - Then write `\r\n` to stdout and flush.  
   - Call **`run_relay_until_ctrl_o(..., Some(prepared))`** so the relay skips set_raw and reset and only runs the PTY loop; it restores termios from `prepared` on return.

3. **Code locations.**  
   - **main.rs** `AppAction::Suspend` and `AppAction::RunCommand`: flush, `prepare_for_relay()`, `write_relay_reset_sequence(stdout)`, `\r\n`, flush, then relay (with `Some(prepared)`). No `execute!(LeaveAlternateScreen, ...)` for the switch.  
   - **subshell.rs** `write_relay_reset_sequence()`: implements the exact byte sequence above; must include leave alternate, show cursor, and disable mouse so the backend is not needed for the switch.

4. **macOS Terminal.app specifics.**  
   - Skip DECSTR (`\x1b[!p`) on macOS in the relay reset; Terminal.app does not support it reliably.  
   - TIOCGWINSZ on macOS uses `c_ulong` for ioctl (see `resize_pty_to_terminal`).

**If you change this:** Re-test repeatedly: first Ctrl+O (shell), return to TUI, second Ctrl+O (shell), run `ls`. The second shell view must show correct prompt and aligned `ls` output. Test on macOS Terminal.app at minimum.

---

### Command line (`filemanager/command.c`)

- Enter on command line: get command string, then call `shell_execute(command, flags)` (which goes through `do_execute` / `do_executev` in execute.c).
- Special handling for `cd` and other built-ins.
- MC uses F6 (“Move”) to focus the command line; unhandled keys can be sent to the command line widget (`MSG_UNHANDLED_KEY` → `send_message (cmdline, ... MSG_KEY)`).

### Run command from command line (morning-commander)

- **RunCommand** runs in the **subshell** (MC-style): leave alternate screen, get/create subshell, send `cd 'cwd'` and `cmd` to the PTY, then relay until Ctrl+O. Command output and the shell prompt stay visible; user presses **Ctrl+O** to return to panels. So the command prompt is always available and the command output is kept.

### Panel ↔ command line flow (morning-commander)

- **Single source of truth:** `AppState::focus` (`Panel` | `CommandLine`). All key dispatch branches on `focus` first; no key is handled by both.
- **Panel → command line:** Type a printable character (focus moves and char is inserted), or press **F6** (focus only, MC-style).
- **Command line → panel:** **Tab** or **Esc** (focus returns to active panel; command line text is kept; use Ctrl+C to clear).
- **Between panels:** **Tab** when focus is Panel switches left/right panel.
- After run command: no fixed delay; wait for any key with 50 ms poll, then enter alternate → drain → refresh panels (restore selection) → draw.
