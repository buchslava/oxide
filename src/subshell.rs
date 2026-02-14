//! PTY subshell for Ctrl+O (toggle shell); command line runs in the original terminal.
//!
//! Ctrl+O flow (MC-style, see REFERENCE.md): leave alternate screen, relay stdin↔PTY until
//! Ctrl+O (0x0F) is read from stdin; then caller re-enters alternate and redraws. Raw mode
//! stays on so we can detect Ctrl+O; the subshell runs in a PTY with its own termios.

use std::io;

#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
const CTRL_O: u8 = 0x0F;

/// Persistent subshell: command line runs here, Ctrl+O toggles full-screen relay.
#[cfg(unix)]
pub struct Subshell {
    master_fd: i32,
    child_pid: i32,
}

#[cfg(unix)]
impl Subshell {
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

    /// Write data to the shell (e.g. command line + newline). Non-blocking friendly.
    pub fn write(&self, data: &[u8]) -> io::Result<usize> {
        nix::unistd::write(unsafe { BorrowedFd::borrow_raw(self.master_fd) }, data)
            .map_err(io::Error::from)
    }

    /// Read any data currently available from the shell (non-blocking).
    pub fn read_available(&self) -> io::Result<Vec<u8>> {
        use nix::errno::Errno;
        use nix::poll::{poll, PollFd, PollFlags};

        let mut fds = [PollFd::new(
            unsafe { BorrowedFd::borrow_raw(self.master_fd) },
            PollFlags::POLLIN,
        )];
        match poll(&mut fds, 0u16) {
            Ok(0) => return Ok(Vec::new()),
            Ok(_) => {}
            Err(Errno::EINTR) => return Ok(Vec::new()),
            Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
        }
        if !fds[0].revents().map_or(false, |r| r.contains(PollFlags::POLLIN)) {
            return Ok(Vec::new());
        }
        let mut buf = [0u8; 4096];
        match nix::unistd::read(self.master_fd, &mut buf) {
            Ok(0) => Ok(Vec::new()),
            Ok(n) => Ok(buf[..n].to_vec()),
            Err(Errno::EINTR) => Ok(Vec::new()),
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        }
    }

    /// Full-screen relay: stdin → pty, pty → stdout, until Ctrl+O (0x0F) on stdin. Shell keeps running.
    /// Caller must leave alternate screen before calling and re-enter after return (see main.rs Suspend).
    pub fn run_relay_until_ctrl_o(&self) -> io::Result<()> {
        use nix::errno::Errno;
        use nix::poll::{poll, PollFd, PollFlags};
        use nix::unistd;

        // Sync PTY size to real terminal so ls/commands format correctly (MC: tty_resize on SIGWINCH).
        Self::resize_pty_to_terminal(self.master_fd);

        // MC uses PTY_BUFFER_SIZE (512); use 4K so long ls lines aren't fragmented and we don't lose info.
        let mut stdin_buf = [0u8; 256];
        let mut pty_buf = [0u8; 4096];

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

            if fds[0].revents().map_or(false, |r| r.contains(PollFlags::POLLIN)) {
                match unistd::read(0, &mut stdin_buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        for &b in &stdin_buf[..n] {
                            if b == CTRL_O {
                                return Ok(());
                            }
                        }
                        let _ = unistd::write(
                            unsafe { BorrowedFd::borrow_raw(self.master_fd) },
                            &stdin_buf[..n],
                        );
                    }
                    Err(Errno::EINTR) => {}
                    Err(_) => break,
                }
            }

            if fds[1].revents().map_or(false, |r| r.contains(PollFlags::POLLIN)) {
                match unistd::read(self.master_fd, &mut pty_buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = unistd::write(unsafe { BorrowedFd::borrow_raw(1) }, &pty_buf[..n]);
                    }
                    Err(Errno::EINTR) => {}
                    Err(_) => break,
                }
            }
        }
        Ok(())
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

/// Run a command in the original terminal (leave TUI, run, re-enter is done by caller).
/// Spawns sh -c "cmd" with inherited stdin/stdout/stderr; blocks until command finishes.
pub fn run_command_in_terminal(cwd: &str, cmd: &str) -> io::Result<()> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let _ = std::process::Command::new(&shell)
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()?
        .wait()?;
    Ok(())
}

#[cfg(not(unix))]
pub struct Subshell;

#[cfg(not(unix))]
impl Subshell {
    pub fn spawn(_cwd: &str) -> io::Result<Self> {
        Ok(Self)
    }
    pub fn write(&self, _data: &[u8]) -> io::Result<usize> {
        Ok(0)
    }
    pub fn run_relay_until_ctrl_o(&self) -> io::Result<()> {
        Ok(())
    }
}
