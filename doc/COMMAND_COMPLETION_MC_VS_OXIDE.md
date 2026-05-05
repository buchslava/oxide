# Command completion: Midnight Commander vs Oxide

When a file manager runs a shell command (for example `top`) and then needs to **restore the panel UI** or **know that the foreground command finished**, it must detect completion without confusing that with normal interactive output (password prompts, pagers, TUIs).

This note compares how **Midnight Commander (MC)** and **Oxide** solve that problem. MC lives alongside this repo as `../mc` in many setups; paths below refer to that tree.

Related Oxide background: [SHELL_PTY_TERMINAL.md](./SHELL_PTY_TERMINAL.md) (PTY, relay, Ctrl+O).

---

## What the user sees

**Oxide** (with auto-return enabled) sends a **single logical line** to the existing subshell PTY, roughly:

```sh
eval 'top'; printf '\n' > '/var/folders/.../oxide_cmd_<ppid>_<shell_pid>.fifo'
```

The `printf` runs in the **same shell** only after `eval 'top'` returns, so a newline on the FIFO is a **direct completion signal** for that wrapped command.

**MC** sends the user command **as typed** (plus a newline), for example `top\n`, without appending `printf` to the same line. Completion is **not** tied to a postfix on that line; instead MC relies on machinery installed at subshell startup (see below).

---

## MC: prompt-time synchronization with a pipe and SIGSTOP

### Initialization (once per subshell)

MC injects shell-specific startup code via `init_subshell_precmd()` in `../mc/src/subshell/common.c`. The pattern is:

1. Hook **just before the prompt is shown** (bash `PROMPT_COMMAND`, zsh `precmd_functions`, ksh/mksh/dash PS1 command substitution, fish `fish_prompt` wrapper, tcsh `precmd` alias, etc.).
2. In that hook, write the current working directory to a **side fd** (`pwd >&<n>`) so the parent can read `subshell_cwd`.
3. Immediately **`kill -STOP $$`** so the shell process **stops** and MC can run its `select` loop, read the pipe, set `subshell_ready`, and optionally parse the prompt from PTY output.

Representative strings (abridged; see source for full quoting and version branches):

- **Bash**: append to `PROMPT_COMMAND` a fragment like `pwd >&%d; kill -STOP $$` (fd is `subshell_pipe[WRITE]`).
- **Zsh**: `_mc_precmd()` in `precmd_functions` with the same idea.
- **POSIX-style shells**: `PS1='$(pwd >&%d; kill -STOP $$)'"$PS1"` or a hop via `MC_PRECMD` so nested subshells do not run a missing function.
- **Tcsh**: `mkfifo` path under `mc_tmpdir()`, opened `O_RDWR` to avoid open deadlock; `precmd` writes `$cwd` to that fifo and stops.

### When a user command finishes

In `feed_subshell()`, MC waits on the PTY **and** `subshell_pipe[READ]`. When the shell reaches the next prompt, the precmd runs first: CWD is written, the shell stops, MC reads the pipe, sets `subshell_ready`, and if `subshell_state == RUNNING_COMMAND` it treats the session as finished and returns to panels.

So completion is defined as **“the shell is about to print a prompt again”**, not as **“this exact input line finished”** as a separate syntactic unit—though for normal foreground commands those coincide.

### Costs and constraints (MC)

- **Shell-specific surface area**: large `switch` on shell type, different hooks, comments about bash 5 `PROMPT_COMMAND` array attributes, fish quirks, tcsh fifo workaround, physical line length limits for cooked TTY input (`COOKED_MODE_BUFFER_SIZE` / comments around `#4480`).
- **SIGSTOP on the interactive shell**: the entire shell stops so MC can synchronize. That is powerful but invasive (debuggers, expectations around job control, anything that assumes the shell keeps running across prompt draws).
- **Prompt parsing**: after the pipe read, MC may scan PTY output to refresh the stored prompt string (`parse_subshell_prompt_string`), which is inherently tied to terminal bytes, newlines, and user themes.

---

## Oxide: per-command FIFO (or PTY marker fallback)

### Where it lives

Implementation and design notes: `src/shell/subshell.rs` (module docs at the top describe the FIFO, `sudo -s`, and `pending_command_done`).

### Normal path: named FIFO + `eval` + postfix `printf`

When `Subshell::run_command_then_relay` is called with an auto-return delay:

