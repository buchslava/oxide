# Rust code quality and hardening plan

This document turns the [Rust codebase review](../.cursor/skills/rust-pro/SKILL.md) (panics, indexing, Clippy, panel refresh invariants, maintainability) into a **prioritized execution plan**. It does not replace `doc/FEATURES.md` or `doc/DEPENDENCIES_AND_SKILLS.md`; it tracks **engineering follow-ups** only.

---

## Goals

1. **Reduce panic risk** on paths that real users and integrations can hit (not only tests).
2. **Lock in behavior** that the product already relies on (especially panel refresh and copy/move/delete).
3. **Raise the lint bar** so regressions are caught in CI, not only by manual review.
4. **Shrink change blast radius** in very large modules without rewriting the app in one pass.

---

## Phase 1 — Quick wins (low risk, small diffs)

| ID | Item | Rationale | Suggested work |
|----|------|-----------|----------------|
| 1.1 | Bottom bar size line `Option` handling | Today guarded by `is_some()` then `unwrap()`; a future edit could desync condition and unwrap. | In `src/ui/renderer.rs` (`draw_bottom_file_bar`), replace the `is_some()` + `unwrap()` pairs with `if let Some(line) = size_info_line` (or `match`) for left and right branches. |
| 1.2 | Redundant selection clamp in `refresh_files` | After `selected_index = 0`, the `selected_index >= files.len()` branch is redundant for non-empty lists; noise for readers. | In `src/browser/panel.rs`, either remove the redundant block or fold into a single clamp helper used by both `refresh_files` and `refresh_files_restore_selection` if you want one code path. |
| 1.3 | `get_names_to_copy` `.expect` | Invariant is real (`marked_indices` non-empty in that branch); `.expect` is honest but triggers strict lint policies. | Replace with `if let (Some(f), Some(l)) = (min, max)` or `debug_assert!` + safe fallback that returns empty (should be unreachable in release). Pick one style and document the invariant in a one-line comment. |

**Exit criteria:** No behavior change intended; `cargo test` and manual smoke (open app, both panels, bottom bar with size dialog) unchanged.

---

## Phase 2 — CI and static analysis

| ID | Item | Rationale | Suggested work |
|----|------|-----------|----------------|
| 2.1 | Clippy in CI | Default `cargo clippy` already passes; stricter lints catch new `unwrap`/`expect` on production paths. | Add a CI job (or local script) running e.g. `cargo clippy --all-targets -- -W clippy::unwrap_used -W clippy::expect_used` **or** a curated allowlist: start with `-D warnings` and add targeted denies (project policy). Allow tests via `cfg_attr(test, allow(...))` where appropriate. |
| 2.2 | Document lint policy | Contributors need a single source of truth. | Short section in `doc/BUILD_LINUX.md` or root `README` (if present): exact `clippy` invocation and how to run the same locally. |

**Exit criteria:** CI fails on newly introduced `unwrap`/`expect` in non-test code according to the policy you choose.

---

## Phase 3 — Behavior tests (highest value for panel/copy flows)

| ID | Item | Rationale | Suggested work |
|----|------|-----------|----------------|
| 3.1 | Deleted current directory | Core resilience story; already described in skills / `doc/FEATURES.md`. | Integration or `browser::panel::tests`-style test: two temp dirs, panel A in a subfolder, panel B deletes that folder (or rename), then `refresh_files_restore_selection` / full refresh path; assert listing path climbed and no panic. |
| 3.2 | Copy with empty `items` (edge) | Unusual but possible from UI race or future caller. | Unit test: `start_copy_operation` + `run_copy_step` with `items: vec![]` completes without indexing and clears state consistently (overlay counts if applicable). |
| 3.3 | Marked diff pair | `two_marked_files` uses `v.len() == 2` before indexing. | Light regression test: exactly two marked files → `Some`; three → `None`. |

**Exit criteria:** New tests pass on Linux (and macOS if that is a supported dev target); no flaky timing (avoid wall-clock sleeps unless unavoidable).

---

## Phase 4 — Architecture and maintainability (SRP)

| ID | Item | Rationale | Suggested work |
|----|------|-----------|----------------|
| 4.1 | Large coordinators | `events.rs`, `mouse.rs`, `copy_runner.rs` carry many reasons to change. | Incremental extraction only: e.g. one self-contained helper module per concern (mouse hit-testing, copy overwrite dialog handling) **without** a big-bang refactor. |
| 4.2 | Panel vs app boundaries | Single-responsibility skill already maps files. | When touching refresh or `PanelLocation`, keep `panel_refresh.rs` as the orchestration edge; avoid duplicating climb logic in new call sites. |

**Exit criteria:** Each PR that touches these areas either keeps modules stable or moves **one** clear boundary (documented in the PR description).

---

## Phase 5 — `unsafe` and platform surfaces (audit, not bulk rewrite)

| ID | Item | Rationale | Suggested work |
|----|------|-----------|----------------|
| 5.1 | PTY / subshell | `src/shell/subshell.rs` uses `libc` and raw fds; bugs are security and stability sensitive. | Scheduled read-through: document invariants (fd lifetime, child pid, signal handling) in module-level `//!` comments; pair review after substantive changes. |
| 5.2 | Unix metadata / `chown` | `src/core/file_ops.rs` unsafe blocks wrap libc. | Same: narrow review when changing ownership or permission paths. |

**Exit criteria:** No new `unsafe` without a comment stating **preconditions** and **why** safe wrapping is correct.

---

## Suggested order of execution

1. **Phase 1** (1–2 small PRs).
2. **Phase 2** once Phase 1 is merged (CI noise stays low).
3. **Phase 3** in parallel with Phase 2 if multiple people are available.
4. **Phase 4** opportunistically whenever a feature already touches a large file.
5. **Phase 5** before large subshell or file_ops changes, or on a fixed cadence (e.g. quarterly).

---

## References

- Rust review checklist: `.cursor/skills/rust-pro/SKILL.md` (Code quality bar).
- Panel refresh and climb invariants: `.cursor/skills/single-responsibility/SKILL.md` (Oxide panel responsibilities).
- User-facing behavior: `doc/FEATURES.md`.

---

## Tracking

Use your issue tracker of choice: create one issue per **ID** (1.1, 2.1, …) and link PRs. Close this meta-doc only when **Phases 1–3** are done if you want a clear “baseline hardening” milestone.
