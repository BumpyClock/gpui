# GPUI

A standalone extraction of [Zed's](https://github.com/zed-industries/zed) GPU-accelerated UI framework.

GPUI is a hybrid immediate and retained mode, GPU accelerated, UI framework for Rust.

## Getting Started

Add GPUI as a git dependency:

```toml
[dependencies]
gpui = { package = "bumpyclock-gpui", git = "https://github.com/BumpyClock/gpui", rev = "<full-40-character-commit-sha>" }
```

Replace the placeholder with an immutable commit containing the renamed `bumpyclock-gpui`
package; do not depend on `main`.

See `crates/gpui/examples/` for example applications.

## Building

```sh
cargo check -p bumpyclock-gpui
cargo test -p bumpyclock-gpui --features test-support
cargo build --example hello_world
```

## Upstream Sync

GPUI is a selective semantic fork of [Zed](https://github.com/zed-industries/zed), not a
contiguous merge. Exact provenance and maintained divergence clusters live in
[`fork.toml`](fork.toml), the machine-readable source of truth. See [UPSTREAM.md](UPSTREAM.md) for
sync policy, invariants, exclusions, verification evidence, and the next-sync procedure.

## Platform validation

The repository currently proves source builds and backend compile checks in CI. Native runtime
behavior remains session- and hardware-dependent: macOS and Windows platform tests run on their
native runners; Linux X11 and Wayland jobs compile and run backend tests without claiming a live
display session; WebAssembly is compile-only. See CI and [UPSTREAM.md](UPSTREAM.md) before treating
a platform as production-supported.

| Target | CI evidence | Maturity |
| --- | --- | --- |
| macOS | Native backend tests on macOS runner; no full app/window presentation claim | preview |
| Windows | Native backend tests plus shader compile on Windows runner; no full app/window presentation claim | preview |
| Linux X11 | Backend compile and tests; no live X11 display/presentation | experimental |
| Linux Wayland | Backend compile and tests; no live Wayland compositor/presentation | experimental |
| WebAssembly | Nightly compile-only checks | compile-only |

## License

The extracted GPUI crates generally declare Apache-2.0; `zlog`, `ztracing`, and
`ztracing_macro` declare GPL-3.0-or-later. See crate manifests and [LICENSE-AUDIT.md](LICENSE-AUDIT.md)
for the unresolved combined-publication review.
