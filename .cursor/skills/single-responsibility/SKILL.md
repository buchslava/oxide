---
name: single-responsibility
description: >-
  Single Responsibility Principle (SRP) for Rust and Oxide architecture, plus Oxide
  file-manager panel responsibilities — PanelLocation, listing refresh, recovery when
  paths are deleted, and ZIP/tar.gz virtual folders. Use when refactoring boundaries,
  splitting modules, or editing panel/location/panel_backend/panel_refresh code.
---

## Use this skill when

- Judging whether a module or type should be split (SRP), or refactoring toward one reason to change.
- Changing how Oxide panels list directories or archives (`refresh_files`, `refresh_files_restore_selection`).
- Fixing bugs where a panel points at a deleted folder or a removed archive file.
- Extending `PanelLocation`, `panel_backend`, or post-operation refresh in `panel_refresh.rs`.
- Adding archive formats or copy/move/delete paths that must keep `current_location` valid.

## Do not use this skill when

- The task is generic Rust with no SRP or panel/location concern (use **rust-pro** / **rust-async-patterns**).
- Working only on UI rendering, dialogs, or keymaps without panel state or architecture boundaries.

---

## Single Responsibility Principle (SRP)

A module, struct, or function should have **only one reason to change**.

> If you can describe something with "and", it probably violates SRP.

### Why SRP matters

Violating SRP leads to:

- Hidden coupling
- Fragile code (one change breaks unrelated logic)
- Poor testability
- Difficult parallelization (important for Rust)

SRP-compliant code:

- Is composable
- Is easier to benchmark and optimize
- Maps naturally to Rust ownership model

### How to detect SRP violations

**1. "AND" rule**

```rust
// ❌ BAD: multiple responsibilities
fn process_job() {
    fetch_from_s3();   // IO
    parse_json();      // parsing
    calculate_cost();  // business logic
    save_to_db();      // IO
}
```

Each of those concerns can change for a different reason (S3 API, schema, pricing rules, DB schema). Split them:

```rust
// ✅ BETTER: one job each; compose at the edge
async fn fetch_object(client: &S3Client, key: &str) -> Result<Vec<u8>, Error> { /* … */ }

fn parse_job_payload(bytes: &[u8]) -> Result<JobPayload, Error> { /* … */ }

fn calculate_cost(payload: &JobPayload) -> Money { /* … */ }

async fn persist_result(pool: &PgPool, record: &JobResult) -> Result<(), Error> { /* … */ }
```

**2. Name smells** — If the name needs several nouns or clauses (`UserAndSessionAndEmailValidator`), or you keep adding `and` in the doc comment, split by noun/verb boundaries.

**3. Test pain** — When tests need heavy mocks for unrelated concerns (network + parser + clock in one test), responsibilities are tangled.

**4. Change blast radius** — A tweak to logging, persistence, or policy forces edits in the same function as parsing—that function is doing too much.

**5. God types** — Structs or modules that know about HTTP, SQL, serialization, and domain rules at once usually violate SRP. Prefer thin types and boundaries (`From`/`TryFrom`, dedicated services, traits for ports).

### Rust-oriented patterns

- **Modules as boundaries**: `crate::io::`, `crate::domain::`, `crate::app::` — each folder owns one kind of change.
- **Traits for single capabilities**: `trait JobSource { fn load(&self) -> … }` keeps fetching behind one abstraction without mixing parsing.
- **Newtypes**: Separate `UserId` from `SessionToken` so validation and formatting live next to the right concept.
- **Avoid giant `impl` blocks**: If an `impl Widget` mixes layout, event handling, and persistence, extract submodules or helper types.
- **Async boundaries**: Keep pure CPU/business logic sync and testable; isolate `async` IO at the edges.

### Refactoring toward SRP

1. List **reasons to change** for the code under review (API churn, formats, business rules, infra).
2. Pick the **smallest extractable unit** (pure function or small struct) and move it out.
3. **Compose** in a coordinator (`run_job`) that only wires dependencies — it should not embed business rules or IO details inline.
4. Add **tests per unit** so each responsibility has a narrow failure surface.
5. Stop when each piece has **one clear stakeholder** (e.g. "only DB migrations affect this").

### Quick checklist

- [ ] Can I name this without "and"?
- [ ] Would a change to storage force a change to parsing here?
- [ ] Can I test the core logic without network or disk?
- [ ] Does this type/module appear in unrelated feature discussions?

If any answer is wrong, consider splitting.

### See also (SRP)

- Open/Closed Principle (extend behavior without editing stable cores)
- Hexagonal / ports-and-adapters (keep domain free of IO details)
- **rust-pro** / **rust-async-patterns** Cursor skills for idiomatic structure and async layering

---

## Oxide panel responsibilities

Keep panel state, filesystem vs archive locations, and listing IO in clear layers. Related files:

| Area | Primary file |
|------|----------------|
| `PanelLocation` (`Fs` vs `Archive`), `enter` / `parent` | `src/core/location.rs` |
| Listing, read/write inside archives, copy into archive | `src/core/panel_backend.rs` |
| Panel state, climb on missing path, selection restore | `src/browser/panel.rs` |
| After copy/move/delete: which panel gets neighbor-based restore | `src/app/panel_refresh.rs` |

### Invariants after refresh

1. **`refresh_files` and `refresh_files_restore_selection`** must not leave the panel on a path that cannot be listed. Before listing, the panel **climbs** to a valid location if needed.
2. **Filesystem (`PanelLocation::Fs`)** — If `metadata` shows the path is not a directory (including missing), walk up with `climb_to_valid_fs_path` until a listable directory is found (stops at `/` if needed).
3. **Archive (`PanelLocation::Archive`)** — If the archive file on disk is gone, fall back to the parent **filesystem** directory (or `/`).
4. **`try_list_current_location`** — If `panel_backend::list` returns **`io::ErrorKind::NotFound`**, run the climb logic once and list again.

### Operations that trigger refresh

User **Ctrl+R**, safe delete, copy/move completion, shell commands that change the tree, and navigation that ends in `refresh_files()`. Any of these can run while the other panel (or an external process) has removed the current directory — the climb behavior avoids surfacing **os error 2** to the user for that case.

### Verification

- After changes, `cargo build` and, when possible, `cargo test browser::panel::tests`.
- Manually: open a folder in one panel, delete that folder from the other panel (or shell), then refresh — listing should show a parent path, not an error line.

### Related user-facing doc

- `doc/FEATURES.md` — resilient refresh when the current folder or on-disk archive is gone.
- `doc/DEPENDENCIES_AND_SKILLS.md` — indexes this skill.
