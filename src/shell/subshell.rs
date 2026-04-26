//! PTY subshell (toggle shell); command line runs in the original terminal.
//!
//! Return to panels: **Ctrl+O** (`0x0F`, MC-style) or **Ctrl+X** then plain **`o`** / **`O`** (see REFERENCE.md).
//! Leave alternate screen, relay stdin↔PTY until one of those is read from stdin; then caller
//! re-enters alternate and redraws. Raw mode stays on; the subshell runs in a PTY with its own termios.
//!
//! ## `RunCommand` FIFO and early reopen (`sudo -s`)
//! The shell line is `eval '…'; printf '\\n' > fifo` — the FIFO byte is the **authoritative** “this
//! `eval` finished” signal (e.g. after `exit` leaves `sudo -s`). Early panel reopen returns from the
//! relay **while `eval` is still running**. If the FIFO reader were dropped then, `OwnedFifo`’s
//! `Drop` would **unlink** the path and the eventual `printf` would not signal Oxide — so **`exit`**
//! would appear to do nothing. The completion reader is therefore **moved** to
//! [`Subshell::pending_command_done`] (and polled on every later relay, including Suspend) until
//! `printf` runs.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::cell::RefCell;
#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use nix::sys::stat::Mode;
#[cfg(unix)]
use nix::unistd::mkfifo;
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
const CTRL_O: u8 = 0x0F;
#[cfg(unix)]
const CTRL_O_KITTY: &[u8] = b"\x1b[111;5u";
#[cfg(unix)]
const CTRL_O_MODIFY_OTHER_KEYS: &[u8] = b"\x1b[27;5;111~";

#[cfg(unix)]
const CTRL_X: u8 = 0x18;
/// Kitty keyboard protocol: `CSI u` with Unicode codepoint for `x` (120).
#[cfg(unix)]
const CTRL_X_KITTY: &[u8] = b"\x1b[120;5u";
/// XTerm *modifyOtherKeys* / CSI `27` form for Ctrl+X (codepoint 120 = `x`).
#[cfg(unix)]
const CTRL_X_MODIFY_OTHER_KEYS: &[u8] = b"\x1b[27;5;120~";

// Cached "relay raw" termios from first run; reused on 2nd+ run so we always apply the same state
// (avoids TUI-modified termios causing uglified output). Doc omitted: thread_local! macro does not
// propagate doc comments.
#[cfg(unix)]
std::thread_local!(static RELAY_RAW_TERMIOS: std::cell::RefCell<Option<libc::termios>> = std::cell::RefCell::new(None));

/// Saved termios from prepare_for_relay(); pass to run_relay_until_ctrl_o so all pre-relay bytes go through one writer (stdout) and relay skips duplicate setup.
#[cfg(unix)]
pub struct PreparedRelay(Option<libc::termios>);
#[cfg(not(unix))]
pub struct PreparedRelay;

/// How a PTY relay session ended (used to show a countdown before restoring the panel UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayExit {
    /// User pressed Ctrl+O or Ctrl+X then O, stdin closed, or auto-reopen was off / not applicable.
    Manual,
    /// Command finished (or Ctrl+C with auto-reopen); caller shows a delay toast then restores panels.
    AutoReopenDelay(std::time::Duration),
}

/// Persistent subshell: command line runs here; **Ctrl+O** or **Ctrl+X** then **O** toggles full-screen relay.
#[cfg(unix)]
pub struct Subshell {
    master_fd: i32,
    child_pid: i32,
    /// `st_rdev` of the PTY slave (for matching `proc_bsdinfo.e_tdev` when `tcgetpgrp` is unreliable).
    slave_st_rdev: u64,
    /// PTY name for `ps -t` (path without `/dev/`, e.g. `ttys012`, `pts/4`).
    ps_tty_arg: String,
    /// When we auto-reopen panels while `eval cmd; printf > fifo` is still running (e.g. `sudo -s`),
    /// keep the completion FIFO reader open here until the shell finally runs `printf` — otherwise
    /// `Drop` unlinks the path and `exit` from the inner shell never signals completion.
    pending_command_done: RefCell<Option<OwnedFifo>>,
    /// Countdown duration to use when [`Self::pending_command_done`] fires (mirrors active auto-exit).
    pending_done_delay: RefCell<Option<std::time::Duration>>,
}

/// FIFO opened for read before the shell runs `printf '\\n' > '…'` after `eval` finishes — PTY bytes
/// are relayed unfiltered (sudo prompts, etc.). `Drop` unlinks the path.
#[cfg(unix)]
struct OwnedFifo {
    file: std::fs::File,
    path: PathBuf,
}

#[cfg(unix)]
impl OwnedFifo {
    /// One path per Oxide process + subshell child. `remove_file` before `mkfifo` clears a stale
    /// FIFO or a **regular file** accidentally created if someone re-runs `printf > path` after the
    /// real FIFO was already unlinked (shell `>` does not recreate a FIFO).
    fn open_for_subshell(child_pid: i32) -> io::Result<Self> {
        let parent = std::process::id();
        let path =
            std::env::temp_dir().join(format!("oxide_cmd_{}_{}.fifo", parent, child_pid));
        let _ = std::fs::remove_file(&path);
        mkfifo(&path, Mode::S_IRUSR | Mode::S_IWUSR).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("mkfifo: {}", e))
        })?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)?;
        Ok(Self { file, path })
    }

    fn as_raw_fd(&self) -> i32 {
        self.file.as_raw_fd()
    }
}

