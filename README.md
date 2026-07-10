# GPUI

A standalone extraction of [Zed's](https://github.com/zed-industries/zed) GPU-accelerated UI framework.

GPUI is a hybrid immediate and retained mode, GPU accelerated, UI framework for Rust.

## Getting Started

Add GPUI as a git dependency:

```toml
[dependencies]
gpui = { git = "https://github.com/BumpyClock/gpui" }
```

See `crates/gpui/examples/` for example applications.

## Building

```sh
cargo check -p gpui
cargo test -p gpui --features test-support
cargo build --example hello_world
```

## Upstream Sync

GPUI changes have been selectively audited and synchronized through
[zed-industries/zed commit `2c4e44704c`](https://github.com/zed-industries/zed/commit/2c4e44704c37ee87e59ac84e3e17388178b28545)
(2026-07-09). This is a semantic fork sync, not a contiguous merge or a claim that the trees are
byte-identical. See [UPSTREAM.md](UPSTREAM.md) for imported areas, fork invariants, exclusions,
verification evidence, and the next-sync procedure.

## License

GPL-3.0
