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
- We: resize PTY **before fork** (after openpty, set size on master so the child never sees wrong size), and again at start of `run_relay_until_ctrl_o()` so each relay run uses current terminal size. Use 4K relay buffer (MC uses PTY_BUFFER_SIZE 512) so long lines aren’t fragmented and we don’t lose output.

**Morning-commander Ctrl+O implementation** (see `main.rs` `AppAction::Suspend` and `subshell.rs` `run_relay_until_ctrl_o`):

- Leave alternate, show cursor, disable mouse → flush.
- Create subshell (PTY) with cwd from active panel; run relay: stdin ↔ PTY until Ctrl+O byte (0x0F); do not clear main screen.
- On return: Enter alternate, hide cursor, enable mouse → drain queue → clear → draw → drain again (handle any FocusGained). No `flush()` after `draw()`.

### Command line (`filemanager/command.c`)

- Enter on command line: get command string, then call `shell_execute(command, flags)` (which goes through `do_execute` / `do_executev` in execute.c).
- Special handling for `cd` and other built-ins.
- MC uses F6 (“Move”) to focus the command line; unhandled keys can be sent to the command line widget (`MSG_UNHANDLED_KEY` → `send_message (cmdline, ... MSG_KEY)`).

### Run command from command line (morning-commander)

- **RunCommand** runs on the **real terminal** (no subshell/relay): leave alternate screen, disable raw mode, echo `$ cmd`, run `sh -c "cmd"` with inherited stdio, wait for key, re-enter TUI. Command output goes **directly** to the terminal, so it is never uglified by relay/PTY. Latest results and the last command are most important; we echo the command so the user sees it.
- **If we ever buffer output:** keep latest (crop from top), not from bottom.

### Panel ↔ command line flow (morning-commander)

- **Single source of truth:** `AppState::focus` (`Panel` | `CommandLine`). All key dispatch branches on `focus` first; no key is handled by both.
- **Panel → command line:** Type a printable character (focus moves and char is inserted), or press **F6** (focus only, MC-style).
- **Command line → panel:** **Tab** or **Esc** (focus returns to active panel; command line text is kept; use Ctrl+C to clear).
- **Between panels:** **Tab** when focus is Panel switches left/right panel.
- After run command: no fixed delay; wait for any key with 50 ms poll, then enter alternate → drain → refresh panels (restore selection) → draw.
