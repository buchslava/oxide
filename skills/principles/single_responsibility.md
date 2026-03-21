---
id: srp-001
title: Single Responsibility Principle (SRP)
tags: [design, solid, architecture, rust, refactoring]
priority: high
---

# 🧠 Single Responsibility Principle (SRP)

A module, struct, or function should have **only one reason to change**.

> If you can describe something with "and", it probably violates SRP.

---

## 🚨 Why SRP matters

Violating SRP leads to:

- Hidden coupling
- Fragile code (one change breaks unrelated logic)
- Poor testability
- Difficult parallelization (important for Rust)

SRP-compliant code:

- Is composable
- Is easier to benchmark and optimize
- Maps naturally to Rust ownership model

---

## 🔍 How to detect SRP violations

### 1. "AND" rule

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

### 2. Name smells

If the name needs several nouns or clauses (`UserAndSessionAndEmailValidator`), or you keep adding `and` in the doc comment, split by noun/verb boundaries.

### 3. Test pain

When tests need heavy mocks for unrelated concerns (network + parser + clock in one test), responsibilities are tangled.

### 4. Change blast radius

A tweak to logging, persistence, or policy forces edits in the same function as parsing—that function is doing too much.

### 5. God types

Structs or modules that know about HTTP, SQL, serialization, and domain rules at once usually violate SRP. Prefer thin types and boundaries (`From`/`TryFrom`, dedicated services, traits for ports).

---

## 🦀 Rust-oriented patterns

- **Modules as boundaries**: `crate::io::`, `crate::domain::`, `crate::app::` — each folder owns one kind of change.
- **Traits for single capabilities**: `trait JobSource { fn load(&self) -> … }` keeps fetching behind one abstraction without mixing parsing.
- **Newtypes**: Separate `UserId` from `SessionToken` so validation and formatting live next to the right concept.
- **Avoid giant `impl` blocks**: If an `impl Widget` mixes layout, event handling, and persistence, extract submodules or helper types.
- **Async boundaries**: Keep pure CPU/business logic sync and testable; isolate `async` IO at the edges.

---

## 🛠 Refactoring toward SRP

1. List **reasons to change** for the code under review (API churn, formats, business rules, infra).
2. Pick the **smallest extractable unit** (pure function or small struct) and move it out.
3. **Compose** in a coordinator (`run_job`) that only wires dependencies — it should not embed business rules or IO details inline.
4. Add **tests per unit** so each responsibility has a narrow failure surface.
5. Stop when each piece has **one clear stakeholder** (e.g. "only DB migrations affect this").

---

## ✅ Quick checklist

- [ ] Can I name this without "and"?
- [ ] Would a change to storage force a change to parsing here?
- [ ] Can I test the core logic without network or disk?
- [ ] Does this type/module appear in unrelated feature discussions?

If any answer is wrong, consider splitting.

---

## 📚 See also

- Open/Closed Principle (extend behavior without editing stable cores)
- Hexagonal / ports-and-adapters (keep domain free of IO details)
- `rust-pro` / `rust-async-patterns` Cursor skills for idiomatic structure and async layering
