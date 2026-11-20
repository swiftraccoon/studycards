# studycards

Study your own flash-card sets in the browser, on your own machine. Cards are
multiple choice. The application decides what to ask next from how you answered
before: the cards you miss rank lowest and return first, unseen cards are fed in
as you go, and once you have seen everything it keeps returning to your weakest
cards. A progress panel shows how much of a set you have seen and your aptitude,
overall and by topic.

Everything runs locally. The program listens on a loopback address only, needs
no network access, and keeps sets and progress as plain files.

## Requirements

- macOS or Linux. The checks in `.github/workflows/checks.yml` run on both;
  Windows is untested.
- Rust 1.96.0, which rustup selects through `rust-toolchain.toml`. The minimum
  supported version is 1.88.
- A web browser.

## Run

```sh
cargo run --release
```

The program prints its address and runs until you stop it with Ctrl+C:

```text
Study cards is running at http://127.0.0.1:8717/
Sets and progress are kept in data
Press Ctrl+C to stop.
```

Open the address in a browser.

```text
studycards [serve] [--port PORT] [--data DIR]
studycards check FILE...
studycards --help | --version
```

`--port` selects the port; `--port 0` picks any free one. `--data` selects the
directory that holds the sets and progress; it defaults to `data` in the working
directory. `check` reports whether each study set file is valid without starting
the application, and exits with a failure status when a file is not.

## Study

Choose a set and select **Study**. Answer by clicking or by typing the letter of
an answer. The row above the card switches between all cards and a single topic.

| Key | Action |
| --- | --- |
| `A` to `F` | Choose that answer |
| `?` | I don't know |
| `N`, `Enter`, `Space`, `→`, `↓` | Next card |
| `S` | Show the correct answer after a wrong one |
| `X` | Open or close the explanation |

The list of answers shows how a card was answered, and nothing else on the page
does: a correct answer is green, and a wrong one is red and struck through.
After **I don't know**, the page marks the correct answer in the list. After a
wrong answer, the correct one stays hidden. You can choose again among the
answers that are left until you find it, select **Show correct answer**, or go
to the next card without it. Each time a card is asked, only your first answer
counts: what you choose after it changes neither **Seen** and **Aptitude** nor
the cards that follow.

**Seen** is the share of cards you have attempted. **Aptitude** rates your
recent answers: each card scores from its last four answers, the newest counting
most, and cards you have not attempted count as zero.
[How cards are chosen](docs/picker.md) describes the scores and the selection
rules exactly.

## Create sets

Use **Create set** on the home page, then add topics and cards in the editor.
The editor writes one TOML file per set to `data/sets/`. You can also write
these files yourself; the application reads them on every request.

```toml
title = "Solar system"

[[topics]]
name = "Planets"

[[topics.cards]]
question = "Which planet is closest to the Sun?"
answer = "Mercury"
wrong = ["Venus", "Earth", "Mars"]
explanation = "Mercury orbits the Sun at about 0.39 AU."
```

- The file name without `.toml` identifies the set. It consists of 1 to 64
  lowercase letters, digits, `-`, and `_`, and starts with a letter or digit.
- A card has one `answer` and one to five `wrong` answers. `explanation` is
  optional and is offered after the card has been answered.
- A title or topic name has at most 120 characters, an answer 500, and a
  question or explanation 4,000.
- Topics and cards may have an `id`, which follows the rule for file names.
  Progress is recorded per card `id`. A card without one is identified by its
  question text, so rewording that question starts the card afresh. The editor
  writes an `id` for every topic and card.
- The format has no keys besides `id` and the ones shown; any other key is an
  error.
- Saving a set in the editor rewrites its file, which removes comments.

`data/sets/example.toml` is a small set to try. `studycards check FILE...`
validates files you have written by hand.

## Data

| Path | Content |
| --- | --- |
| `data/sets/<set>.toml` | A study set |
| `data/progress/<set>.json` | Answer history and queued cards for that set |

`<set>` stands for the identifier of a set. Deleting a progress file, or
**Reset progress** in the editor, starts a set over. `cargo doc --open` shows
the full description of both formats, under the modules `deck` and `study`.

## Security

studycards is for one person on a machine they trust.

- The application has no login. Any program on the same machine, under any
  user account, can connect to its port and read or change the sets and the
  progress.
- Pages of other websites are kept out. The server answers only requests that
  are addressed to `127.0.0.1` or `localhost` with its port, refuses a request
  that the browser marks as coming from another site unless it opens a page in
  a tab, and accepts a form only from its own pages. A link on another site
  that opens a study page has the effect of opening that page yourself: the
  first batches of that page are chosen and stored.
- A study set from someone else is data. The pages show its text as text, and
  `studycards check` prints control characters as escapes such as `\u{1b}`.
  The program reads a set file whole, whatever its size.
- Sets and progress files get the default permissions of the system. On a
  shared machine, other users may be able to read them.

## Develop

`AGENTS.md` records the project's decisions and layout, and `.rust-reference/`
holds the guidance that the project follows; reading it needs no tools. The
checks below use the pinned toolchain; the last one tests the minimum supported
Rust version and needs `rustup toolchain install 1.88 --profile minimal` once.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
cargo +1.88 test --locked
```

The keyboard shortcuts are written in TypeScript in `web/src/app.ts` and
compiled to `assets/app.js`, which is part of the repository so that building
the program needs only Cargo. Changing the script requires Node.js and pnpm in
the versions that `web/package.json` names under `engines` and
`packageManager`. Run these commands and keep the regenerated `assets/app.js`
with the change:

```sh
pnpm --dir web install --frozen-lockfile
pnpm --dir web run check
pnpm --dir web run build
```

## License

Copyright (C) 2026 studycards contributors

studycards is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version.

studycards is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See the GNU General Public License for more details.

[LICENSE](LICENSE) holds the GNU General Public License, version 3.

`.rust-reference/` is a copy of a reference pack for Rust projects and is not
part of studycards. Its guidance and tooling use the BSD Zero Clause License
in [.rust-reference/LICENSE](.rust-reference/LICENSE), and the documents under
`.rust-reference/references/` keep the licenses that
[their index](.rust-reference/references/README.md#licenses-and-attribution)
lists.
