<div align="center">
  <h1>PaladinsCat Discord Bot</h1>
  <p>A high-performance Discord interface for <a href="https://paladinscat.com/">PaladinsCat</a>.</p>

  [![CI](https://github.com/PaladinsCat/PaladinsCat-discord-bot/actions/workflows/ci.yml/badge.svg)](https://github.com/PaladinsCat/PaladinsCat-discord-bot/actions/workflows/ci.yml)
  [![CodeQL](https://github.com/PaladinsCat/PaladinsCat-discord-bot/actions/workflows/codeql.yml/badge.svg)](https://github.com/PaladinsCat/PaladinsCat-discord-bot/actions/workflows/codeql.yml)
</div>

Built with Rust, Tokio, and Twilight, the bot exposes PaladinsCat searches and match intelligence through Discord commands, autocomplete, rich embeds, cached API access, and queued Chromium-based image rendering.

## Project map

| Area | Purpose |
| --- | --- |
| `src/commands.rs` | Slash-command routing and responses |
| `src/api.rs` | PaladinsCat API integration |
| `src/autocomplete.rs` | Interactive command discovery |
| `src/embeds.rs` | Discord presentation builders |
| `src/image/` | Render queue, templates, assets, and browser rendering |
| `src/cache.rs` | Asynchronous response and render caching |

## Development

```text
cargo build
cargo test
cargo clippy -- -D warnings
```

Run `cargo fmt --all -- --check` before submitting a pull request. Runtime credentials are intentionally external and must never be committed. See [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md).

Licensed under the [MIT License](LICENSE).
