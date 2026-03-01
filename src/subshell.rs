//! PTY subshell for Ctrl+O (toggle shell); command line runs in the original terminal.
//!
//! Ctrl+O flow (MC-style, see REFERENCE.md): leave alternate screen, relay stdin↔PTY until
//! Ctrl+O (0x0F) is read from stdin; then caller re-enters alternate and redraws. Raw mode
//! stays on so we can detect Ctrl+O; the subshell runs in a PTY with its own termios.

use std::io::{self, Write};

#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
const CTRL_O: u8 = 0x0F;
#[cfg(unix)]
const CTRL_O_KITTY: &[u8] = b"\x1b[111;5u";
#[cfg(unix)]
const CTRL_O_MODIFY_OTHER_KEYS: &[u8] = b"\x1b[27;5;111~";

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

/// Persistent subshell: command line runs here, Ctrl+O toggles full-screen relay.
#[cfg(unix)]
pub struct Subshell {
    master_fd: i32,
    child_pid: i32,
}

#[cfg(unix)]
impl Subshell {
    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
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

    /// Feed stdin bytes into relay and intercept Ctrl+O in both raw and escaped encodings.
    /// Returns true when Ctrl+O is detected (caller should exit relay).
    fn relay_stdin_chunk(&self, carry: &mut Vec<u8>, chunk: &[u8]) -> io::Result<bool> {
        carry.extend_from_slice(chunk);
        loop {
            let plain_pos = carry.iter().position(|&b| b == CTRL_O).map(|p| (p, 1usize));
            let kitty_pos =
                Self::find_subsequence(carry, CTRL_O_KITTY).map(|p| (p, CTRL_O_KITTY.len()));
            let mok_pos = Self::find_subsequence(carry, CTRL_O_MODIFY_OTHER_KEYS)
                .map(|p| (p, CTRL_O_MODIFY_OTHER_KEYS.len()));

            let mut found: Option<(usize, usize)> = None;
            for cand in [plain_pos, kitty_pos, mok_pos].into_iter().flatten() {
                found = match found {
                    None => Some(cand),
                    Some(curr) => Some(if cand.0 < curr.0 { cand } else { curr }),
                };
            }

            if let Some((pos, len)) = found {
                if pos > 0 {
                    Self::write_all_fd(self.master_fd, &carry[..pos])?;
                }
                carry.drain(..pos + len);
                return Ok(true);
            }

            let keep = Self::trailing_ctrl_o_prefix_len(carry);
            let forward_len = carry.len().saturating_sub(keep);
            if forward_len > 0 {
                Self::write_all_fd(self.master_fd, &carry[..forward_len])?;
                carry.drain(..forward_len);
            }
            return Ok(false);
        }
    }

