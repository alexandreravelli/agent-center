# Changelog

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
