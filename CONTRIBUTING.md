# Contributing to Agent Center

Thanks for helping! Bug reports, new apps for the catalog and code changes are all welcome.

## Reporting a problem

Open an [issue](https://github.com/alexandreravelli/agent-center/issues) with the output of
`agent-center --version`, your Debian version, and the log shown by *Show Log* (or the terminal
output of the `agent-center` command). For a security problem, follow [SECURITY.md](SECURITY.md)
instead.

## Setting up

Agent Center needs Rust 1.92 or newer and the GTK 3 headers:

```sh
sudo apt install -y build-essential curl git libgtk-3-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
git clone https://github.com/alexandreravelli/agent-center
cd agent-center
```

The code is split in two:

- the library (`src/lib.rs`): the catalog, the version checks and the script runner;
- the program (`src/main.rs`): the command line, the GTK window (`gui.rs`) and the update
  notifier (`notifier.rs`).

To try a change without installing anything, point Agent Center at the catalog of your checkout:

```sh
AGENT_CENTER_CATALOG=$PWD/catalog AGENT_CENTER_LOGOS=$PWD/logos cargo run
```

## Before you open a pull request

CI runs these checks; run them first:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps
cargo deny check            # cargo install cargo-deny
```

`cargo test --no-default-features` and `cargo clippy --no-default-features` check the command line
alone, without GTK.

- Keep pull requests small, with one topic each, and add tests for new behaviour.
- Document every public item of the library; examples in doc comments are run as tests.
- Write code, comments, messages and docs in English.

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org/): `type(scope): summary`,
in the imperative, under 72 characters, with a body that explains why when it is not obvious.

| Type | For |
| --- | --- |
| `feat` | a new feature (a new app in the catalog too) |
| `fix` | a bug fix |
| `docs`, `test`, `refactor`, `style`, `perf` | documentation, tests, code changes without new behaviour |
| `build`, `ci`, `chore` | dependencies, packaging, workflows, maintenance |

Add `!` after the type (`feat!:`) for a change that breaks the command line or the catalog format.
The release notes and the next version number are computed from these messages.

## Adding an app to the catalog

1. Add an `[[app]]` table to the right file in [`catalog/`](catalog), following the vendor's
   documented install method. Prefer a signed APT repository, then the vendor's package, then its
   install script. See "Add your own app" in the [README](README.md#add-your-own-app).
2. Add the vendor's logo as `logos/<id>.svg` and its source to
   [`logos/SOURCES.md`](logos/SOURCES.md).
3. Check it: `cargo test` validates every catalog file and script syntax, and
   `AGENT_CENTER_CATALOG=$PWD/catalog cargo run -- info <id>` shows what will run. Install, update
   and remove the app on a Debian 13 machine or VM before you open the pull request.

## Releases

[release-plz](https://release-plz.dev) keeps a release pull request open: it bumps the version
from the commit messages and writes the [changelog](CHANGELOG.md). Merging it makes the Release
workflow run every check, build the package, sign the APT repository and publish the GitHub
release.