    /// Read from PTY in non-blocking mode (MC: read_nonblock). Avoids lockup when slave tcflush() revokes data between poll and read.
    fn read_pty_nonblock(fd: i32, buf: &mut [u8]) -> io::Result<Option<usize>> {
        use nix::errno::Errno;
        let old_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if old_flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, old_flags | libc::O_NONBLOCK) } != 0 {
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

    fn write_all_fd(fd: i32, mut data: &[u8]) -> io::Result<()> {
        use nix::errno::Errno;
        while !data.is_empty() {
            match nix::unistd::write(unsafe { BorrowedFd::borrow_raw(fd) }, data) {
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

    /// After Ctrl+O, flush any PTY bytes already produced so we don't cut escape/UTF-8 sequences
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
    /// Disable VDISCARD so Ctrl+O never toggles kernel output discard on the slave.
    fn set_pty_slave_cooked_mode(slave_fd: i32) {
        let mut tio: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(slave_fd, &mut tio) } != 0 {
            return;
        }
        // Cooked input: canonical mode, echo, CR→NL
        tio.c_lflag |= libc::ICANON | libc::ECHO | libc::IEXTEN;
        tio.c_lflag &= !libc::ISIG; // let shell handle signals
        tio.c_iflag |= libc::ICRNL;
        tio.c_iflag &= !libc::IXON; // pass ^S/^Q to shell (MC does this in raw_mode)
        // Cooked output: postprocess, \n → \r\n
        tio.c_oflag |= libc::OPOST | libc::ONLCR;
        tio.c_cc[libc::VMIN] = 1;
        tio.c_cc[libc::VTIME] = 0;
        // Disable VDISCARD so Ctrl+O doesn't toggle output discard (macOS/BSD)
        if libc::VDISCARD < libc::NCCS {
            tio.c_cc[libc::VDISCARD] = libc::_POSIX_VDISABLE as libc::cc_t;
        }
        #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"))]
        {
            tio.c_lflag &= !libc::FLUSHO;
        }
        let _ = unsafe { libc::tcsetattr(slave_fd, libc::TCSANOW, &tio) };
    }

    /// Set real tty to MC-style raw for relay (invoke_subshell: tcsetattr(STDOUT, &raw_mode)).
    /// Use STDOUT (fd 1) like MC. On 2nd+ run reuse cached relay-raw termios so we apply the same state as the first run (avoids TUI-modified termios causing uglified output).
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
                let _ = unsafe { libc::tcsetattr(1, libc::TCSANOW, r) };
            }
        });
        Some(saved)
    }

    fn restore_real_tty(saved: Option<libc::termios>) {
        if let Some(tio) = saved {
            let _ = unsafe { libc::tcsetattr(1, libc::TCSANOW, &tio) };
        }
    }

    /// Set real tty to relay raw and return saved termios. Call before LeaveAlternateScreen so all bytes after (reset, relay) go through one path. Pass result to run_relay_until_ctrl_o(..., Some(prepared)).
    #[cfg(unix)]
    pub fn prepare_for_relay() -> PreparedRelay {
        PreparedRelay(Self::set_real_tty_relay_raw())
    }

    /// Write the full relay transition: leave alternate, show cursor, disable mouse, CAN, reset (wrap, G0/G1, SGR, keypad, scroll, cursor). Caller must set relay raw first and pass prepared to run_relay. See REFERENCE.md "Solution: Second Ctrl+O uglification (macOS Terminal.app)" — do not use backend for the switch; this sequence must stay in sync with that doc.
    #[cfg(unix)]
    pub fn write_relay_reset_sequence<W: Write>(w: &mut W) -> io::Result<()> {
        w.write_all(b"\x1b[?1049l\x1b[?47l")?; // leave alternate (main screen)
        w.write_all(b"\x1b[?25h")?; // show cursor (DECTCEM)
        w.write_all(b"\x1b[?1002l\x1b[?1006l")?; // disable mouse (X10, SGR)
        w.write_all(b"\x18")?; // CAN: parser to ground
        #[cfg(not(target_os = "macos"))]
        w.write_all(b"\x1b[!p")?; // DECSTR (skip on macOS Terminal.app)
        w.write_all(b"\x1b[?7h\x1b(B\x1b)B\x1b[0m")?; // wrap, G0/G1 ASCII, SGR
        w.write_all(b"\x1b>")?; // numeric keypad
        w.write_all(b"\x1b[r\x1b[999;999H\r")?; // scroll region, cursor to bottom
        w.flush()
    }

    /// Set PTY window size to match the real terminal (stdout). MC: tty_resize in lib/tty/tty.c.
    /// Copy full winsize (including ws_xpixel/ws_ypixel) so ls/commands format correctly.
    fn resize_pty_to_terminal(master_fd: i32) {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        #[cfg(target_os = "macos")]
        let get_ok = unsafe { libc::ioctl(1, libc::TIOCGWINSZ as libc::c_ulong, &mut ws) } == 0;
        #[cfg(not(target_os = "macos"))]
        let get_ok = unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0;
        if get_ok && (ws.ws_col > 0 || ws.ws_row > 0) {
            #[cfg(target_os = "macos")]
            let _ = unsafe { libc::ioctl(master_fd, libc::TIOCSWINSZ as libc::c_ulong, &ws) };
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
                    libc::ioctl(slave_fd, libc::TIOCSCTTY as libc::c_ulong, 0);
                    Self::set_pty_slave_cooked_mode(slave_fd);
                    libc::dup2(slave_fd, 0);
                    libc::dup2(slave_fd, 1);
                    libc::dup2(slave_fd, 2);
                    libc::close(slave_fd);
                }
                let _ = std::env::set_current_dir(cwd);
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                let err = Command::new(&shell)
                    .arg("-i")
                    .current_dir(cwd)
                    .exec();
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

    /// Full-screen relay: stdin → pty, pty → stdout, until Ctrl+O (0x0F) on stdin. Shell keeps running.
    /// Caller must leave alternate screen before calling and re-enter after return (see main.rs Suspend).
    /// If `prepared` is Some, caller already set relay raw and wrote reset sequence to stdout; we skip that and use saved termios for restore. If None, we set raw and write reset ourselves.
    /// If `show_prompt_first` is true (Ctrl+O toggle), send " \b" and flush PTY so the prompt is visible.
    pub fn run_relay_until_ctrl_o(&self, show_prompt_first: bool, prepared: Option<PreparedRelay>) -> io::Result<()> {
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
                let _ = Self::write_all_fd(1, b"\x18");
                #[cfg(not(target_os = "macos"))]
                let _ = Self::write_all_fd(1, b"\x1b[!p");
                let _ = Self::write_all_fd(1, b"\x1b[?7h\x1b(B\x1b)B\x1b[0m");
                let _ = Self::write_all_fd(1, b"\x1b>");
                let _ = Self::write_all_fd(1, b"\x1b[r\x1b[999;999H\r");
                saved
            }
        };

        if show_prompt_first {
            // Drain any stale PTY output from the previous session so the new prompt is not mixed with old data.
            let _ = Self::drain_pty_output(self.master_fd);
            // Force a new prompt every time: send newline so the shell prints a fresh prompt (works on 2nd+ attempt).
            let _ = Self::write_all_fd(self.master_fd, b"\r\n");
            // MC: " \b" hack so prompt reappears.
            let _ = Self::write_all_fd(self.master_fd, b" \x08");
            // Brief yield so the shell can write the new prompt before we start reading.
            std::thread::sleep(std::time::Duration::from_millis(20));
            let _ = Self::flush_pty_prompt_to_stdout(self.master_fd);
        }

        // MC uses PTY_BUFFER_SIZE (512); use 4K so long ls lines aren't fragmented and we don't lose info.
        let mut stdin_buf = [0u8; 256];
        let mut pty_buf = [0u8; 4096];
        let mut stdin_carry: Vec<u8> = Vec::with_capacity(32);

        let relay_result = (|| -> io::Result<()> {
            loop {
                let mut fds = [
                    PollFd::new(unsafe { BorrowedFd::borrow_raw(0) }, PollFlags::POLLIN),
                    PollFd::new(
                        unsafe { BorrowedFd::borrow_raw(self.master_fd) },
                        PollFlags::POLLIN,
                    ),
                ];
                match poll(&mut fds, 100u16) {
                    Ok(0) => continue,
                    Ok(_) => {}
                    Err(Errno::EINTR) => continue,
                    Err(_) => break,
                }

                if fds[0]
                    .revents()
                    .map_or(false, |r| r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP))
                {
                    match unistd::read(0, &mut stdin_buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if self.relay_stdin_chunk(&mut stdin_carry, &stdin_buf[..n])? {
                                let _ = Self::drain_pty_output(self.master_fd);
                                return Ok(());
                            }
                        }
                        Err(Errno::EINTR) => {}
                        Err(_) => break,
                    }
                }

                if fds[1]
                    .revents()
                    .map_or(false, |r| r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP))
                {
                    match Self::read_pty_nonblock(self.master_fd, &mut pty_buf)? {
                        Some(0) => break,
                        Some(n) => {
                            Self::write_all_fd(1, &pty_buf[..n])?;
                        }
                        None => {} // EAGAIN, no data this time
                    }
                }
            }
            Ok(())
        })();
        Self::restore_real_tty(real_tty_saved);
        relay_result
    }

    /// Escape path for shell (single-quote style so spaces/special chars are safe).
    fn shell_escape_path(path: &str) -> String {
        format!("'{}'", path.replace('\'', "'\"'\"'"))
    }

    /// Change shell cwd to match the active panel then relay until Ctrl+O (for Suspend so ls matches panel).
    /// Sends `cd 'cwd'` then flushes the shell output (echoed cd + prompt) to stdout and relays without
    /// forcing an extra newline/prompt, so the user sees a single clean command line.
    pub fn run_cd_then_relay(&self, cwd: &str, prepared: Option<PreparedRelay>) -> io::Result<()> {
        let cd_escaped = Self::shell_escape_path(cwd);
        let mut buf = Vec::with_capacity(8 + cd_escaped.len() + 2);
        buf.extend_from_slice(b"cd ");
        buf.extend_from_slice(cd_escaped.as_bytes());
        buf.push(b'\n');
        Self::write_all_fd(self.master_fd, &buf)?;
        let _ = Self::drain_pty_output(self.master_fd);
        self.run_relay_until_ctrl_o(false, prepared)
    }

    /// Run a command in the subshell then relay until Ctrl+O (MC: invoke_subshell with command).
    /// If `prepared` is Some, caller already set relay raw and wrote reset to stdout (single-writer flow).
    pub fn run_command_then_relay(&self, cwd: &str, cmd: &str, prepared: Option<PreparedRelay>) -> io::Result<()> {
        let cd_escaped = Self::shell_escape_path(cwd);
        let mut buf = Vec::with_capacity(8 + cd_escaped.len() + cmd.len() + 2);
        buf.extend_from_slice(b"cd ");
        buf.extend_from_slice(cd_escaped.as_bytes());
        buf.push(b'\n');
        buf.extend_from_slice(cmd.as_bytes());
        buf.push(b'\n');
        Self::write_all_fd(self.master_fd, &buf)?;
        self.run_relay_until_ctrl_o(false, prepared)
    }
}

#[cfg(unix)]
impl Drop for Subshell {
    fn drop(&mut self) {
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
    pub fn write(&self, _data: &[u8]) -> io::Result<usize> {
        Ok(0)
    }
    pub fn run_relay_until_ctrl_o(&self, _show_prompt_first: bool, _prepared: Option<PreparedRelay>) -> io::Result<()> {
        Ok(())
    }
    pub fn run_cd_then_relay(&self, _cwd: &str, _prepared: Option<PreparedRelay>) -> io::Result<()> {
        Ok(())
    }
    pub fn run_command_then_relay(&self, _cwd: &str, _cmd: &str, _prepared: Option<PreparedRelay>) -> io::Result<()> {
        Ok(())
    }
}
