//! PTY subshell (toggle shell); command line runs in the original terminal.
//!
//! Return to panels: **Ctrl+O** (`0x0F`, MC-style) or **Ctrl+X** then plain **`o`** / **`O`** (see REFERENCE.md).
//! Leave alternate screen, relay stdin↔PTY until one of those is read from stdin; then caller
//! re-enters alternate and redraws. Raw mode stays on; the subshell runs in a PTY with its own termios.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
    fn open_in_temp() -> io::Result<Self> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("oxide_cmd_{}.fifo", nonce));
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

#[cfg(unix)]
enum AutoExitCompletion {
    Fifo(OwnedFifo),
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
        let fifo_fd = auto_exit.as_ref().and_then(|c| match &c.completion {
            AutoExitCompletion::Fifo(f) => Some(f.as_raw_fd()),
            AutoExitCompletion::Stream(_) => None,
        });
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
                if let Some(fd) = fifo_fd {
                    fds.push(PollFd::new(
                        unsafe { BorrowedFd::borrow_raw(fd) },
                        PollFlags::POLLIN | PollFlags::POLLHUP,
                    ));
                }
                match poll(&mut fds, 100u16) {
                    Ok(0) => continue,
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

                if fifo_fd.is_some()
                    && fds
                        .get(2)
                        .and_then(|p| p.revents())
                        .map_or(false, |r| {
                            r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP)
                        })
                {
                    let fd = fifo_fd.expect("fds[2] only when fifo_fd is set");
                    // With no writer yet, some OSes report readable but read() returns 0 — not completion.
                    let mut got_byte = false;
                    loop {
                        match unistd::read(fd, &mut fifo_scratch) {
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
                    if got_byte {
                        let _ = Self::drain_pty_output(self.master_fd);
                        let delay = auto_exit.as_ref().expect("fifo auto_exit").delay;
                        return Ok(RelayExit::AutoReopenDelay(delay));
                    }
                }

                if fds[1].revents().map_or(false, |r| {
                    r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP)
                }) {
                    match Self::read_pty_nonblock(self.master_fd, &mut pty_buf)? {
                        Some(0) => break,
                        Some(n) => {
                            if let Some(cfg) = auto_exit.as_ref() {
                                match &cfg.completion {
                                    AutoExitCompletion::Fifo(_) => {
                                        Self::write_all_fd(1, &pty_buf[..n])?;
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
            match OwnedFifo::open_in_temp() {
                Ok(fifo) => {
                    let fifo_q = Self::shell_escape_path(&fifo.path.to_string_lossy());
                    // One line: `eval` runs the user command; `printf` runs only after it finishes.
                    // Side channel avoids scanning the PTY stream (sudo prompts, echo, short reads).
                    buf.extend_from_slice(b"eval ");
                    buf.extend_from_slice(cmd_escaped.as_bytes());
                    buf.extend_from_slice(b"; printf '\\n' > ");
                    buf.extend_from_slice(fifo_q.as_bytes());
                    buf.push(b'\n');
                    auto_exit_cfg = Some(AutoExitConfig {
                        delay,
                        completion: AutoExitCompletion::Fifo(fifo),
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
#[link(name = "proc", kind = "dylib")]
extern "C" {
    fn proc_pidinfo(
        pid: libc::c_int,
        flavor: libc::c_int,
        arg: u64,
        buffer: *mut libc::c_void,
        buffersize: libc::c_int,
    ) -> libc::c_int;
}

#[cfg(target_os = "macos")]
fn get_cwd_macos(pid: u32) -> Option<PathBuf> {
    const PROC_PIDVNODEPATHINFO: libc::c_int = 9;
    let mut buf = [0u8; 4096];
    let bytes_read = unsafe {
        proc_pidinfo(
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
}
