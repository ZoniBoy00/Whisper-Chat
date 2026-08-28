# Contributing to Whisper

Thank you for your interest in Whisper! This project is open source and we
welcome contributions of all kinds — bug reports, feature requests, code,
documentation, testing and security research.

## Getting Started

1. **Fork** the repository and clone your fork.
2. Install the [Rust toolchain](https://rustup.rs/) (stable, edition 2021).
3. For the desktop client, install [Node.js](https://nodejs.org/) and the
   [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/).
4. Run the test suite to verify your setup:

```sh
cargo test --workspace          # Linux/macOS (full workspace)
cargo test -p e2ee-core -p whisper-relay   # Windows without MSVC Build Tools
```

## How to Contribute

### Bug Reports & Feature Requests

Open a [GitHub Issue](../../issues) with a clear description. For bugs, include
steps to reproduce, expected vs actual behaviour and your environment (OS, Rust
version, Tauri version).

### Code Contributions

1. **Discuss first** — open an issue or comment on an existing one before
   starting significant work. We want to avoid wasted effort.
2. **Branch** — create a feature branch from `main`.
3. **Test-driven** — write tests before or alongside your code. Crypto changes
   *require* tests; no tests, no merge.
4. **Keep it focused** — one logical change per PR. Avoid unrelated refactors.
5. **Code comments in English** — always. Documentation language is separate.
6. **No hand-rolled cryptography** — all crypto comes from
   [vodozemac](https://github.com/matrix-org/vodozemac). Own primitives or
   protocol modifications are not accepted.
7. **Run checks before submitting:**

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
```

8. **Open a Pull Request** with a clear title and description referencing the
   related issue.

### What We Need Help With

Check the [Roadmap](docs/ROADMAP.md) and issues labelled `good first issue` or
`help wanted`. Current priorities:

- Relay deployment testing on real infrastructure
- Mobile (Flutter) Megolm key sharing
- Constant-memory media encryption
- Voice/video calls
- External security audit preparation
- Post-quantum cryptography (X25519Kyber768)

## Code Style

- **Rust**: follow standard `rustfmt` + `clippy` with `-D warnings`.
- **TypeScript/React**: Prettier + ESLint (config in `desktop/`).
- **Commit messages**: conventional commits preferred (`feat:`, `fix:`, `docs:`,
  `test:`, `refactor:`).
- **Never commit secrets**: `.env`, `server/data/`, identity files and local
  test artifacts are gitignored. Double-check before pushing.

## Security

See [SECURITY.md](SECURITY.md) for vulnerability reporting. **Do not open
public issues for security vulnerabilities.**

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE). Cryptography is provided by
[vodozemac](https://github.com/matrix-org/vodozemac) (Apache-2.0).

## Questions?

Open a [Discussion](../../discussions) or reach out via GitHub Issues. We're
happy to help! 🔒

</content>