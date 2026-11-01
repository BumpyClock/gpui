# CI runners

Quality and tests, both Linux backend entries, and WebAssembly run on Boat. The existing Linux dependency action, Rust 1.95.0 pin, nightly WASM command, test inventory, required names and release-only duplicate-ID test remain intact. Native Windows/macOS jobs and shader/Metal checks remain on their original runners.

This is an approved runner-source migration, not a performance optimization or successful hosted-run claim. No sharding, cache redesign, publication or upstream sync is included. Native hardware experiments still require separate approval.

BumpyClock owner-only public Boat execution is approved. Outside contributors and bots remain blocked by policy. Do not replace native platform acceptance with Linux results.

## Optional local pre-push checks

Prepare Rust 1.95.0, rustfmt and the existing lockfile dependencies explicitly before installing the hook. Use `cargo fetch --locked` and a successful platform-correct workspace check during setup. The hook never installs a toolchain or fetches dependencies.

Run `sh scripts/setup-hooks.sh` to opt in. The idempotent installer refuses unrelated configured or active hooks. It sets repository-local `core.hooksPath=.githooks`, including shared linked-worktree configuration. No user checkout is auto-installed.

The executable hook checks `cargo fmt --all -- --check` and the complete host workspace with `cargo check --locked --offline --workspace --all-targets`, using `rustup run 1.95.0`. No filenames are passed to Cargo. Every non-deleted pushed ref must resolve to checked-out HEAD and the checkout must be clean, including untracked files. Different commits must be checked out and validated separately; no automatic stash. New branches, unknown bases, renames, empty input and multiple same-commit refs receive full checks. Deletion-only pushes skip source validation. Compiler and formatting failures propagate.

`python3 tooling/tests/test_hooks.py` tests executable hooks and installer behavior with disposable Git fixtures and a stub Rust toolchain. CI runs these fixtures in the existing quality job. They do not prove native GPUI behavior. Native Windows/macOS toolchain and feature acceptance, Linux backend tests, fork validation, release-only tests and WASM remain in full CI. Git Bash is required for the POSIX hook on Windows. No host latency guarantee is made.