#[cfg(unix)]
impl Drop for OwnedFifo {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Legacy: scan PTY stream for a completion token (fragile with shell echo / short lines).
#[cfg(unix)]
struct StreamMonitoredCompletion {
    completion_lf: Vec<u8>,
    completion_crlf: Vec<u8>,
    helper_echo: Vec<u8>,
}

#[cfg(unix)]
struct AutoExitConfig {
    delay: std::time::Duration,
    completion: AutoExitCompletion,
}

/// `sudo -s` / `sudo -i` style: `printf` to [`FifoCompletion::command_done`] runs only after the inner
/// shell exits. Optionally we still reopen panels early when the session reads as root (see
/// [`SudoEarlyPanelReopen`]), after password / NOPASS timing gates.
#[cfg(unix)]
fn cmd_allocates_interactive_sudo_shell(cmd: &str) -> bool {
    let low = cmd.trim().to_ascii_lowercase();
    if !(low.starts_with("sudo ") || low == "sudo") {
        return false;
    }
    low.contains(" -s")
        || low.contains(" -i")
        || low.contains(" -si")
        || low.starts_with("sudo su")
        || low == "sudo -s"
        || low == "sudo -i"
}

/// Early auto-reopen for interactive sudo shells only: avoids firing while the user is still typing
/// the password (no Enter yet). Root detection uses the same probe as chrome (incl. `ps` fallback)
/// **only after** those gates — plus a short stability streak. Timer-driven: idle root prompts send
/// no PTY bytes, so this must tick on every `poll` wake, not only when the PTY has data.
#[cfg(unix)]
struct SudoEarlyPanelReopen {
    scan_tail: Vec<u8>,
    password_prompt_seen: bool,
    password_submit_at: Option<std::time::Instant>,
    /// NOPASS path: deadline from relay start (set in [`SudoEarlyPanelReopen::new`]).
    relay_started_at: std::time::Instant,
    last_poll_at: Option<std::time::Instant>,
    /// Consecutive root-positive samples at poll interval.
    root_streak: u32,
}

#[cfg(unix)]
impl SudoEarlyPanelReopen {
    fn new() -> Self {
        Self {
            scan_tail: Vec::new(),
            password_prompt_seen: false,
            password_submit_at: None,
            relay_started_at: std::time::Instant::now(),
            last_poll_at: None,
            root_streak: 0,
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RelayFifoRole {
    /// `printf` for the current `run_command_then_relay` line.
    ActiveCommandDone,
    /// Stashed reader from an earlier early-reopen while `eval` was still running.
    PendingCommandDone,
}

#[cfg(unix)]
struct FifoCompletion {
    /// Written when `eval '…'; printf` finishes (non-interactive commands, or after `exit` from `sudo -s`).
    /// Temporarily [`None`] after early panel reopen (fifo moved to [`Subshell::pending_command_done`]).
    command_done: Option<OwnedFifo>,
    /// Present only for [`cmd_allocates_interactive_sudo_shell`]: reopen panels when root shell is up.
    sudo_early_panels: Option<SudoEarlyPanelReopen>,
}

#[cfg(unix)]
enum AutoExitCompletion {
    Fifo(FifoCompletion),
    Stream(StreamMonitoredCompletion),
}

#[cfg(unix)]
impl Subshell {
    fn find_subsequence(
        haystack: &[u8],
        needle: &[u8],
    ) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    fn trailing_ctrl_o_prefix_len(buf: &[u8]) -> usize {
        let patterns = [CTRL_O_KITTY, CTRL_O_MODIFY_OTHER_KEYS];
        let mut keep = 0usize;
        for pat in patterns {
            let max_k = pat.len().saturating_sub(1).min(buf.len());
            for k in 1..=max_k {
                if buf[buf.len() - k..] == pat[..k] {
                    keep = keep.max(k);
                }
            }
        }
        keep
    }

    fn trailing_ctrl_x_prefix_len(buf: &[u8]) -> usize {
        let patterns = [CTRL_X_KITTY, CTRL_X_MODIFY_OTHER_KEYS];
        let mut keep = 0usize;
        for pat in patterns {
            let max_k = pat.len().saturating_sub(1).min(buf.len());
            for k in 1..=max_k {
                if buf[buf.len() - k..] == pat[..k] {
                    keep = keep.max(k);
                }
            }
        }
        keep
    }

    fn earliest_ctrl_o_in_carry(carry: &[u8]) -> Option<(usize, usize)> {
        let plain_pos = carry.iter().position(|&b| b == CTRL_O).map(|p| (p, 1usize));
        let kitty_pos =
            Self::find_subsequence(carry, CTRL_O_KITTY).map(|p| (p, CTRL_O_KITTY.len()));
        let mok_pos = Self::find_subsequence(carry, CTRL_O_MODIFY_OTHER_KEYS)
            .map(|p| (p, CTRL_O_MODIFY_OTHER_KEYS.len()));
        [plain_pos, kitty_pos, mok_pos]
            .into_iter()
            .flatten()
            .min_by_key(|t| t.0)
    }

    /// Feed stdin bytes into relay and intercept **Ctrl+O** (and Kitty / modifyOtherKeys forms) or
    /// **Ctrl+X** then plain **`o`** / **`O`**. Returns true when the relay should exit.
    fn relay_stdin_chunk(
        &self,
        carry: &mut Vec<u8>,
        chunk: &[u8],
        chord_withheld: &mut Option<Vec<u8>>,
    ) -> io::Result<bool> {
        carry.extend_from_slice(chunk);

        loop {
            if let Some(w) = chord_withheld.as_ref() {
                if carry.is_empty() {
                    return Ok(false);
                }
                let b0 = carry[0];
                if b0 == b'o' || b0 == b'O' {
                    carry.drain(..1);
                    chord_withheld.take();
                    return Ok(true);
                }
                Self::write_all_fd(self.master_fd, w)?;
                chord_withheld.take();
                continue;
            }

            if let Some((pos, len)) = Self::earliest_ctrl_o_in_carry(carry) {
                if pos > 0 {
                    Self::write_all_fd(self.master_fd, &carry[..pos])?;
                }
                carry.drain(..pos + len);
                return Ok(true);
            }

            let plain_x = carry
                .iter()
                .position(|&b| b == CTRL_X)
                .map(|p| (p, 1usize));
            let kitty_x =
                Self::find_subsequence(carry, CTRL_X_KITTY).map(|p| (p, CTRL_X_KITTY.len()));
            let mok_x = Self::find_subsequence(carry, CTRL_X_MODIFY_OTHER_KEYS)
                .map(|p| (p, CTRL_X_MODIFY_OTHER_KEYS.len()));

            let earliest = [plain_x, kitty_x, mok_x]
                .into_iter()
                .flatten()
                .min_by_key(|t| t.0);

            if let Some((pos, x_len)) = earliest {
                if pos > 0 {
                    Self::write_all_fd(self.master_fd, &carry[..pos])?;
                    carry.drain(..pos);
                    continue;
                }
                if carry.len() == x_len {
                    *chord_withheld = Some(carry[..x_len].to_vec());
                    carry.drain(..x_len);
                    return Ok(false);
                }
                let follow = carry[x_len];
                if follow == b'o' || follow == b'O' {
                    carry.drain(..x_len + 1);
                    return Ok(true);
                }
                Self::write_all_fd(self.master_fd, &carry[..x_len + 1])?;
                carry.drain(..x_len + 1);
                continue;
            }

            let keep = Self::trailing_ctrl_o_prefix_len(carry).max(Self::trailing_ctrl_x_prefix_len(carry));
            let forward_len = carry.len().saturating_sub(keep);
            if forward_len > 0 {
                Self::write_all_fd(self.master_fd, &carry[..forward_len])?;
                carry.drain(..forward_len);
            }
            return Ok(false);
        }
    }

    /// Read from PTY in non-blocking mode (MC: read_nonblock). Avoids lockup when slave tcflush() revokes data between poll and read.
    fn read_pty_nonblock(
        fd: i32,
        buf: &mut [u8],
    ) -> io::Result<Option<usize>> {
        use nix::errno::Errno;
        let old_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if old_flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe {
            libc::fcntl(
                fd,
                libc::F_SETFL,
                old_flags | libc::O_NONBLOCK,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let result = match nix::unistd::read(fd, buf) {
            Ok(0) => Ok(Some(0)),
            Ok(n) => Ok(Some(n)),
            Err(e) if e == Errno::EAGAIN || e == Errno::EWOULDBLOCK => Ok(None),
            Err(Errno::EINTR) => Ok(None),
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        };
        let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, old_flags) };
        result
    }

    fn write_all_fd(
        fd: i32,
        mut data: &[u8],
    ) -> io::Result<()> {
        use nix::errno::Errno;
        while !data.is_empty() {
            match nix::unistd::write(
                unsafe { BorrowedFd::borrow_raw(fd) },
                data,
            ) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "short write while relaying PTY data",
                    ));
                }
                Ok(n) => data = &data[n..],
                Err(Errno::EINTR) => continue,
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
            }
        }
        Ok(())
    }

    /// Stream PTY bytes to stdout while suppressing the auto-return **completion** token
    /// (`marker + '\n'` from `printf '%s\n'`).
    /// Returns true when that token is detected (possibly across chunk boundaries).
    fn write_pty_chunk_without_marker(
        pending: &mut Vec<u8>,
        chunk: &[u8],
        completion_lf: &[u8],
        completion_crlf: &[u8],
        helper_echo: &[u8],
    ) -> io::Result<bool> {
        pending.extend_from_slice(chunk);

        loop {
            let echo_pos = Self::find_subsequence(pending, helper_echo);
            let done_pos = Self::find_subsequence(pending, completion_crlf)
                .or_else(|| Self::find_subsequence(pending, completion_lf));
            match (echo_pos, done_pos) {
                (Some(e), Some(m)) if e < m => {
                    if e > 0 {
                        Self::write_all_fd(1, &pending[..e])?;
                    }
                    pending.drain(..e + helper_echo.len());
                    continue;
                }
                (Some(e), None) => {
                    if e > 0 {
                        Self::write_all_fd(1, &pending[..e])?;
                    }
                    pending.drain(..e + helper_echo.len());
                    continue;
                }
                (_, Some(m)) => {
                    if m > 0 {
                        Self::write_all_fd(1, &pending[..m])?;
                    }
                    pending.clear();
                    return Ok(true);
                }
                (None, None) => break,
            }
        }

        // Tail withheld so a completion token split across PTY reads is not flushed early.
        let keep = completion_lf
            .len()
            .max(completion_crlf.len())
            .saturating_sub(1);
        if pending.len() > keep {
            let flush_len = pending.len() - keep;
            Self::write_all_fd(1, &pending[..flush_len])?;
            pending.drain(..flush_len);
        }
        Ok(false)
    }

