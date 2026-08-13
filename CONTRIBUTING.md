# Contributing to PaladinsCat Discord Bot

## Development Setup
1. Install Rust (stable): `rustup default stable`
2. Build: `cargo build`
3. Lint: `cargo clippy -- -D warnings`
4. Test: `cargo test`
5. Format: `cargo fmt`

## Branch Naming
- Features: `feat/description`
- Fixes: `fix/description`

## Pull Requests
- Reference an issue number in the PR title
- Ensure CI passes (cargo check, clippy, test, fmt)
