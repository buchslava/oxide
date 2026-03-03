# Shell, PTY, Terminals, and File Descriptors

This document explains the theory behind Oxide's subshell (Ctrl+O) feature and how it relates to the implementation. It covers pseudo-terminals (PTY), file descriptors, terminal modes (termios), and I/O multiplexing.

## Overview

Oxide provides a Midnight Commander–style **Ctrl+O** shell toggle: pressing Ctrl+O suspends the TUI and hands the terminal to an interactive subshell. A second Ctrl+O returns to the panels. The subshell runs in a **PTY** (pseudo-terminal), and the TUI acts as a relay between the user's real terminal (stdin/stdout) and the PTY.

---

## File Descriptors (FD)

### Theory

A **file descriptor** is a non-negative integer that refers to an open I/O resource (file, pipe, socket, or device). On Unix, the first three FDs are reserved:

| FD | Name    | Purpose                    |
|----|---------|----------------------------|
| 0  | stdin   | Standard input             |
| 1  | stdout  | Standard output            |
| 2  | stderr  | Standard error             |

FDs are process-local: after `fork()`, the child inherits copies. They can be duplicated with `dup2()` or closed with `close()`.

### Related Documentation

- [dup2(2)](https://man7.org/linux/man-pages/man2/dup2.2.html) — duplicate a file descriptor
- [close(2)](https://man7.org/linux/man-pages/man2/close.2.html)

### In Oxide

- The TUI reads from FD 0 (stdin) and writes to FD 1 (stdout).
- The subshell uses `dup2(slave_fd, 0)`, `dup2(slave_fd, 1)`, `dup2(slave_fd, 2)` so the shell’s stdio is tied to the PTY slave.
- Raw FD I/O uses `nix::unistd::read` / `nix::unistd::write` and `libc::fcntl` for non-blocking mode.

---

## Pseudo-Terminals (PTY)

### Theory

A **pseudo-terminal** is a pair of virtual character devices: **master** and **slave**.

- **Master** — Held by the parent (Oxide). Data written here is fed to the slave as terminal input. Data written by the process on the slave can be read from the master.
- **Slave** — Behaves like a real terminal. A process (e.g. shell) opening the slave sees a normal terminal; it can do line editing, job control, etc.

Data flows both ways. Writing Ctrl+C to the master sends SIGINT to the foreground process group on the slave, as with a real terminal.

UNIX 98 PTYs are the standard; Oxide uses `openpty()` to create them.

### Related Documentation

- [pty(7)](https://man7.org/linux/man-pages/man7/pty.7.html) — pseudo-terminal interfaces
- [openpty(3)](https://man7.org/linux/man-pages/man3/openpty.3.html)
- [pts(4)](https://man7.org/linux/man-pages/man4/pts.4.html) — slave device namespace

### In Oxide

- `Subshell::spawn()` uses `nix::pty::openpty()` to create the pair.
- The parent keeps `master_fd`; the child receives the slave and uses it for stdio.
- The relay loop reads from stdin (FD 0) and master, writes stdin bytes to master and master output to stdout (FD 1).

---

## termios and Terminal Modes

### Theory

Terminal behavior is controlled by `termios`: line discipline, echo, special characters, etc. Two important modes:

#### Cooked (canonical) mode

- Input is line-buffered (waits for newline).
- Echo, line editing (backspace, etc.), and special chars (Ctrl+C, Ctrl+Z) are handled by the kernel.
- Typical for interactive shells.

#### Raw mode

- Characters are delivered as typed, without buffering or editing.
- No echo.
- Needed to detect exact keypresses (e.g. Ctrl+O).

Flags in `c_lflag`:

| Flag    | Effect when set                          |
|---------|-------------------------------------------|
| ICANON  | Canonical (cooked) input                  |
| ECHO    | Echo typed characters                     |
| ISIG    | Generate signals on Ctrl+C, Ctrl+Z, etc.  |
| IEXTEN  | Extended local processing                 |
| IXON    | Software flow control (Ctrl+S/Q)          |
| OPOST   | Output post-processing                    |
| ONLCR   | Map newline to carriage return + newline  |

Functions: `tcgetattr()` (get), `tcsetattr()` (set), `tcflush()` (flush buffers).

### Related Documentation

- [termios(3)](https://man7.org/linux/man-pages/man3/termios.3.html)
- [tcsetattr(3p)](https://man7.org/linux/man-pages/man3/tcsetattr.3p.html)
- [Canonical vs noncanonical mode](https://www.gnu.org/software/libc/manual/html_node/Canonical-or-Not.html)

### In Oxide

1. **Real terminal (stdout)** — Set to raw for relay (`set_real_tty_relay_raw`) so Ctrl+O is seen as byte 0x0F; restored when returning to the TUI.
2. **PTY slave** — Left in cooked mode (`set_pty_slave_cooked_mode`) so the shell behaves normally. ISIG is enabled so that Ctrl+C sends SIGINT to the foreground process group (e.g. interrupting `tail -f`).
3. **Caching** — Raw mode for relay is cached in `RELAY_RAW_TERMIOS` and reused, so subsequent Ctrl+O uses the same state instead of whatever the TUI left behind.

---

## Process and Controlling Terminal

### Theory

- `fork()` creates a child that shares the parent’s controlling terminal.
- `setsid()` creates a new session and detaches from the controlling terminal.
- `TIOCSCTTY` makes the given terminal the session’s controlling terminal (needed so the shell has a proper tty).
- `dup2(slave_fd, 0/1/2)` redirects stdio to the slave before `exec()`.

### Related Documentation

- [setsid(2)](https://man7.org/linux/man-pages/man2/setsid.2.html)
- [TIOCSCTTY](https://man7.org/linux/man-pages/man2/ioctl_tty.2.html)

### In Oxide

In the child after `fork()`:

1. `setsid()` — new session.
2. `ioctl(slave_fd, TIOCSCTTY, 0)` — make slave the controlling terminal.
3. `set_pty_slave_cooked_mode(slave_fd)` — configure slave for shell use.
4. `dup2(slave_fd, 0)`, `dup2(slave_fd, 1)`, `dup2(slave_fd, 2)` — redirect stdio.
5. `close(slave_fd)` — child no longer needs the original slave FD.
6. `exec(shell)` — run interactive shell (e.g. `$SHELL -i`).

---

## I/O Multiplexing with poll()

### Theory

`poll()` lets a process wait on multiple FDs. When any of them has data (or is writable), `poll()` returns. This avoids blocking on one FD while data is available on another.

```c
struct pollfd fds[] = {
    { .fd = stdin_fd,  .events = POLLIN },
    { .fd = pty_fd,    .events = POLLIN },
};
poll(fds, 2, timeout);
// Check fds[0].revents, fds[1].revents for POLLIN
```

`select()` is older; `poll()` is cleaner and avoids FD limits.

### Related Documentation

- [poll(2)](https://man7.org/linux/man-pages/man2/poll.2.html)

### In Oxide

`run_relay_until_ctrl_o` uses `poll()` to monitor:

- FD 0 (stdin) — user input
- `master_fd` — shell output

A loop reads from whichever is ready and forwards data. Ctrl+O on stdin is intercepted and stops the relay.

---

## Non-Blocking I/O and EAGAIN

### Theory

A read on a blocking FD blocks until data is available. With `O_NONBLOCK`:

- `read()` returns immediately with whatever is available.
- If no data: returns -1 with `errno == EAGAIN` (or `EWOULDBLOCK`).
- Avoids deadlocks when data may arrive later (e.g. between `poll()` and `read()`).

### In Oxide

`read_pty_nonblock()` temporarily sets `O_NONBLOCK` on the master FD, reads, then restores flags. This prevents lockups when the shell does things like `tcflush()` that can invalidate data between `poll()` and `read()`.

---

## Window Size (TIOCGWINSZ / TIOCSWINSZ)

### Theory

Terminals have a logical size (columns, rows). Programs like `ls` and vim use it for layout. The kernel stores this in a `winsize` struct; `TIOCGWINSZ` reads it, `TIOCSWINSZ` writes it.

### In Oxide

`resize_pty_to_terminal()`:

1. Reads the real terminal’s size with `TIOCGWINSZ` on fd 1 (stdout).
2. Writes it to the PTY master with `TIOCSWINSZ`.

This keeps the subshell’s idea of screen size in sync with the actual terminal.

---

## Alternate Screen and Single-Writer Flow

### Theory

Terminals support an alternate screen buffer. Swapping between main and alternate keeps the scrollback of one intact while the other is used.

- Enter alternate: `\x1b[?1049h` (and related sequences)
- Leave alternate: `\x1b[?1049l`

For a clean transition when switching between TUI and subshell:

- The TUI uses Crossterm’s alternate screen.
- When handing off to the subshell, the process must leave alternate and reset terminal state (wrap, character sets, mouse, etc.) before the shell takes over.
- To avoid mixed output, all “switch” bytes should go through a single writer (stdout), not split between Crossterm’s backend and manual writes.

### In Oxide

1. `prepare_for_relay()` — save current termios, set relay raw.
2. `write_relay_reset_sequence()` — write all transition bytes to stdout: leave alternate, show cursor, disable mouse, CAN, reset SGR, keypad, scroll, etc.
3. `run_relay_until_ctrl_o()` — relay loop; on Ctrl+O, drain PTY output, restore termios.
4. Back in `main.rs`: enter alternate again, clear, redraw TUI.

`write_relay_reset_sequence()` is called on raw stdout before handing to the relay, ensuring one consistent output path.

---

## Ctrl+O Detection and Encodings

### Theory

Ctrl+O is byte 0x0F. Modern terminals may send it as an escape sequence (e.g. CSI sequences with modifier info) instead of the raw byte.

### In Oxide

`relay_stdin_chunk()` handles:

- Plain `0x0F`
- Kitty protocol: `\x1b[111;5u`
- Modify Other Keys: `\x1b[27;5;111~`

If Ctrl+O appears in the middle of a chunk, bytes before it are forwarded to the PTY; the Ctrl+O itself is consumed and triggers relay exit. A `carry` buffer handles sequences split across reads; a `trailing_ctrl_o_prefix_len` keeps incomplete sequences for the next chunk.

---

## Implementation Flow Summary

| Step | Location | Action |
|------|----------|--------|
| 1 | `main.rs` | Enable raw mode, enter alternate screen (Crossterm) |
| 2 | `main.rs` | Preload subshell on startup |
| 3 | On Ctrl+O | `prepare_for_relay()` → set relay raw, save termios |
| 4 | | `write_relay_reset_sequence()` to stdout (leave alternate, reset, etc.) |
| 5 | | Get/create subshell; `run_cd_then_relay()` or `run_command_then_relay()` |
| 6 | `subshell.rs` | `resize_pty_to_terminal()` |
| 7 | | Relay loop: `poll(stdin, master)`, read/write in both directions |
| 8 | | On Ctrl+O: drain PTY, restore termios, return |
| 9 | `main.rs` | Enter alternate, clear, redraw TUI |

---

## References

- [pty(7) — Linux manual page](https://man7.org/linux/man-pages/man7/pty.7.html)
- [termios(3)](https://man7.org/linux/man-pages/man3/termios.3.html)
- [poll(2)](https://man7.org/linux/man-pages/man2/poll.2.html)
- [openpty(3)](https://man7.org/linux/man-pages/man3/openpty.3.html)
- [dup2(2)](https://man7.org/linux/man-pages/man2/dup2.2.html)
- [GNU Libc: Canonical or Not](https://www.gnu.org/software/libc/manual/html_node/Canonical-or-Not.html)
- [Beej's Notes on Terminal I/O](https://beej.us/298C/notes/notes7.html)