    fn flush_pty_pending_without_marker(
        pending: &mut Vec<u8>,
        helper_echo: &[u8],
    ) -> io::Result<()> {
        if pending.is_empty() {
            return Ok(());
        }
        if let Some(pos) = Self::find_subsequence(pending, helper_echo) {
            if pos > 0 {
                Self::write_all_fd(1, &pending[..pos])?;
            }
            pending.clear();
            return Ok(());
        }
        Self::write_all_fd(1, pending)?;
        pending.clear();
        Ok(())
    }

    /// Flush any PTY output to stdout (MC: show prompt after " \b" before relay).
    /// Ensures the command prompt is visible when toggling to the shell. Uses a longer window
    /// so a newly spawned subshell has time to print its first prompt (MC inits subshell at startup).
    fn flush_pty_prompt_to_stdout(master_fd: i32) -> io::Result<()> {
        use nix::errno::Errno;
        use nix::poll::{poll, PollFd, PollFlags};
        use std::time::{Duration, Instant};

        let mut pty_buf = [0u8; 4096];
        let start = Instant::now();
        let mut last_data_at = start;
        let quiet_window = Duration::from_millis(50);
        let hard_limit = Duration::from_millis(500);

        while start.elapsed() < hard_limit {
            let mut fds = [PollFd::new(
                unsafe { BorrowedFd::borrow_raw(master_fd) },
                PollFlags::POLLIN | PollFlags::POLLHUP,
            )];
            match poll(&mut fds, 10u16) {
                Ok(0) => {
                    if last_data_at.elapsed() >= quiet_window {
                        break;
                    }
                }
                Ok(_) => {
                    if !fds[0].revents().map_or(false, |r| {
                        r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP)
                    }) {
                        if last_data_at.elapsed() >= quiet_window {
                            break;
                        }
                        continue;
                    }
                    match Self::read_pty_nonblock(master_fd, &mut pty_buf)? {
                        Some(0) => break,
                        Some(n) => {
                            Self::write_all_fd(1, &pty_buf[..n])?;
                            last_data_at = Instant::now();
                        }
                        None => {}
                    }
                }
                Err(Errno::EINTR) => continue,
                Err(_) => break,
            }
        }
        Ok(())
    }

    /// After Ctrl+O (or Ctrl+X then O), flush any PTY bytes already produced so we don't cut escape/UTF-8 sequences
    /// in half and so the full prompt and latest command results are shown before re-entering alternate screen.
    fn drain_pty_output(master_fd: i32) -> io::Result<()> {
        use nix::errno::Errno;
        use nix::poll::{poll, PollFd, PollFlags};
        use std::time::{Duration, Instant};

        let mut pty_buf = [0u8; 4096];
        let start = Instant::now();
        let mut last_data_at = start;
        let quiet_window = Duration::from_millis(50);
        let hard_limit = Duration::from_millis(600);

        while start.elapsed() < hard_limit {
            let mut fds = [PollFd::new(
                unsafe { BorrowedFd::borrow_raw(master_fd) },
                PollFlags::POLLIN | PollFlags::POLLHUP,
            )];
            match poll(&mut fds, 10u16) {
                Ok(0) => {
                    if last_data_at.elapsed() >= quiet_window {
                        break;
                    }
                }
                Ok(_) => {
                    if !fds[0].revents().map_or(false, |r| {
                        r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP)
                    }) {
                        continue;
                    }
                    match Self::read_pty_nonblock(master_fd, &mut pty_buf)? {
                        Some(0) => break,
                        Some(n) => {
                            Self::write_all_fd(1, &pty_buf[..n])?;
                            last_data_at = Instant::now();
                        }
                        None => {}
                    }
                }
                Err(Errno::EINTR) => continue,
                Err(_) => break,
            }
        }
        Ok(())
    }

    /// Set PTY slave to cooked/shell termios (MC: tcsetattr(slave, &shell_mode)).
    /// The shell must see a normal terminal: ICANON, ECHO, ONLCR so output is correct.
    /// Disable VDISCARD so stray control keys never toggle kernel output discard on the slave.
    fn set_pty_slave_cooked_mode(slave_fd: i32) {
        let mut tio: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(slave_fd, &mut tio) } != 0 {
            return;
        }
        // Cooked input: canonical mode, echo, CR→NL; ISIG so Ctrl+C sends SIGINT to foreground (e.g. interrupt tail -f).
        tio.c_lflag |= libc::ICANON | libc::ECHO | libc::IEXTEN | libc::ISIG;
        tio.c_iflag |= libc::ICRNL;
        tio.c_iflag &= !libc::IXON; // pass ^S/^Q to shell (MC does this in raw_mode)
                                    // Cooked output: postprocess, \n → \r\n
        tio.c_oflag |= libc::OPOST | libc::ONLCR;
        tio.c_cc[libc::VMIN] = 1;
        tio.c_cc[libc::VTIME] = 0;
        // Disable VDISCARD so control keys don't toggle output discard (macOS/BSD)
        if libc::VDISCARD < libc::NCCS {
            tio.c_cc[libc::VDISCARD] = libc::_POSIX_VDISABLE as libc::cc_t;
        }
        #[cfg(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            tio.c_lflag &= !libc::FLUSHO;
        }
        let _ = unsafe { libc::tcsetattr(slave_fd, libc::TCSANOW, &tio) };
    }

    /// Set real tty to MC-style raw for relay (invoke_subshell: tcsetattr(STDOUT, &raw_mode)).
    /// Use STDOUT (fd 1) like MC. On 2nd+ run reuse cached relay-raw termios so we apply the same state as the first run (avoids TUI-modified termios causing uglified output).
    ///
    /// **macOS:** Crossterm’s raw mode is applied to stdin when it is a tty; we also apply the same
    /// relay-raw attributes to fd 0 when both 0 and 1 are ttys so stdin and stdout stay consistent
    /// for the relay (helps odd prompt / echo edge cases after leaving the alternate screen).
    fn set_real_tty_relay_raw() -> Option<libc::termios> {
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(1, &mut saved) } != 0 {
            return None;
        }
        RELAY_RAW_TERMIOS.with(|cell| {
            if cell.borrow().is_none() {
                let mut raw = saved;
                raw.c_lflag &= !(libc::ICANON | libc::ISIG | libc::ECHO);
                raw.c_iflag &= !(libc::IXON | libc::ICRNL);
                raw.c_oflag &= !libc::OPOST;
                raw.c_cc[libc::VMIN] = 1;
                raw.c_cc[libc::VTIME] = 0;
                cell.replace(Some(raw));
            }
            if let Some(ref r) = *cell.borrow() {
                // Drain queued TUI output before applying relay raw (avoids mid-sequence handoff).
                let _ = unsafe { libc::tcsetattr(1, libc::TCSADRAIN, r) };
                #[cfg(target_os = "macos")]
                if unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 } {
                    let _ = unsafe { libc::tcsetattr(0, libc::TCSADRAIN, r) };
                }
            }
        });
        Some(saved)
    }

    fn restore_real_tty(saved: Option<libc::termios>) {
        if let Some(tio) = saved {
            let _ = unsafe { libc::tcsetattr(1, libc::TCSANOW, &tio) };
            #[cfg(target_os = "macos")]
            if unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 } {
                let _ = unsafe { libc::tcsetattr(0, libc::TCSANOW, &tio) };
            }
        }
    }

    /// Set real tty to relay raw and return saved termios. Call before LeaveAlternateScreen so all bytes after (reset, relay) go through one path. Pass result to run_relay_until_ctrl_o(..., Some(prepared)).
    #[cfg(unix)]
    pub fn prepare_for_relay() -> PreparedRelay {
        PreparedRelay(Self::set_real_tty_relay_raw())
    }

    /// Write the full relay transition: leave alternate, show cursor, disable mouse, CAN, reset (wrap, G0/G1, SGR, keypad, scroll, cursor). Caller must set relay raw first and pass prepared to run_relay.
    #[cfg(unix)]
    pub fn write_relay_reset_sequence<W: Write>(w: &mut W) -> io::Result<()> {
        w.write_all(b"\x1b[?1049l\x1b[?47l")?; // leave alternate (main screen)
        w.write_all(b"\x1b[?25h")?; // show cursor (DECTCEM)
        w.write_all(b"\x1b[?1002l\x1b[?1006l")?; // disable mouse (X10, SGR)
        // Crossterm/Ratatui stacks: clear modes that can defer or alter host painting of PTY output.
        w.write_all(b"\x1b[?1003l\x1b[?1004l\x1b[?2004l\x1b[?2026l")?;
        w.write_all(b"\x18")?; // CAN: parser to ground
        #[cfg(not(target_os = "macos"))]
        w.write_all(b"\x1b[!p")?; // DECSTR (skip on macOS Terminal.app)
        // Without DECSTR (macOS) or partial DECSTR: drop origin / LR margin so `999;999H` is viewport-relative.
        w.write_all(b"\x1b[?6l\x1b[?69l")?; // DECOM off, DECLRMM off
        w.write_all(b"\x1b[?7h\x1b(B\x1b)B\x1b[0m")?; // wrap, G0/G1 ASCII, SGR
        w.write_all(b"\x1b>")?; // numeric keypad
        w.write_all(b"\x1b[r\x1b[999;999H\r")?; // scroll region, cursor to bottom
        w.flush()?;
        let _ = unsafe { libc::tcdrain(1) };
        Ok(())
    }

    /// Set PTY window size to match the real terminal (stdout). MC: tty_resize in lib/tty/tty.c.
    /// Copy full winsize (including ws_xpixel/ws_ypixel) so ls/commands format correctly.
    fn resize_pty_to_terminal(master_fd: i32) {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        #[cfg(target_os = "macos")]
        let get_ok = unsafe {
            libc::ioctl(
                1,
                libc::TIOCGWINSZ as libc::c_ulong,
                &mut ws,
            )
        } == 0;
        #[cfg(not(target_os = "macos"))]
        let get_ok = unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0;
        if get_ok && (ws.ws_col > 0 || ws.ws_row > 0) {
            #[cfg(target_os = "macos")]
            let _ = unsafe {
                libc::ioctl(
                    master_fd,
                    libc::TIOCSWINSZ as libc::c_ulong,
                    &ws,
                )
            };
            #[cfg(not(target_os = "macos"))]
            let _ = unsafe { libc::ioctl(master_fd, libc::TIOCSWINSZ, &ws) };
        }
    }

    /// Spawn a subshell in a PTY. Cwd is the initial working directory.
    /// MC: init_subshell_child calls tty_resize(slave) before exec; we set size on master before fork.
    pub fn spawn(cwd: &str) -> io::Result<Self> {
        use nix::pty::openpty;

        let pty = openpty(None, None).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let slave_fd = pty.slave.as_raw_fd();
        let master_fd = pty.master.as_raw_fd();
        let (slave_st_rdev, ps_tty_arg) = pty_slave_rdev_and_ps_tty_arg(master_fd);

        // Set PTY size before fork so the shell never sees wrong dimensions (MC does this in child on slave).
        Self::resize_pty_to_terminal(master_fd);

        let pid = unsafe { libc::fork() };
        match pid {
            -1 => {
                drop(pty);
                return Err(io::Error::last_os_error());
            }
            0 => {
                drop(pty.master);
                std::mem::forget(pty.slave);
                unsafe {
                    libc::setsid();
                    #[cfg(target_os = "linux")]
                    libc::ioctl(slave_fd, libc::TIOCSCTTY, 0);
                    #[cfg(target_os = "macos")]
                    libc::ioctl(
                        slave_fd,
                        libc::TIOCSCTTY as libc::c_ulong,
                        0,
                    );
                    Self::set_pty_slave_cooked_mode(slave_fd);
                    libc::dup2(slave_fd, 0);
                    libc::dup2(slave_fd, 1);
                    libc::dup2(slave_fd, 2);
                    libc::close(slave_fd);
                }
                let _ = std::env::set_current_dir(cwd);
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let err = Command::new(&shell).arg("-i").current_dir(cwd).exec();
                eprintln!("exec {}: {}", shell, err);
                std::process::exit(1);
            }
            child_pid => {
                drop(pty.slave);
                std::mem::forget(pty.master);
                Ok(Self {
                    master_fd,
                    child_pid,
                    slave_st_rdev,
                    ps_tty_arg,
                    pending_command_done: RefCell::new(None),
                    pending_done_delay: RefCell::new(None),
                })
            }
        }
    }

    /// Full-screen relay: stdin → pty, pty → stdout, until **Ctrl+O** or **Ctrl+X** then **`o`** / **`O`** on stdin.
    /// Shell keeps running.
    /// Caller must leave alternate screen before calling and re-enter after return (see main.rs Suspend).
    /// If `prepared` is Some, caller already set relay raw and wrote reset sequence to stdout; we skip that and use saved termios for restore. If None, we set raw and write reset ourselves.
    /// If `show_prompt_first` is true (return from panels chord), send LF to the PTY and flush so the shell redraws its prompt.
    fn run_relay_until_ctrl_o(
        &self,
        show_prompt_first: bool,
        prepared: Option<PreparedRelay>,
        auto_exit: Option<AutoExitConfig>,
    ) -> io::Result<RelayExit> {
        use nix::errno::Errno;
        use nix::poll::{poll, PollFd, PollFlags};
        use nix::unistd;

        // Sync PTY size to real terminal so ls/commands format correctly (MC: tty_resize on SIGWINCH).
        // When caller already sent reset (prepared), give terminal time to process before reading winsize (avoids column misalignment on 2nd+ run).
        if prepared.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        Self::resize_pty_to_terminal(self.master_fd);

        let real_tty_saved = match prepared {
            Some(p) => p.0,
            None => {
                let saved = Self::set_real_tty_relay_raw();
                let _ = Self::write_all_fd(1, b"\x1b[?1003l\x1b[?1004l\x1b[?2004l\x1b[?2026l");
                let _ = Self::write_all_fd(1, b"\x18");
                #[cfg(not(target_os = "macos"))]
                let _ = Self::write_all_fd(1, b"\x1b[!p");
                let _ = Self::write_all_fd(1, b"\x1b[?6l\x1b[?69l");
                let _ = Self::write_all_fd(1, b"\x1b[?7h\x1b(B\x1b)B\x1b[0m");
                let _ = Self::write_all_fd(1, b"\x1b>");
                let _ = Self::write_all_fd(1, b"\x1b[r\x1b[999;999H\r");
                let _ = unsafe { libc::tcdrain(1) };
                saved
            }
        };

        if show_prompt_first {
            // Drain any stale PTY output from the previous session so the new prompt is not mixed with old data.
            let _ = Self::drain_pty_output(self.master_fd);
            // Force a new prompt every time: send newline so the shell prints a fresh prompt (works on 2nd+ attempt).
            #[cfg(not(target_os = "macos"))]
            let _ = Self::write_all_fd(self.master_fd, b"\r\n");
            #[cfg(target_os = "macos")]
            let _ = Self::write_all_fd(self.master_fd, b"\n");
            // MC: " \b" hack so prompt reappears.
            // let _ = Self::write_all_fd(self.master_fd, b" \x08");
            // Brief yield so the shell can write the new prompt before we start reading.
            std::thread::sleep(std::time::Duration::from_millis(20));
            let _ = Self::flush_pty_prompt_to_stdout(self.master_fd);
        }

        // MC uses PTY_BUFFER_SIZE (512); use 4K so long ls lines aren't fragmented and we don't lose info.
        let mut stdin_buf = [0u8; 256];
        let mut pty_buf = [0u8; 4096];
        let mut stdin_carry: Vec<u8> = Vec::with_capacity(32);
        let mut chord_withheld: Option<Vec<u8>> = None;
        let mut auto_exit = auto_exit;
        let max_auto_token_len = auto_exit
            .as_ref()
            .and_then(|c| match &c.completion {
                AutoExitCompletion::Stream(s) => Some(
                    s.completion_lf
                        .len()
                        .max(s.completion_crlf.len())
                        .max(s.helper_echo.len()),
                ),
                AutoExitCompletion::Fifo(_) => None,
            })
            .unwrap_or(0);
        let mut marker_pending: Vec<u8> = Vec::with_capacity(max_auto_token_len.max(1));

        let relay_result = (|| -> io::Result<RelayExit> {
            let mut fifo_scratch = [0u8; 64];
            loop {
                let fifo_slots: Vec<(RelayFifoRole, i32)> = {
                    let mut slots = Vec::new();
                    if let Some(ref c) = auto_exit {
                        if let AutoExitCompletion::Fifo(f) = &c.completion {
                            if let Some(ref cd) = f.command_done {
                                slots.push((RelayFifoRole::ActiveCommandDone, cd.as_raw_fd()));
                            }
                        }
                    }
                    if let Some(ref p) = *self.pending_command_done.borrow() {
                        slots.push((RelayFifoRole::PendingCommandDone, p.as_raw_fd()));
                    }
                    slots
                };
                let mut fds = vec![
                    PollFd::new(
                        unsafe { BorrowedFd::borrow_raw(0) },
                        PollFlags::POLLIN | PollFlags::POLLHUP,
                    ),
                    PollFd::new(
                        unsafe { BorrowedFd::borrow_raw(self.master_fd) },
                        PollFlags::POLLIN | PollFlags::POLLHUP,
                    ),
                ];
                for &(_, fd) in &fifo_slots {
                    fds.push(PollFd::new(
                        unsafe { BorrowedFd::borrow_raw(fd) },
                        PollFlags::POLLIN | PollFlags::POLLHUP,
                    ));
                }
                match poll(&mut fds, 100u16) {
                    Ok(_) => {}
                    Err(Errno::EINTR) => continue,
                    Err(_) => break,
                }

                if fds[0].revents().map_or(false, |r| {
                    r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP)
                }) {
                    match unistd::read(0, &mut stdin_buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let saw_ctrl_c =
                                auto_exit.is_some() && stdin_buf[..n].iter().any(|b| *b == 0x03);
                            if self.relay_stdin_chunk(
                                &mut stdin_carry,
                                &stdin_buf[..n],
                                &mut chord_withheld,
                            )? {
                                let _ = Self::drain_pty_output(self.master_fd);
                                return Ok(RelayExit::Manual);
                            }
                            if let Some(cfg) = auto_exit.as_mut() {
                                if let AutoExitCompletion::Fifo(f) = &mut cfg.completion {
                                    if let Some(ref mut se) = f.sudo_early_panels {
                                        if se.password_prompt_seen && se.password_submit_at.is_none()
                                        {
                                            if stdin_buf[..n]
                                                .iter()
                                                .any(|b| *b == b'\n' || *b == b'\r')
                                            {
                                                se.password_submit_at =
                                                    Some(std::time::Instant::now());
                                            }
                                        }
                                    }
                                }
                            }
                            if saw_ctrl_c {
                                if let Some(cfg) = auto_exit.as_ref() {
                                    // Interrupted: delay is shown as a TUI countdown after relay ends.
                                    let _ = Self::drain_pty_output(self.master_fd);
                                    return Ok(RelayExit::AutoReopenDelay(cfg.delay));
                                }
                            }
                        }
                        Err(Errno::EINTR) => {}
                        Err(_) => break,
                    }
                }

                for (i, &(role, fd)) in fifo_slots.iter().enumerate() {
                    let idx = 2 + i;
                    if fds
                        .get(idx)
                        .and_then(|p| p.revents())
                        .map_or(false, |r| {
                            r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP)
                        })
                    {
                        if Self::consume_fifo_signal(fd, &mut fifo_scratch)? {
                            let _ = Self::drain_pty_output(self.master_fd);
                            let delay = match role {
                                RelayFifoRole::ActiveCommandDone => {
                                    auto_exit.as_ref().expect("fifo auto_exit").delay
                                }
                                RelayFifoRole::PendingCommandDone => self
                                    .pending_done_delay
                                    .borrow_mut()
                                    .take()
                                    .unwrap_or_else(|| std::time::Duration::from_secs(1)),
                            };
                            if role == RelayFifoRole::PendingCommandDone {
                                self.pending_command_done.borrow_mut().take();
                            }
                            return Ok(RelayExit::AutoReopenDelay(delay));
                        }
                    }
                }

                if fds[1].revents().map_or(false, |r| {
                    r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP)
                }) {
                    match Self::read_pty_nonblock(self.master_fd, &mut pty_buf)? {
                        Some(0) => break,
                        Some(n) => {
                            if let Some(cfg) = auto_exit.as_mut() {
                                match &mut cfg.completion {
                                    AutoExitCompletion::Fifo(f) => {
                                        Self::write_all_fd(1, &pty_buf[..n])?;
                                        if let Some(ref mut se) = f.sudo_early_panels {
                                            const SCAN_CAP: usize = 65_536;
                                            se.scan_tail.extend_from_slice(&pty_buf[..n]);
                                            if se.scan_tail.len() > SCAN_CAP {
                                                let d = se.scan_tail.len() - SCAN_CAP;
                                                se.scan_tail.drain(..d);
                                            }
                                            if !se.password_prompt_seen
                                                && Self::pty_tail_has_sudo_password_prompt(
                                                    &se.scan_tail,
                                                )
                                            {
                                                se.password_prompt_seen = true;
                                            }
                                        }
                                    }
                                    AutoExitCompletion::Stream(stream) => {
                                        if Self::write_pty_chunk_without_marker(
                                            &mut marker_pending,
                                            &pty_buf[..n],
                                            &stream.completion_lf,
                                            &stream.completion_crlf,
                                            &stream.helper_echo,
                                        )? {
                                            return Ok(RelayExit::AutoReopenDelay(cfg.delay));
                                        }
                                    }
                                }
                            } else {
                                Self::write_all_fd(1, &pty_buf[..n])?;
                            }
                        }
                        None => {} // EAGAIN, no data this time
                    }
                }

                // After PTY bytes (if any): timer-driven early reopen — idle `#` prompt may send no more PTY data.
                if let Some(cfg) = auto_exit.as_mut() {
                    if let AutoExitCompletion::Fifo(f) = &mut cfg.completion {
                        if let Some(ref mut se) = f.sudo_early_panels {
                            let delay = cfg.delay;
                            if let Some(exit) =
                                self.sudo_early_panels_try_exit(se, &mut f.command_done, delay)?
                            {
                                return Ok(exit);
                            }
                        }
                    }
                }
            }
            if let Some(cfg) = auto_exit.as_ref() {
                if let AutoExitCompletion::Stream(stream) = &cfg.completion {
                    if !marker_pending.is_empty() {
                        Self::flush_pty_pending_without_marker(
                            &mut marker_pending,
                            &stream.helper_echo,
                        )?;
                    }
                }
            }
            Ok(RelayExit::Manual)
        })();
        Self::restore_real_tty(real_tty_saved);
        relay_result
    }

    /// Returns [`RelayExit::AutoReopenDelay`] when an interactive `sudo` session looks ready (root
    /// stable across a few timer polls). Must run every relay loop turn, including `poll` timeouts.
    fn sudo_early_panels_try_exit(
        &self,
        se: &mut SudoEarlyPanelReopen,
        command_done: &mut Option<OwnedFifo>,
        reopen_delay: std::time::Duration,
    ) -> io::Result<Option<RelayExit>> {
        use std::time::{Duration, Instant};
        const AFTER_PASSWORD_SUBMIT: Duration = Duration::from_millis(550);
        const NOPASS_BEFORE_POLL: Duration = Duration::from_millis(900);
        const POLL_INTERVAL: Duration = Duration::from_millis(200);
        const ROOT_STREAK: u32 = 2;

        let may_poll = if se.password_prompt_seen {
            se.password_submit_at
                .is_some_and(|t| t.elapsed() >= AFTER_PASSWORD_SUBMIT)
        } else {
            se.relay_started_at.elapsed() >= NOPASS_BEFORE_POLL
        };
        if !may_poll {
            return Ok(None);
        }
        let now = Instant::now();
        if !se
            .last_poll_at
            .map(|t| now.saturating_duration_since(t) >= POLL_INTERVAL)
            .unwrap_or(true)
        {
            return Ok(None);
        }
        se.last_poll_at = Some(now);
        if self.pty_foreground_has_root_euid() {
            se.root_streak = se.root_streak.saturating_add(1);
        } else {
            se.root_streak = 0;
        }
        if se.root_streak >= ROOT_STREAK {
            let _ = Self::drain_pty_output(self.master_fd);
            if let Some(fifo) = command_done.take() {
                *self.pending_done_delay.borrow_mut() = Some(reopen_delay);
                *self.pending_command_done.borrow_mut() = Some(fifo);
            }
            return Ok(Some(RelayExit::AutoReopenDelay(reopen_delay)));
        }
        Ok(None)
    }

    /// Drain a single completion byte from a FIFO (non-blocking read loop).
    fn consume_fifo_signal(
        fd: i32,
        scratch: &mut [u8],
    ) -> io::Result<bool> {
        use nix::errno::Errno;
        use nix::unistd;
        let mut got_byte = false;
        loop {
            match unistd::read(fd, scratch) {
                Ok(0) => break,
                Ok(n) => {
                    if n > 0 {
                        got_byte = true;
                    }
                }
                Err(Errno::EAGAIN) => break,
                Err(Errno::EINTR) => continue,
                Err(e) => {
                    return Err(io::Error::new(io::ErrorKind::Other, e));
                }
            }
        }
        Ok(got_byte)
    }

    fn ascii_lower_byte(b: u8) -> u8 {
        if b.is_ascii_uppercase() {
            b.to_ascii_lowercase()
        } else {
            b
        }
    }

    fn bytes_contains_ascii_ci(haystack: &[u8], needle_lower: &[u8]) -> bool {
        if needle_lower.is_empty() {
            return true;
        }
        if haystack.len() < needle_lower.len() {
            return false;
        }
        'outer: for i in 0..=haystack.len() - needle_lower.len() {
            for j in 0..needle_lower.len() {
                if Self::ascii_lower_byte(haystack[i + j]) != needle_lower[j] {
                    continue 'outer;
                }
            }
            return true;
        }
        false
    }

    /// Stable `sudo` password-phase strings on the PTY (not `$PS1`).
    fn pty_tail_has_sudo_password_prompt(tail: &[u8]) -> bool {
        Self::bytes_contains_ascii_ci(tail, b"[sudo] password")
            || Self::bytes_contains_ascii_ci(tail, b"password for ")
            || Self::bytes_contains_ascii_ci(tail, b"password:")
    }

    /// Escape path for shell (single-quote style so spaces/special chars are safe).
    fn shell_escape_path(path: &str) -> String {
        format!("'{}'", path.replace('\'', "'\"'\"'"))
    }

    /// Current working directory of the subshell process (after user may have run `cd`).
    /// Used when returning from relay to sync the active panel to the shell's cwd.
    /// Linux: readlink /proc/pid/cwd. macOS: libproc proc_pidinfo (PROC_PIDVNODEPATHINFO). Other Unix: None.
    pub fn get_cwd(&self) -> Option<PathBuf> {
        #[cfg(target_os = "linux")]
        {
            let path = format!("/proc/{}/cwd", self.child_pid);
            std::fs::read_link(&path).ok()
        }
        #[cfg(target_os = "macos")]
        {
            get_cwd_macos(self.child_pid as u32)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = self;
            None
        }
    }

    /// True if the PTY’s foreground process group has effective UID 0 (e.g. interactive root after `sudo -s`).
    /// Oxide’s own [`libc::geteuid`] may still be non-zero; use this for “danger” chrome after subshell relay.
    ///
    /// Uses a `ps -t` fallback when the precise foreground probe is inconclusive on some terminals.
    pub fn pty_foreground_has_root_euid(&self) -> bool {
        if pty_foreground_euid_is_root(
            self.master_fd,
            self.child_pid,
            self.slave_st_rdev,
            self.ps_tty_arg.as_str(),
        ) {
            return true;
        }
        ps_tty_has_any_euid_zero(&self.ps_tty_arg)
    }

    /// Change shell cwd to match the active panel then relay until Ctrl+O or Ctrl+X then O (for Suspend so ls matches panel).
    /// Only sends `cd 'cwd'` when the shell is not already in that directory, to avoid redundant commands in history.
    pub fn run_cd_then_relay(
        &self,
        cwd: &str,
        prepared: Option<PreparedRelay>,
    ) -> io::Result<()> {
        let panel_canonical = Path::new(cwd).canonicalize().ok();
        let shell_canonical = self.get_cwd().and_then(|p| p.canonicalize().ok());
        let need_cd = match (
            panel_canonical.as_ref(),
            shell_canonical.as_ref(),
        ) {
            (Some(a), Some(b)) => a != b,
            _ => true, // if we can't resolve either, send cd to be safe
        };
        if need_cd {
            let cd_escaped = Self::shell_escape_path(cwd);
            let mut buf = Vec::with_capacity(8 + cd_escaped.len() + 2);
            buf.extend_from_slice(b"cd ");
            buf.extend_from_slice(cd_escaped.as_bytes());
            buf.push(b'\n');
            Self::write_all_fd(self.master_fd, &buf)?;
            let _ = Self::drain_pty_output(self.master_fd);
        }
        // Caller already moved the real cursor (relay reset). If we did not send `cd`, the PTY
        // has no fresh echo to realign the terminal — force a prompt so cursor matches (2nd+ return from shell).
        let _ = self.run_relay_until_ctrl_o(!need_cd, prepared, None)?;
        Ok(())
    }

    /// Run a command in the subshell then relay until Ctrl+O or Ctrl+X then O (MC: invoke_subshell with command).
    /// Only sends `cd 'cwd'` when the shell is not already in that directory.
    pub fn run_command_then_relay(
        &self,
        cwd: &str,
        cmd: &str,
        prepared: Option<PreparedRelay>,
        auto_exit_after_idle: Option<std::time::Duration>,
    ) -> io::Result<RelayExit> {
        let panel_canonical = Path::new(cwd).canonicalize().ok();
        let shell_canonical = self.get_cwd().and_then(|p| p.canonicalize().ok());
        let need_cd = match (
            panel_canonical.as_ref(),
            shell_canonical.as_ref(),
        ) {
            (Some(a), Some(b)) => a != b,
            _ => true,
        };
        let mut buf = Vec::new();
        if need_cd {
            let cd_escaped = Self::shell_escape_path(cwd);
            buf.extend_from_slice(b"cd ");
            buf.extend_from_slice(cd_escaped.as_bytes());
            buf.push(b'\n');
        }
        let mut auto_exit_cfg: Option<AutoExitConfig> = None;
        if let Some(delay) = auto_exit_after_idle {
            let cmd_escaped = Self::shell_escape_path(cmd);
            match OwnedFifo::open_for_subshell(self.child_pid) {
                Ok(command_done) => {
                    let sudo_early_panels = if cmd_allocates_interactive_sudo_shell(cmd) {
                        Some(SudoEarlyPanelReopen::new())
                    } else {
                        None
                    };
                    let fifo_q = Self::shell_escape_path(&command_done.path.to_string_lossy());
                    // One line: `eval` runs the user command; `printf` runs only after it finishes.
                    // Side channel avoids scanning the PTY stream (sudo prompts, echo, short reads).
                    buf.extend_from_slice(b"eval ");
                    buf.extend_from_slice(cmd_escaped.as_bytes());
                    buf.extend_from_slice(b"; printf '\\n' > ");
                    buf.extend_from_slice(fifo_q.as_bytes());
                    buf.push(b'\n');
                    auto_exit_cfg = Some(AutoExitConfig {
                        delay,
                        completion: AutoExitCompletion::Fifo(FifoCompletion {
                            command_done: Some(command_done),
                            sudo_early_panels,
                        }),
                    });
                }
                Err(e) => {
                    eprintln!(
                        "oxide: mkfifo/open failed ({}); using PTY marker for auto-reopen",
                        e
                    );
                    let nonce = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64
                        ^ (self.child_pid as u64);
                    let marker = format!(
                        "OXD_{:08x}_{}",
                        (nonce & 0xffff_ffff) as u32,
                        self.child_pid
                    );
                    let mut completion_lf = marker.clone().into_bytes();
                    completion_lf.push(b'\n');
                    let mut completion_crlf = marker.clone().into_bytes();
                    completion_crlf.extend_from_slice(b"\r\n");
                    let helper = format!("printf '%s\\n' '{marker}'");
                    buf.extend_from_slice(b"eval ");
                    buf.extend_from_slice(cmd_escaped.as_bytes());
                    buf.extend_from_slice(b"; ");
                    buf.extend_from_slice(helper.as_bytes());
                    buf.push(b'\n');
                    auto_exit_cfg = Some(AutoExitConfig {
                        delay,
                        completion: AutoExitCompletion::Stream(StreamMonitoredCompletion {
                            completion_lf,
                            completion_crlf,
                            helper_echo: helper.into_bytes(),
                        }),
                    });
                }
            }
        } else {
            buf.extend_from_slice(cmd.as_bytes());
            buf.push(b'\n');
        }
        Self::write_all_fd(self.master_fd, &buf)?;
        self.run_relay_until_ctrl_o(false, prepared, auto_exit_cfg)
    }
}

