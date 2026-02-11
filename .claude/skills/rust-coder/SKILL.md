---
name: rust-coder
description: Write Rust code with awareness of known AI pitfalls — ownership, lifetimes, async, module integration, idiomatic patterns. Use when writing new Rust code or implementing features.
user-invocable: true
---

# Rust Coder

Write Rust code that compiles, is idiomatic, and integrates correctly into the project. This skill encodes specific failure modes that AI models are statistically worst at.

## Before Writing Code

1. **Read surrounding code first.** Never generate Rust in isolation. Understand the module structure, existing types, trait implementations, and error handling patterns already in use.
2. **Check Cargo.toml** for the Rust edition, dependency versions, and feature flags. Generate code against the actual versions, not whatever version is in training data.
3. **Identify the module path.** Know exactly where `mod` declarations live, what's `pub`, `pub(crate)`, or private. Get imports right the first time.

## Ownership & Lifetimes (AI failure rate: very high)

- **Don't `.clone()` to escape the borrow checker.** If you're about to add `.clone()`, stop and think about whether borrowing or restructuring ownership is correct.
- **Don't guess lifetime annotations.** If a function needs lifetimes, trace the actual data flow: where does the borrowed data come from, where does it go, what outlives what. Draw the dependency, then annotate.
- **Prefer owned types in struct fields** unless there's a clear performance reason for borrowing. `String` over `&str`, `Vec<T>` over `&[T]` for struct fields.
- **When lifetime errors occur**, don't blindly add `'static` or `'a` everywhere. Understand why the compiler is complaining — it's usually telling you about a real design issue.

## Async Rust (AI failure rate: high for non-trivial cases)

### Mental model
An `async fn` returns an anonymous `Future` — a state machine holding everything alive across `.await` points. Whether it's `Send` depends on what it holds.

### Rules while writing async code
- **No blocking in async.** Never use `std::fs`, `std::net`, or `thread::sleep` in async functions. Use `tokio::fs`, `tokio::net`, `tokio::time::sleep`, or `spawn_blocking`.
- **No `std::sync::Mutex` guards across `.await`.** The task can move threads. Use `tokio::sync::Mutex` or drop the guard before `.await`.
- **Spawned tasks need `Send + 'static`.** Check that everything captured by a `tokio::spawn` closure satisfies this. `Rc` is `!Send` — use `Arc`.
- **Drop non-Send data before `.await`** if you can't change the type.

### Async fn in traits
```rust
// Native (Rust 1.75+) — NOT dyn-safe
trait MyTrait {
    async fn do_thing(&self) -> Result<()>;
}

// For dynamic dispatch: box the future
trait MyTrait {
    fn do_thing(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
}

// Or use async-trait crate for convenience
#[async_trait]
trait MyTrait {
    async fn do_thing(&self) -> Result<()>;
}
```
**Prefer generics over dyn** when you don't need dynamic dispatch (most cases).

### Pin
- `Box::pin(future)` for heap pinning (storing futures in structs)
- `tokio::pin!(future)` for stack pinning (select!, manual polling)
- Most types are `Unpin` and can be moved freely. Async fn futures are `!Unpin`.

### Streams
```rust
use tokio_stream::StreamExt;
while let Some(item) = stream.next().await {
    process(item).await;
}
```

### Runtime
- Don't mix runtimes (tokio vs async-std). Pick one.
- Use `#[tokio::main]` for binaries, `#[tokio::test]` for async tests.

## Error Handling

- Use `?` operator and proper error propagation. Never `unwrap()` in library code or anything that isn't a quick script/test.
- Use `thiserror` for library error types, `anyhow` for application-level errors (check which the project already uses).
- Match the project's existing error handling pattern. If they use custom error enums, follow that convention.

## Idiomatic Patterns

- Use iterators over manual loops: `.iter().map().filter().collect()` not `for i in 0..len`.
- Use `enum` with variants over stringly-typed APIs or boolean flags.
- Use `impl Into<T>` / `AsRef<T>` for flexible function parameters.
- Prefer `Option` and `Result` over sentinel values or panics.
- Use `Default::default()` and builder patterns where appropriate.
- Use `#[derive(...)]` for standard traits — don't hand-implement `Debug`, `Clone`, `PartialEq` unless there's a reason.

## Module Integration (AI failure rate: catastrophic at 94.8%)

- **Verify every `use` statement.** Check that the path actually exists in the crate/dependency. Don't guess.
- **Check re-exports.** Many crates re-export items at different paths than their internal structure. Read `lib.rs` or the crate docs.
- **Check feature gates.** If a type/function requires a cargo feature flag, mention it.
- **When adding a new module**, remember to add the `mod` declaration in the parent module.

## Type System

- Be precise with generics. Don't over-constrain with unnecessary trait bounds. Don't under-constrain and get confusing errors downstream.
- When implementing traits, read the trait definition first. Check required methods, provided methods, and associated types.
- Use type aliases to simplify complex types: `type Result<T> = std::result::Result<T, MyError>;`

## Testing

- Tests go in a `#[cfg(test)] mod tests` block at the bottom of the file, or in a `tests/` directory for integration tests.
- Use `#[test]`, `#[should_panic]`, and `#[ignore]` attributes correctly.
- Use `assert_eq!`, `assert_ne!`, `assert!(matches!(...))` — not just `assert!`.
- Test the ownership edge cases: does the function work with borrowed data? Owned data? Both?

## After Writing Code

1. Mentally trace through the borrow checker: are there any overlapping mutable borrows? Are lifetimes correct?
2. Check every `use` statement resolves to a real path.
3. Ensure error types are compatible (no implicit conversions that don't exist).
4. For async: no blocking calls, no mutex guards across `.await`, `Send` bounds satisfied on spawned tasks.
5. Run `cargo check` and fix errors before presenting the code.
