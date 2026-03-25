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

Last synced with [zed-industries/zed](https://github.com/zed-industries/zed) at commit [`dbd95ea742`](https://github.com/zed-industries/zed/commit/dbd95ea742) (2025-03-25).

## License

GPL-3.0
