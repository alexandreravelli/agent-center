# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/alexandreravelli/agent-center/compare/v0.2.1...v0.3.0) - 2026-10-11

### Added

- Shell completion for bash, zsh and fish, installed by the package. `agent-center completions <shell>` prints the script, and `list --ids` prints the app ids.
- `--dry-run` for `install`, `update` and `remove`: shows every script that would run, in order, as root or as you, and runs nothing.
- `--json` for `list` and `info`, for scripts and provisioning tools.
- Colored output in a terminal: installed versions, available updates, steps and errors. Off when the output is piped, with `NO_COLOR` or with `TERM=dumb`.
- Suggestions for mistyped apps, commands and options: `install claude` answers *Did you mean claude-code or claude-desktop?*

### Changed

- Unknown options are refused with exit code 2 instead of being ignored (`list --instaled` listed every app), and `info` takes one app.
- Clearer error messages; the catalog, version and script logic is now a documented library inside the package.
- Each release runs every check (tests, Clippy, MSRV, cargo-deny) before the package is built and published.

### Fixed

- A cancelled password dialog or a refused `sudo` says "authentication failed or was cancelled" instead of an exit status.
- Two apps that require each other are queued once instead of over and over.
- Colored installer output no longer leaves stray characters (from `tput sgr0`) in the log.

### Security

- The `GITHUB_TOKEN` used for version checks is passed to curl through its standard input: on the command line, other local users could read it in the process list.
- Scripts that run as root get a fixed `PATH` and `HOME`, whatever environment sudo was configured to keep, so a directory the user can write to never comes first in root's `PATH`.

## [0.2.1](https://github.com/alexandreravelli/agent-center/compare/v0.2.0...v0.2.1) - 2026-10-10

### Changed

- New macOS-style app icon.

## [0.2.0](https://github.com/alexandreravelli/agent-center/compare/v0.1.0...v0.2.0) - 2026-10-10

### Added

- Agent Center has its own APT repository, signed and added by the package, so `apt upgrade` installs new versions. Removing the package removes it.
- *Open* button on installed apps: desktop apps start, and command-line agents open in a terminal.
- Update notifications: `agent-center notify`, started at login, checks once a day and shows a desktop notification for new versions. Clicking it opens the Updates page.
- `agent-center --updates` opens the window on the Updates page, or switches the open window to it.
- New app: Cursor CLI (38 apps).
- The release files include `agent-center_amd64.deb`, a copy of the package under a name that does not change.

## [0.1.0](https://github.com/alexandreravelli/agent-center/releases/tag/v0.1.0) - 2026-10-10

### Added

- App center for Debian 13 with a GTK 3 window and a command-line interface (`agent-center list`, `info`, `install`, `update`, `remove`).
- Catalog of 37 apps in five categories: AI Agents, AI Apps, Browsers, IDEs & Editors, Developer Tools.
- Each app installs with the method its vendor documents: signed APT repositories, npm packages, `.deb` releases, or install scripts.
- Removal also deletes the APT repository and signing key an app added; optionally deletes its settings and data in the home directory.
- Update checks against npm, PyPI, GitHub releases and the vendors' APT indexes, with *Update All*.
- Required apps are installed first (Node.js for npm-based agents, uv for Aider, Hermes Agent for Hermes Desktop).
- System-wide steps run through `sudo` with a password window, or `pkexec`; root only runs scripts from root-owned catalog files.
- Claude Code and ChatGPT are featured at the top of All Apps (`featured = true` in the catalog).
- Each app shows its vendor's official logo (SVG), with the icon theme and a lettered badge as fallbacks.
- The catalog is plain TOML: add apps in `/etc/agent-center/catalog.d` or `~/.config/agent-center/catalog.d`.
