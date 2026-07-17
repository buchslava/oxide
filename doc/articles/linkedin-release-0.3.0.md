# LinkedIn release — Oxide 0.3.0

Oxide **0.3.0** is out — still that dual-panel terminal file manager in the Midnight Commander vibe, written in Rust for Linux and macOS, with a real subshell when you actually need a shell.

The headline this time is **markdown in the terminal**. Open a markdown with **F3**, and you get a rendered preview — not a raw dump: headings, lists, tables, fenced code with **syntax highlighting** for common languages, and local embedded images  drawn with the same Kitty / iTerm2 / Sixel / half-blocks path as the image gallery. Browse docs like a small reader: **Tab** cycles links, **Enter** (or click) follows them, **Backspace** walks the nav stack, and **T** flips back to plain text when you need the source.

Also shipping with this release: a more reliable **clipboard on ARM / Raspberry Pi** (Linux Wayland / aarch64) — Ctrl+C / Ctrl+V in the command line and dialogs should behave better on Pi-style setups, not only classic X11 — plus a right-click **panel context menu**.

If you live in the terminal and keep READMEs next to the code, this one is worth a pull. Prebuilt binaries (including **aarch64**) and source: https://github.com/buchslava/oxide — full notes: https://github.com/buchslava/oxide/blob/main/CHANGELOG.md#030---2026-07-17
