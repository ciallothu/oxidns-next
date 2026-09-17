# Contributing to OxiDNS

Thank you for helping improve OxiDNS. The full contribution, support, testing,
plugin-sync, and documentation workflow is available in the public manual:

- English: <https://oxidns.org/en/contributing>
- 中文: <https://oxidns.org/contributing>

Before opening a public report, remove passwords, tokens, private keys, private
DNS names, client addresses, and query history. Report suspected vulnerabilities
privately according to [SECURITY.md](SECURITY.md).

For a normal Rust change, start with:

```bash
cargo check
cargo test
just check
```

The repository uses Rust 2024 edition and nightly rustfmt. Plugin changes must
keep Rust registration, feature wiring, tests, Chinese and English docs, WebUI
definitions, and the canonical configuration synchronized when applicable.
