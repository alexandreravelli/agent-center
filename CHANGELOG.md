# Changelog

## 0.2.0 - 2026-10-10

- Agent Center has its own APT repository, signed and added by the package, so `apt upgrade` installs new versions. Removing the package removes it.
- *Open* button on installed apps: desktop apps start, and command-line agents open in a terminal.
- Update notifications: `agent-center notify`, started at login, checks once a day and shows a desktop notification for new versions. Clicking it opens the Updates page.
- `agent-center --updates` opens the window on the Updates page, or switches the open window to it.
- New app: Cursor CLI (38 apps).
- The release files include `agent-center_amd64.deb`, a copy of the package under a name that does not change.

## 0.1.0 - 2026-10-10

First release.

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
