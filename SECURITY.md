# Security policy

Agent Center runs install scripts as root, so security reports get priority.

## Supported versions

Only the latest release gets fixes. The package's APT repository brings it with
`sudo apt update && sudo apt upgrade`.

## Reporting a vulnerability

Please do not open a public issue. Report it privately through GitHub:
[Security > Report a vulnerability](https://github.com/alexandreravelli/agent-center/security/advisories/new).

Say what an attacker can do, and how to reproduce it. The fix is released as soon as it is ready,
with an advisory that credits you unless you prefer otherwise.

## What is in scope

- Anything that lets a user, a file in a home directory, or another program run a command as root
  through Agent Center, or read or change what root runs.
- The integrity of downloads made by the catalog's scripts, and of Agent Center's APT repository.
- Secrets leaking (for example a `GITHUB_TOKEN` visible to other users).

The install scripts of the vendors themselves are out of scope: report those to the vendor.

## How Agent Center limits root access

- Scripts that run as root only come from root-owned catalog directories
  (`/usr/share/agent-center/catalog`, `/etc/agent-center/catalog.d`); apps defined in a home
  directory always run as the user.
- Root scripts run with a fixed `PATH` and `HOME`, whatever environment sudo was given.
- Data deleted with `--purge` must be inside the home directory; links are deleted, not followed.