#[cfg(target_os = "macos")]
fn get_cwd_macos(pid: u32) -> Option<PathBuf> {
    const PROC_PIDVNODEPATHINFO: libc::c_int = 9;
    let mut buf = [0u8; 4096];
    let bytes_read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            PROC_PIDVNODEPATHINFO,
            0,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as libc::c_int,
        )
    };
    if bytes_read <= 0 {
        return None;
    }
    // proc_vnodepathinfo contains pvi_rdir (root) and pvi_cdir (cwd). Layout varies by OS version.
    // Scan the buffer for null-terminated absolute paths; prefer the longest that exists and is a directory (cwd).
    let mut best: Option<PathBuf> = None;
    let mut i = 0;
    let len = bytes_read as usize;
    while i < len {
        if buf[i] != b'/' {
            i += 1;
            continue;
        }
        let start = i;
        while i < len && buf[i] != 0 {
            i += 1;
        }
        if i > start {
            if let Ok(s) = std::str::from_utf8(&buf[start..i]) {
                let s = s.trim();
                if !s.is_empty() {
                    let candidate_path = PathBuf::from(s);
                    if candidate_path.is_dir()
                        && best.as_ref().map_or(true, |b| {
                            candidate_path.as_os_str().len() > b.as_os_str().len()
                        })
                    {
                        best = Some(candidate_path);
                    }
                }
            }
        }
        i += 1;
    }
    best
}

