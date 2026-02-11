---
name: rust-reviewer
description: Review Rust code for correctness, safety, idiomatic patterns, and common AI-generated code smells. Use when reviewing Rust code quality or before merging.
user-invocable: true
---

# Rust Code Reviewer

Review Rust code targeting the specific failure modes that AI-generated and human-written Rust most commonly gets wrong. Not a generic code review — Rust-specific.

## Review Checklist (ordered by severity)

### 1. Unsafe Code Audit (critical)

First: **is unsafe even necessary?**
- FFI calls → yes, required
- Raw pointer deref → yes, but can it be restructured to avoid raw pointers?
- Mutable statics → yes, but consider `OnceCell`, `LazyLock`, or atomics instead
- Implementing `Send`/`Sync` → yes, but verify the type genuinely satisfies the contract
- Bypassing borrow checker → **no. The design is wrong. Redesign it.**
- Performance → maybe. Must have measured proof.

For every `unsafe` block that IS necessary:
- [ ] `// SAFETY:` comment explains **why** invariants hold (not just "this is safe")
- [ ] Block scope is minimal
- [ ] Pointer validity: non-null, points to allocated memory, properly aligned, valid bit pattern
- [ ] Aliasing: no simultaneous `&T` and `&mut T` to same data, no two `&mut T` to same data
- [ ] Initialization: memory initialized before read, `MaybeUninit` used correctly
- [ ] Type invariants: transmutes maintain validity, `#[repr(C)]` where layout matters, UTF-8 valid for strings
- [ ] Concurrency: `Send`/`Sync` contracts genuine, no data races, correct atomic ordering
- [ ] Drop safety: `drop_in_place` exactly once, no double-free, correct allocator
- [ ] Encapsulation: unsafe wrapped behind a safe API, invariants enforced by construction

```rust
// BAD: // SAFETY: this is fine
// BAD: // SAFETY: we know the pointer is valid

// GOOD:
// SAFETY: `ptr` was obtained from `Box::into_raw` in `new()` and has not
// been freed since. The Box was allocated with the global allocator, same
// as used by `Box::from_raw`. Non-null guaranteed by Box.
```

### 2. Lifetime Correctness

- [ ] Lifetime annotations trace actual data flow, not guessed
- [ ] No hidden `'static` assumptions that break callers
- [ ] Struct lifetimes match actual borrow durations of their fields
- [ ] No unnecessary lifetime parameters (elision handles most cases)
- [ ] Self-referential patterns avoided or correctly handled with `Pin`

### 3. Async Correctness

- [ ] No blocking operations (`std::fs`, `std::net`, `thread::sleep`) in async fns
- [ ] No `std::sync::MutexGuard` held across `.await` points
- [ ] Spawned tasks: all captured data is `Send + 'static`
- [ ] No mixing async runtimes (tokio vs async-std)
- [ ] Cancellation safety considered for futures that may be dropped mid-await
- [ ] Trait objects: boxed with correct `Send`/lifetime bounds
- [ ] `?` works across `.await` (error types compatible)

### 4. Ownership Smells

- [ ] **Excessive `.clone()`**: each one should have a reason, not a borrow-checker escape
- [ ] **`Rc`/`Arc` overuse**: shared ownership should be intentional
- [ ] **Unnecessary `Box<dyn Trait>`**: could this be `impl Trait` instead?
- [ ] **`String` where `&str` suffices** in function params (and vice versa in struct fields)

### 5. Error Handling

- [ ] No `unwrap()` / `expect()` on fallible paths (network, IO, user input, parsing)
- [ ] `unwrap()` acceptable in: tests, provably infallible cases with comment, early `main()` setup
- [ ] Error types implement `std::error::Error` + `Display`
- [ ] Error context preserved across conversions (`.context()` or `map_err`)
- [ ] `?` used consistently

### 6. API Design

- [ ] Rust conventions: `new()`, `builder()`, `into_*`/`as_*`/`to_*` naming
- [ ] `#[must_use]` where ignoring return value is likely a bug
- [ ] Standard trait impls: `Debug`, `Display`, `Clone`, `PartialEq` where appropriate
- [ ] Functions accept borrows (`&str`, `&[T]`, `&Path`) when they don't need ownership

### 7. Idiomatic Patterns

- [ ] Iterators over manual indexing
- [ ] Pattern matching over if-else chains on enums
- [ ] `if let` / `let else` for single-variant matches
- [ ] No C-style `for i in 0..v.len() { v[i] }` — use `for item in &v`
- [ ] `Default` derive/impl where a zero value makes sense

### 8. Dependencies & Imports

- [ ] Imports resolve to actual paths in the dependency tree
- [ ] No unused imports
- [ ] No deprecated API usage
- [ ] Feature flags documented if enabling optional crate features

## Output Format

Present findings grouped by severity:
1. **Blocking** — must fix (unsafe soundness holes, lifetime bugs, compilation errors)
2. **Should fix** — correctness or significant quality (error handling, ownership smells, async pitfalls)
3. **Suggestions** — idiomatic improvements, style (won't break anything)
