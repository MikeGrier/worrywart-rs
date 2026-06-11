---
applyTo: "**/*.rs"
---

# Rust-specific instructions

Ensure that the code is clear and uses good standard naming practices.

Apply comments that are well written and add context which is not obvious. Comments should
describe context or information which is not easily derived from the code such as design
constraints that are not immediately visible or obvious, shortcuts that are made which
future maintainers should know about, or innovations that are not common practice.

## Tool mapping

| Operation | MCP tool |
|---|---|
| Check for compile errors (fast, no binary) | `mcp_cargo_cargo_check` |
| Full compile | `mcp_cargo_cargo_build` |
| Run tests | `mcp_cargo_cargo_test` |
| Format source code | `mcp_cargo_cargo_fmt` |
| Check formatting without modifying files | `mcp_cargo_cargo_fmt_check` |
| Lint (Clippy) | `mcp_cargo_cargo_clippy` |
| Build documentation | `mcp_cargo_cargo_doc` |
| Show dependency tree | `mcp_cargo_cargo_tree` |
| Get workspace/package metadata | `mcp_cargo_cargo_metadata` |
| Remove build artifacts | `mcp_cargo_cargo_clean` |

## Key parameters

- **`working_dir`**: Set to the workspace root unless you intentionally want to
  scope to a sub-directory.
- **`package`**: Use to scope an operation to a single crate (e.g. `hornero`).
- **`all_targets`**: Set to `true` when checking or building tests, examples, and
  benches in addition to the library target.

## Pre-commit gate — mandatory for any commit that includes `.rs` files

**Before running `git commit`, you MUST perform all of the following steps in
order when any `.rs` file is among the staged changes. Do not skip or reorder.**

1. **Format** — run `cargo fmt` (`mcp_cargo-mcp_cargo_fmt`). Stage any
   reformatted files before committing. Any file that `cargo fmt` modifies
   **must** be re-staged; do not proceed until formatting produces no further
   changes.
2. **Compile** — run `cargo check --all-targets`. Every error and warning must
   be fixed before continuing. Do not proceed to the next step while any
   diagnostic remains.
3. **Test** — run `cargo test` for the affected package(s). All tests must pass
   (or pre-existing failures must be recorded in `UNRESOLVED-TEST-FAILURES.md`).
4. **Lint** — run `cargo clippy --all-targets`. Every Clippy warning must be
   fixed before continuing. Do not suppress warnings with `#[allow(...)]`
   unless there is a documented reason. Do not proceed to `git commit` while
   any Clippy diagnostic remains.

None of these steps may be deferred to a later commit. A commit that skips
formatting or introduces Clippy warnings is a defect in the commit itself.
If any step reports issues, stop, fix them, and re-run that step before
moving forward.

# Rust milestone builds

For Rust workspaces, the end-of-milestone "clean build" means `cargo check` (not
`cargo build`). This catches all compile errors and warnings without spending time on
codegen and linking. Use `cargo check --all-targets` to include tests, examples, and
benches. The same applies to the "debug + release" requirement: run both
`cargo check --all-targets` and `cargo check --all-targets --release`.

**Scope the build to `default-members`, never `--workspace`.** The exact command for a
milestone-boundary clean build is:

```
cargo check --all-targets
cargo check --all-targets --release
```

with **no** `--workspace` flag and **no** explicit `-p <pkg>` enumeration. Plain
`cargo check` honors the `[workspace] default-members` list in the root `Cargo.toml`.
Some workspace members are intentionally listed in `members` but **excluded** from
`default-members` because they are slow to build (for example, `cql-parser`, whose
build script runs LALRPOP over a large grammar and can take a very long time). Passing
`--workspace` overrides `default-members` and forces every member to build, pulling in
those deliberately-excluded slow crates and making the build appear to hang. **Do not
pass `--workspace` (or `--all` ) for milestone builds; do not enumerate every package
with repeated `-p`.** `--all-targets` is required; `--workspace` is forbidden.

If you genuinely need to build an excluded crate (e.g. you changed it), build that
crate explicitly with `-p <crate>` rather than switching the whole command to
`--workspace`.

# What does it mean for a checklist item to be done

For an item to be considered complete, the unit and integration tests associated with
any crates associated with it (so this could be a large set - when in doubt, check the
repository!) have to be built, clean, for both debug and release.

The builds have to complete successfully without warnings. Fix any build errors or
warnings.

The unit tests and integration tests must pass. Perform at least first level analysis
of the tests which fail.

In the same directory as the nearest CHECKLIST.md (stopping at the crate directory;
create one there if none exists), create or update an UNRESOLVED-TEST-FAILURES.md
file listing each failing test with its failure message or summary. Add an item to
the CHECKLIST.md referencing the UNRESOLVED-TEST-FAILURES.md file so it is tracked
as outstanding work.

Test failures do not have to be fully resolved in a single session. By recording
them in UNRESOLVED-TEST-FAILURES.md, incremental progress can be made over time
across multiple sessions. Remove entries as each failure is resolved. Delete the
file and mark the checklist item complete once all failures are addressed.

# Test structure conventions

## naming and warnings

Name tests and test code to follow logical patterns. If these patterns would cause
release build warnings in the test code, add the appropriate `#[allow(...)]` attribute
(e.g. `non_snake_case`, `unused_variables`, `unused_imports`, `dead_code`) so that
the release builds can complete without warnings.

## Unit tests

Unit tests live in a **physical `tests.rs` file**, separate from the production
source code. Never place test code inline inside a source file.

### File layout

For a module declared in `src/lib.rs` (or any file that is itself the module):
- Declare `#[cfg(test)] mod tests;` at the bottom of the source file.
- Place all test code in `src/tests.rs`.

For a sub-module declared via a file `src/foo.rs`:
- Declare `#[cfg(test)] mod tests;` at the bottom of `src/foo.rs`.
- Place all test code in `src/foo/tests.rs`.

The `#[cfg(test)]` attribute belongs on the `mod tests;` **declaration** in the
parent file. The `tests.rs` file itself does **not** need `#[cfg(test)]` at the
top.

### Example

`src/lib.rs`:
```rust
// ... production code ...

#[cfg(test)]
mod tests;
```

`src/tests.rs`:
```rust
// Copyright (c) Michael Grier. All rights reserved.
use super::*;

#[test]
fn my_test() { ... }
```

Never place `#[test]` functions outside a physical `tests.rs` (or
`<module>/tests.rs`) file for unit tests. Integration tests in the `tests/`
directory are exempt from this restriction. Never name the submodule anything
other than `tests` for unit test purposes.

## Integration tests

Integration tests live in the crate's `tests/` directory as separate `.rs` files.
Each file is compiled as its own crate and has full access to the crate's public
API. These follow separate naming and fixture conventions documented per-crate.

# Workspace

The repository builds as a single workspace. The workspace manifest, Cargo.toml,
is at the root of the repository.