/// Slave device id and a short tty name suitable for `ps -t` (no `/dev/` prefix).
#[cfg(unix)]
fn pty_slave_rdev_and_ps_tty_arg(master_fd: i32) -> (u64, String) {
    use std::ffi::CStr;
    use std::os::unix::fs::MetadataExt;

    let path_str: Option<String> = {
        #[cfg(target_os = "linux")]
        {
            let mut buf = [0u8; 512];
            let r = unsafe { libc::ptsname_r(master_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if r != 0 {
                None
            } else {
                let len = buf.iter().position(|&b| b == 0).unwrap_or(0);
                std::str::from_utf8(&buf[..len])
                    .ok()
                    .map(str::to_string)
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let name_ptr = unsafe { libc::ptsname(master_fd) };
            if name_ptr.is_null() {
                None
            } else {
                unsafe { CStr::from_ptr(name_ptr) }
                    .to_str()
                    .ok()
                    .map(str::to_string)
            }
        }
    };
    let Some(ref path) = path_str else {
        return (0, String::new());
    };
    let rdev = std::fs::metadata(path)
        .map(|m| m.rdev())
        .unwrap_or(0);
    let tty_arg = path
        .strip_prefix("/dev/")
        .unwrap_or(path.as_str())
        .to_string();
    (rdev, tty_arg)
}

/// Any process attached to this tty reports effective uid 0 (used only as a fallback for [`pty_foreground_has_root_euid`]).
#[cfg(unix)]
fn ps_tty_has_any_euid_zero(tty: &str) -> bool {
    if tty.is_empty() {
        return false;
    }
    let try_ps = |args: &[&str]| -> bool {
        let mut cmd = std::process::Command::new("ps");
        for a in args {
            cmd.arg(a);
        }
        let Ok(out) = cmd.output() else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let t = line.trim();
                if t.is_empty() || t.eq_ignore_ascii_case("uid") {
                    return None;
                }
                t.parse::<u32>().ok()
            })
            .any(|u| u == 0)
    };
    #[cfg(target_os = "macos")]
    {
        try_ps(&["-t", tty, "-o", "uid="])
    }
    #[cfg(target_os = "linux")]
    {
        try_ps(&["--no-headers", "-t", tty, "-o", "uid="])
            || try_ps(&["-t", tty, "-o", "uid="])
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = tty;
        false
    }
}

/// Whether the PTY foreground process group has any member with EUID 0; falls back to the session leader’s EUID.
#[cfg(unix)]
fn pty_foreground_euid_is_root(
    master_fd: i32,
    child_pid: i32,
    #[allow(unused_variables)] slave_st_rdev: u64,
    #[allow(unused_variables)] ps_tty_arg: &str,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = ps_tty_arg;
        return pty_foreground_euid_is_root_macos(master_fd, child_pid, slave_st_rdev);
    }
    #[cfg(target_os = "linux")]
    {
        let mut pgrp = unsafe { libc::tcgetpgrp(master_fd) };
        if pgrp <= 0 {
            pgrp = linux_foreground_pgrp_from_proc(child_pid).unwrap_or(-1);
        }
        if pgrp > 0 && foreground_pgrp_has_euid_zero(pgrp) {
            return true;
        }
        linux_process_euid(child_pid) == Some(0)
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    {
        let _ = (master_fd, child_pid, slave_st_rdev, ps_tty_arg);
        false
    }
}

#[cfg(all(unix, target_os = "linux"))]
fn linux_foreground_pgrp_from_proc(pid: i32) -> Option<i32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = s.rsplit_once(") ")?;
    let mut fields = rest.1.split_whitespace();
    let _state = fields.next()?;
    let _ppid = fields.next()?;
    let _pgrp = fields.next()?;
    let _session = fields.next()?;
    let _tty_nr = fields.next()?;
    let tpgid: i32 = fields.next()?.parse().ok()?;
    if tpgid <= 0 {
        None
    } else {
        Some(tpgid)
    }
}

