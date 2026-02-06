---
name: rust-borrow-fixer
description: Diagnose and fix Rust borrow checker and lifetime errors. Paste compiler errors and code — get structured analysis and fixes.
user-invocable: true
---

# Rust Borrow Checker Fixer

Systematically diagnose and resolve borrow checker and lifetime errors. Don't guess — trace the actual ownership and borrowing relationships.

## When Invoked

The user has a Rust compilation error related to borrowing, lifetimes, or ownership. They will provide compiler output and/or code.

## Diagnosis Process

### Step 1: Classify the Error

Identify which category the error falls into:

| Error Pattern | Category | Typical Cause |
|---|---|---|
| `cannot borrow X as mutable because it is also borrowed as immutable` | Aliasing violation | Overlapping `&` and `&mut` to same data |
| `X does not live long enough` | Lifetime too short | Borrowed data dropped before borrow ends |
| `cannot move out of X because it is borrowed` | Move-while-borrowed | Trying to take ownership while borrow exists |
| `lifetime may not live long enough` | Lifetime mismatch | Function signature lifetimes don't match body |
| `missing lifetime specifier` | Elision failure | Compiler can't infer lifetimes, needs annotation |
| `cannot return reference to local variable` | Dangling reference | Returning a borrow to stack data |
| `closure may outlive the current function` | Capture lifetime | Closure captures a borrow that doesn't live long enough |
| `use of moved value` | Use-after-move | Value consumed, then used again |

### Step 2: Trace the Data Flow

For the problematic variable/reference:
1. **Where is it created?** (owned, borrowed, or received as parameter)
2. **Where is it borrowed?** (each `&` and `&mut`)
3. **Where does each borrow end?** (last use of the reference)
4. **Where is it moved or dropped?** (ownership transfer or scope end)
5. **Do any borrows overlap in incompatible ways?**

Draw this out mentally or in comments before attempting a fix.

### Step 3: Choose a Fix Strategy

**Prefer fixes in this order** (least invasive to most):

1. **Reorder statements** — Often the simplest fix. Move the mutable borrow after the last use of the immutable borrow.

2. **Narrow borrow scope** — Introduce a block `{ }` to limit how long a borrow lives.
   ```rust
   // Before: borrows overlap
   let x = &data;
   let y = &mut data; // ERROR

   // After: first borrow ends before second
   { let x = &data; use(x); }
   let y = &mut data; // OK
   ```

3. **Split the struct** — If you're borrowing two fields of the same struct, borrow them individually instead of borrowing `&mut self`.
   ```rust
   // Before: can't borrow self twice
   self.process(self.data); // ERROR

   // After: borrow fields independently
   let data = &self.data;
   Self::process_data(data, &mut self.output);
   ```

4. **Change function signature** — Accept `&self` instead of `&mut self` if mutation isn't needed, or take ownership if the caller doesn't need the value anymore.

5. **Use interior mutability** — `Cell`, `RefCell`, `Mutex` when shared mutation is genuinely needed (not as a borrow-checker escape hatch).

6. **Restructure ownership** — Sometimes the borrow checker is telling you the design is wrong. If data needs to be shared and mutated by multiple owners, rethink who owns what.

**Avoid these "fixes":**
- Adding `.clone()` without understanding why — this hides the real issue
- Adding `'static` lifetime — this almost never solves the actual problem
- Using `unsafe` to bypass the borrow checker — this is always wrong
- Wrapping everything in `Rc<RefCell<T>>` — this defeats Rust's compile-time guarantees

### Step 4: Verify the Fix

After applying the fix:
1. Does it compile? Run `cargo check`.
2. Does it preserve the original intent? (No accidental copies of large data, no changed semantics)
3. Is it the minimal change? Don't restructure the whole module for a one-line fix.
4. Are there other similar patterns in the file that need the same fix?

## Common Patterns and Solutions

### "Temporary value dropped while borrowed"
```rust
// BAD: temporary String dropped, but we borrowed a &str from it
let s: &str = &format!("hello {}", name);

// GOOD: bind the temporary to extend its lifetime
let owned = format!("hello {}", name);
let s: &str = &owned;
```

### "Cannot borrow as mutable more than once"
```rust
// BAD: two mutable borrows
let a = &mut vec[0];
let b = &mut vec[1]; // ERROR even though different indices

// GOOD: use split_at_mut or index once
let (left, right) = vec.split_at_mut(1);
let a = &mut left[0];
let b = &mut right[0];
```

### "Closure may outlive current function"
```rust
// BAD: closure captures &local
let local = String::new();
thread::spawn(|| use_string(&local)); // ERROR

// GOOD: move ownership into closure
let local = String::new();
thread::spawn(move || use_string(&local)); // OK, closure owns it
```

## Output Format

1. **Error classification** — which category and why
2. **Data flow trace** — where the problematic borrow originates and conflicts
3. **Recommended fix** — with code, using the least invasive approach
4. **Why this fix is correct** — not just "it compiles" but why the ownership is sound
