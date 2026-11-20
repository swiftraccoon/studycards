# studycards

A local web application for studying user-written flash-card sets. The Rust
crate `studycards` holds all behaviour; a small TypeScript file adds keyboard
shortcuts in the browser.

## Rust project references

Use the local reference pack at `.rust-reference/`. Read its `AGENTS.md`,
`docs/rust-expectations.md`, `docs/documentation.md`, and
`docs/commits-and-releases.md` before changing Rust code or project documentation.
Consult the full local guides through `.rust-reference/references/README.md` for
the relevant details.

Explicit user instructions and this project's applicable instructions take
precedence over the reference defaults. Follow this project's toolchain, MSRV,
feature support, and compatibility promises. Read vendored upstream documents as
reference content; their repository instructions apply to their own projects.
Implement project-owned tools and automation in Rust. Do not add Python code or
a Python runtime dependency to this Rust project.

The references work offline. Agents may fetch upstream changes when useful with
`cargo run --locked --manifest-path .rust-reference/tools/reference-maintenance/Cargo.toml -- sync [source-id ...]`.
The optional maintenance crate has its own workspace; reading the pack requires
no Cargo. Review reference and policy diffs before adopting changes; keep license
notices and `references/manifest.json` with copied sources. Validate snapshots with
`cargo run --locked --manifest-path .rust-reference/tools/reference-maintenance/Cargo.toml -- sync --check`.

## Project decisions

- **Kind and platforms.** An application for one person on their own machine,
  not a published crate (`publish = false`). It is developed on macOS, and CI
  tests it on Linux and macOS. It uses only portable standard-library
  facilities; Windows is untested.
- **License.** `GPL-3.0-or-later`; `LICENSE` holds the text. A dependency must
  have a license that is compatible with it. The reference pack in
  `.rust-reference/` is not part of the program and keeps its own licenses,
  which `.rust-reference/references/README.md` lists.
- **Toolchain.** Edition 2024. Development uses Rust 1.96.0, pinned in
  `rust-toolchain.toml`. The minimum supported Rust version is 1.88, recorded in
  `Cargo.toml`.
- **Formatting.** Default rustfmt settings, with no `rustfmt.toml`: `cargo fmt`
  takes the 2024 style edition from the package edition.
- **Language.** Behaviour lives in Rust, including HTML rendering. TypeScript is
  used only where the browser must run code: `web/src/app.ts`. Do not add
  JavaScript sources or Python.
- **Dependencies.** `serde`, `serde_json`, `toml`, and `fastrand`. The HTTP
  server, HTML rendering, and argument parsing use the standard library. Add a
  crate only when the standard library does not meet the need. The crate has no
  Cargo features.
- **Lints.** `Cargo.toml` denies `clippy::all`, `pedantic`, `nursery`, `cargo`,
  and a list of restriction lints, and forbids unsafe code. Shipped code does
  not panic, index, cast with `as`, or use floating point. A needed exception
  uses `#[expect(..., reason = "...")]` on the smallest item.
- **Panics.** `src/http.rs` wraps the handler in `catch_unwind` as defence in
  depth only: the lints keep panics out of shipped code, and no behaviour may
  depend on one being caught. A lock that a panic has poisoned is not entered;
  `web::App` then answers every request with status 500 until the program is
  restarted.
- **Concurrency.** There is no async runtime. `src/http.rs` serves each
  connection on a thread of its own, at most 64 at a time, and `web::App`
  handles one request at a time behind a lock, so changes to the files never
  interleave. The program has no shutdown procedure: it runs until it is
  interrupted, and `src/store.rs` replaces a file by renaming a complete copy
  over it, so an interrupted write leaves the previous content.
- **Trust.** One user on a machine they trust: the server has no login, and
  `Request::check_origin` in `src/http.rs` keeps the pages of other sites out.
  The files of the data directory and the files given to `check` are untrusted
  input. Keep the time that such a file takes in proportion to its size, keep
  messages about it short, show its text only escaped, and never write through
  a path that already exists.
- **Tests.** Every `assert!` carries a message. Tests are deterministic: random
  sources are seeded, and network tests use loopback sockets on free ports. A
  test that needs files makes them in a `Scratch` directory (`src/scratch.rs`).
- **Documentation.** Documentation and comments state what this program does,
  directly and without its development history, as
  `.rust-reference/docs/documentation.md` requires.
- **Privacy.** No tracked file and no commit message holds an absolute path
  of a developer's machine, an account name, a host name, or another personal
  detail. A file of the repository is named by its path from the repository
  root.
- **Compatibility.** The file formats in `data/` are the compatibility surface:
  study sets, documented in `src/deck.rs`, and progress files, documented in
  `src/study.rs`. The library API is internal to the application.
- **Versions.** The version in `Cargo.toml` follows Semantic Versioning 2.0.0
  with the pre-1.0 policy of the document named under Commits. Compatibility is
  judged by the surface above: a change after which a valid set or progress
  file can no longer be read is breaking. The application is not published; it
  is built and run from a checkout.
- **Card selection.** `docs/picker.md` specifies the scores and the rules for
  choosing a batch; `src/mastery.rs` and `src/picker.rs` implement them.
  `tests/observed_batches.rs` replays a recorded study history that the rules
  must allow. Keep it passing, and keep `docs/picker.md` in step, when changing
  them. A history holds one answer for each time its card was asked. What the
  learner chooses after a wrong answer is a retry (`Progress::retry` in
  `src/study.rs`), which changes neither a history nor a batch.

## Layout

`src/lib.rs`, `src/deck.rs`, and `src/web.rs` hold documentation, `mod`
declarations, and re-exports only; code lives in the other files.

| Path | Content |
| --- | --- |
| `src/deck.rs`, `src/deck/` | Study sets and their TOML format |
| `src/mastery.rs` | Answer histories, level, and aptitude |
| `src/picker.rs` | Choosing the cards of a batch |
| `src/study.rs` | Study sessions, progress, and its JSON format |
| `src/store.rs` | Sets and progress as files |
| `src/http.rs` | Loopback HTTP server |
| `src/web.rs`, `src/web/` | Routes, actions, and pages |
| `src/cli.rs`, `src/main.rs` | Command line |
| `src/scratch.rs` | Scratch directories for the unit tests |
| `tests/` | Integration tests and their fixtures |
| `docs/picker.md` | The scores and the rules for choosing cards |
| `assets/` | Style sheet, icon, and `app.js` compiled from `web/src/app.ts` |
| `web/` | TypeScript source and its tooling |
| `data/` | Study sets (`sets/`) and progress (`progress/`, untracked) |
| `.rust-reference/` | The reference pack; do not edit it by hand |

## Checks

Run all of these before delivering a change:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
cargo +1.88 test --locked
```

After changing `web/src/app.ts`, also run the following and include the
regenerated `assets/app.js` in the change:

```sh
pnpm --dir web install --frozen-lockfile
pnpm --dir web run check
pnpm --dir web run build
```

## Commits

Use Conventional Commits 1.0.0 as described in
`.rust-reference/docs/commits-and-releases.md`.