#[cfg(all(unix, target_os = "linux"))]
fn foreground_pgrp_has_euid_zero(pgrp: i32) -> bool {
    use nix::unistd::{getpgid, Pid};
    let pg = Pid::from_raw(pgrp);
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<i32>() else {
            continue;
        };
        if pid <= 0 {
            continue;
        }
        let p = Pid::from_raw(pid);
        if getpgid(Some(p)) != Ok(pg) {
            continue;
        }
        if linux_process_euid(pid) == Some(0) {
            return true;
        }
    }
    false
}

#[cfg(all(unix, target_os = "linux"))]
fn linux_process_euid(pid: i32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            return parts.get(1).and_then(|s| s.parse().ok());
        }
    }
    None
}

#[cfg(all(unix, target_os = "macos"))]
fn macos_proc_taskall(pid: i32) -> Option<libc::proc_taskallinfo> {
    let mut info: libc::proc_taskallinfo = unsafe { std::mem::zeroed() };
    let sz = std::mem::size_of::<libc::proc_taskallinfo>() as libc::c_int;
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKALLINFO,
            0,
            std::ptr::addr_of_mut!(info).cast::<libc::c_void>(),
            sz,
        )
    };
    if (n as usize) < std::mem::size_of::<libc::proc_taskallinfo>() {
        return None;
    }
    Some(info)
}

