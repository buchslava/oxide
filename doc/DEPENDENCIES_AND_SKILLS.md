# Dependencies and Cursor / AI skills

## Dependencies

- `chrono` — Date/time
- `ratatui` — TUI framework
- `crossterm` — Terminal I/O
- `dirs` — Home directory
- `ratatui-code-editor` — Embedded editor
- `libc`, `nix` — Unix-only (permissions, subshell PTY)
- `zip` — ZIP archives (panel backend)
- `tar`, `flate2` — tar.gz archives (panel backend)

## Cursor / AI assistant skills

This repo includes [agent skills](https://github.com/sickn33/antigravity-awesome-skills) under `.cursor/skills/` for use in Cursor Chat (e.g. `@rust-pro`). They are vendored from the community collection [**antigravity-awesome-skills**](https://github.com/sickn33/antigravity-awesome-skills) (MIT).

| Skill | Location | Upstream source |
|--------|----------|-----------------|
| **rust-pro** | `.cursor/skills/rust-pro/SKILL.md` | [`skills/rust-pro`](https://github.com/sickn33/antigravity-awesome-skills/tree/main/skills/rust-pro) |
| **rust-async-patterns** | `.cursor/skills/rust-async-patterns/SKILL.md` | [`skills/rust-async-patterns`](https://github.com/sickn33/antigravity-awesome-skills/tree/main/skills/rust-async-patterns) (includes `resources/implementation-playbook.md`) |
| **posix-shell-pro** | `.cursor/skills/posix-shell-pro/SKILL.md` | [`skills/posix-shell-pro`](https://github.com/sickn33/antigravity-awesome-skills/tree/main/skills/posix-shell-pro) |

The upstream `posix-shell-pro` skill references a playbook file that is not shipped in that repository; this project adds `.cursor/skills/posix-shell-pro/resources/implementation-playbook.md` as a short pointer so that instruction is not a dead link.

### Project-local skills

Skills and design notes maintained in this repository (YAML frontmatter where applicable). Reference them in Chat via path or `@single-responsibility` / `@`-mention if your Cursor setup indexes `.cursor/skills/`.

| Topic | Location |
|-------|----------|
| **single-responsibility** — SRP for Rust/Oxide, and Oxide panel locations / refresh / archives | [`.cursor/skills/single-responsibility/SKILL.md`](../.cursor/skills/single-responsibility/SKILL.md) |