1. Oxide creates a **named FIFO** under the temp directory: `oxide_cmd_<parent_pid>_<subshell_child_pid>.fifo`, removes any stale path, `mkfifo`, then opens the read side **non-blocking** before injecting the shell line.
2. It sends: `eval <quoted_cmd>; printf '\n' > <quoted_fifo_path>` (one newline-terminated line).
3. The relay loop **polls** stdin, the PTY master, and the FIFO fd. Data on the PTY is relayed unchanged (sudo prompts, full-screen programs, etc.).
4. When the shell finally runs `printf`, the reader gets input on the FIFO → Oxide treats the command as finished (subject to idle delay / Ctrl+O behavior encoded in `RelayExit`).

**Why `eval`**: the user command is passed as a **single quoted shell word** so semicolons, pipes, and newlines inside the user string do not break the `; printf` postfix.

### Fallback: PTY stream marker

If `mkfifo` / open fails, Oxide falls back to appending `printf '%s\n' '<random OXD_…>'` after `eval`, and scans the PTY stream for that token (documented in code as more fragile with echo and short reads).

### Edge case: early panel reopen (`sudo -s`)

If the user starts an inner root shell, Oxide may return to the panels **before** the outer `eval …; printf` completes. The FIFO reader must stay alive (`pending_command_done`) or `Drop` would unlink the FIFO and the eventual `printf` would never signal completion—documented in the same module.

---

## Side-by-side summary

| Aspect | MC (`../mc/src/subshell/common.c`) | Oxide (`src/shell/subshell.rs`) |
|--------|-------------------------------------|----------------------------------|
| **Completion notion** | Next **prompt** (precmd / PS1 hook) | **`eval` compound command** returned; postfix `printf` runs |
| **Side channel** | Pipe (or tcsh FIFO) for **cwd** + synchronization | Named FIFO (or PTY marker) for **done** |
| **Shell setup** | Injects hooks into bash/zsh/fish/tcsh/… at subshell init | No `PROMPT_COMMAND` / `precmd` injection for this feature |
| **Process control** | **`kill -STOP $$`** on the shell | No stop signal; shell keeps running normally |
| **Command line on wire** | User command only (`invoke_subshell`) | Wrapped `eval '…'; printf …` when auto-return is on |
| **CWD after command** | Written from shell precmd into MC’s buffer | Oxide uses separate cwd probing / `cd` sync paths (not the FIFO byte) |

---

## Pros of Oxide’s approach (and honest tradeoffs)

### Pros

1. **Completion is syntactic, not prompt-themed**  
   The signal is “this **`eval …; printf`** line finished”. That tracks nested situations where “prompt appearance” and “outer shell line finished” can diverge; Oxide already documents **`sudo -s`** / inner shells and keeps a FIFO reader across early UI return.

2. **No prompt-hook injection**  
   MC’s design requires rewriting `PROMPT_COMMAND`, `PS1`, zsh `precmd_functions`, fish prompt, tcsh `precmd`, etc. User rc files can interact with those hooks; MC carries workarounds (e.g. `MC_PRECMD` indirection for subshells-of-subshells). Oxide does not depend on the user’s prompt configuration for **run command then return**.

3. **No SIGSTOP on the shell**  
   Stopping the shell is a strong synchronization primitive for MC but surprising globally (tools that attach to the shell, timing, “why did my shell freeze”). Oxide only waits on **`poll`**-visible I/O.

4. **PTY stream left alone for “done”** (FIFO path)  
   Completion does not require spotting a magic string in terminal output, avoiding collisions with command output, sudo, or themes—aligned with the in-code comment that the FIFO avoids scanning the PTY for completion when possible.

5. **One implementation for “run this command”**  
   FIFO + `poll` is the same idea on bash/zsh/dash/… as long as the line is valid for that shell’s `eval`. MC maintains parallel precmd strategies per shell family.

6. **Graceful degradation**  
   If FIFO creation fails, Oxide still has a deliberate PTY-marker fallback; MC effectively depends on the precmd channel for normal operation.

### Tradeoffs (Oxide)

- The **injected line** includes `eval` and `printf`; it may appear in scrollback/history differently from MC’s “bare” command line for the same action.
- **Requires `mkfifo` + temp path** (permissions, disk) for the best path; MC mostly uses an anonymous `pipe()` for non-tcsh shells.
- **`eval` semantics**: the shell parses the string; untrusted input and `eval` are a classic pairing—Oxide must quote carefully (it uses shell escaping for the command and paths). MC runs the user command as a normal line without wrapping it in `eval` for `invoke_subshell`, but its precmd still executes arbitrary hook code MC installs.

---

## References

- Oxide: `src/shell/subshell.rs` — `OwnedFifo`, `run_command_then_relay`, `FifoCompletion`, `pending_command_done`.
- MC: `../mc/src/subshell/common.c` — `init_subshell_precmd`, `feed_subshell`, `invoke_subshell`, `parse_subshell_prompt_string`.