#[cfg(all(unix, target_os = "macos"))]
fn macos_pgrp_pids(pgrp: i32) -> Vec<i32> {
    if pgrp <= 0 {
        return Vec::new();
    }
    const MAX: usize = 512;
    let mut buf = [0i32; MAX];
    let n = unsafe {
        libc::proc_listpgrppids(
            pgrp,
            buf.as_mut_ptr() as *mut libc::c_void,
            (MAX * std::mem::size_of::<libc::pid_t>()) as libc::c_int,
        )
    };
    if n <= 0 {
        return Vec::new();
    }
    let count = (n as usize) / std::mem::size_of::<libc::pid_t>();
    buf[..count.min(MAX)]
        .iter()
        .copied()
        .filter(|&p| p > 0)
        .collect()
}

#[cfg(all(unix, target_os = "macos"))]
fn macos_pgrp_has_uid0(pgrp: i32) -> bool {
    macos_pgrp_pids(pgrp)
        .into_iter()
        .any(|pid| macos_proc_taskall(pid).is_some_and(|i| i.pbsd.pbi_uid == 0))
}

#[cfg(all(unix, target_os = "macos"))]
fn macos_slave_rdev_has_uid0(slave_st_rdev: u64) -> bool {
    if slave_st_rdev == 0 {
        return false;
    }
    const MAX_PIDS: usize = 8192;
    let mut buf = [0i32; MAX_PIDS];
    let size_bytes = (MAX_PIDS * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    let n_bytes = unsafe {
        libc::proc_listallpids(
            buf.as_mut_ptr() as *mut libc::c_void,
            size_bytes,
        )
    };
    if n_bytes <= 0 {
        return false;
    }
    let n_pids = (n_bytes as usize / std::mem::size_of::<libc::pid_t>()).min(MAX_PIDS);
    for &pid in buf[..n_pids].iter().filter(|&&p| p > 0) {
        if let Some(info) = macos_proc_taskall(pid) {
            let edev = info.pbsd.e_tdev as u64;
            if edev == 0 {
                continue;
            }
            let r = slave_st_rdev;
            if (edev == r || edev == (r & 0xffff_ffff)) && info.pbsd.pbi_uid == 0 {
                return true;
            }
        }
    }
    false
}

/// macOS: `tcgetpgrp` on the PTY master can fail; use `e_tpgid` from the session shell, `proc_listpgrppids`,
/// and `proc_taskallinfo.pbi_uid` (effective). Fall back to matching `e_tdev` to the slave `st_rdev`.
#[cfg(all(unix, target_os = "macos"))]
fn pty_foreground_euid_is_root_macos(
    master_fd: i32,
    child_pid: i32,
    slave_st_rdev: u64,
) -> bool {
    let mut pgrp = unsafe { libc::tcgetpgrp(master_fd) };
    if pgrp <= 0 {
        pgrp = macos_proc_taskall(child_pid)
            .map(|i| i.pbsd.e_tpgid as i32)
            .unwrap_or(-1);
    }
    if pgrp > 0 && macos_pgrp_has_uid0(pgrp) {
        return true;
    }
    if macos_slave_rdev_has_uid0(slave_st_rdev) {
        return true;
    }
    macos_proc_taskall(child_pid).is_some_and(|i| i.pbsd.pbi_uid == 0)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn foreground_pgrp_has_euid_zero(_pgrp: i32) -> bool {
    false
}

/// Kill the entire subshell session so no process (e.g. nohup) outlives the app.
/// On Linux: enumerate all processes in the session via /proc + getsid, send SIGTERM, then SIGKILL.
/// On macOS: enumerate via libc::proc_listallpids + getsid, then SIGTERM / SIGKILL.
/// On other Unix: send SIGTERM then SIGKILL to the shell's process group (best effort).
#[cfg(unix)]
fn kill_subshell_session(session_leader_pid: i32) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let session_leader = Pid::from_raw(session_leader_pid);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let self_pid = nix::unistd::getpid();

    #[cfg(target_os = "linux")]
    fn pids_in_session_linux(
        session_leader: Pid,
        exclude_pid: Pid,
    ) -> Vec<Pid> {
        use nix::unistd::getsid;
        use std::fs;

        let mut pids = Vec::new();
        let Ok(entries) = fs::read_dir("/proc") else {
            return pids;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Ok(pid) = name.to_string_lossy().parse::<i32>() else {
                continue;
            };
            if pid <= 0 || Pid::from_raw(pid) == exclude_pid {
                continue;
            }
            let pid = Pid::from_raw(pid);
            if let Ok(sid) = getsid(Some(pid)) {
                if sid == session_leader {
                    pids.push(pid);
                }
            }
        }
        pids
    }

    #[cfg(target_os = "macos")]
    fn pids_in_session_macos(
        session_leader: Pid,
        exclude_pid: Pid,
    ) -> Vec<Pid> {
        use nix::unistd::getsid;

        const MAX_PIDS: usize = 8192;
        let mut buf = [0i32; MAX_PIDS];
        let size_bytes = (MAX_PIDS * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        let n_bytes = unsafe {
            libc::proc_listallpids(
                buf.as_mut_ptr() as *mut libc::c_void,
                size_bytes,
            )
        };
        if n_bytes <= 0 {
            return Vec::new();
        }
        let n_pids = (n_bytes as usize) / std::mem::size_of::<libc::pid_t>();
        let n_pids = n_pids.min(MAX_PIDS);

        let mut pids = Vec::new();
        for i in 0..n_pids {
            let pid = buf[i];
            if pid <= 0 || Pid::from_raw(pid) == exclude_pid {
                continue;
            }
            let pid = Pid::from_raw(pid);
            if let Ok(sid) = getsid(Some(pid)) {
                if sid == session_leader {
                    pids.push(pid);
                }
            }
        }
        pids
    }

    #[cfg(target_os = "linux")]
    {
        let pids = pids_in_session_linux(session_leader, self_pid);
        for &pid in &pids {
            let _ = kill(pid, Signal::SIGTERM);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pids2 = pids_in_session_linux(session_leader, self_pid);
        for &pid in &pids2 {
            let _ = kill(pid, Signal::SIGKILL);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let pids = pids_in_session_macos(session_leader, self_pid);
        for &pid in &pids {
            let _ = kill(pid, Signal::SIGTERM);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pids2 = pids_in_session_macos(session_leader, self_pid);
        for &pid in &pids2 {
            let _ = kill(pid, Signal::SIGKILL);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // Process-group kill: shell's PID is the process group leader (setsid in child).
        let _ = kill(
            Pid::from_raw(-session_leader_pid),
            Signal::SIGTERM,
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = kill(
            Pid::from_raw(-session_leader_pid),
            Signal::SIGKILL,
        );
    }

    let _ = kill(session_leader, Signal::SIGKILL);
}

#[cfg(unix)]
impl Drop for Subshell {
    fn drop(&mut self) {
        kill_subshell_session(self.child_pid);
        let _ = nix::unistd::close(self.master_fd);
        let _ = nix::sys::wait::waitpid(
            nix::unistd::Pid::from_raw(self.child_pid),
            None,
        );
    }
}

#[cfg(not(unix))]
pub struct Subshell;

#[cfg(not(unix))]
impl Subshell {
    pub fn prepare_for_relay() -> PreparedRelay {
        PreparedRelay
    }
    pub fn write_relay_reset_sequence<W: Write>(_w: &mut W) -> io::Result<()> {
        Ok(())
    }
    pub fn spawn(_cwd: &str) -> io::Result<Self> {
        Ok(Self)
    }
    pub fn write(
        &self,
        _data: &[u8],
    ) -> io::Result<usize> {
        Ok(0)
    }
    pub fn run_relay_until_ctrl_o(
        &self,
        _show_prompt_first: bool,
        _prepared: Option<PreparedRelay>,
        _auto_exit_after_idle: Option<std::time::Duration>,
    ) -> io::Result<RelayExit> {
        Ok(RelayExit::Manual)
    }

    pub fn run_cd_then_relay(
        &self,
        _cwd: &str,
        _prepared: Option<PreparedRelay>,
    ) -> io::Result<()> {
        Ok(())
    }

    pub fn run_command_then_relay(
        &self,
        _cwd: &str,
        _cmd: &str,
        _prepared: Option<PreparedRelay>,
        _auto_exit_after_idle: Option<std::time::Duration>,
    ) -> io::Result<RelayExit> {
        Ok(RelayExit::Manual)
    }

    pub fn pty_foreground_has_root_euid(&self) -> bool {
        false
    }
}
