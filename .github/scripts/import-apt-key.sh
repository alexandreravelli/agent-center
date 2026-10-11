#!/bin/bash
# Imports the APT signing key from the APT_SIGNING_KEY secret into a new GNUPGHOME, which it
# exports to the next steps (GITHUB_ENV). The key must be the one whose public part is in
# data/agent-center-archive-keyring.gpg, the key APT trusts on every machine.
set -euo pipefail

if [ -z "${APT_SIGNING_KEY:-}" ]; then
    echo "::error::The APT_SIGNING_KEY secret is missing (see \"APT repository\" in README.md)."
    exit 1
fi
case "$APT_SIGNING_KEY" in
    *"BEGIN PGP PRIVATE KEY BLOCK"*) ;;
    *"BEGIN PGP PUBLIC KEY BLOCK"*)
        echo "::error::APT_SIGNING_KEY holds the public key; it needs the private key (gpg --armor --export-secret-keys)."
        exit 1 ;;
    *)
        echo "::error::APT_SIGNING_KEY is not an armored private key (-----BEGIN PGP PRIVATE KEY BLOCK-----)."
        exit 1 ;;
esac
GNUPGHOME="$(mktemp -d)"
export GNUPGHOME
echo "GNUPGHOME=$GNUPGHOME" >> "$GITHUB_ENV"
# Only gpg's own messages are reported: some of its lines quote the bad input.
if ! out=$(printf '%s\n' "$APT_SIGNING_KEY" | gpg --batch --import 2>&1); then
    why=$(printf '%s\n' "$out" | grep -E 'no valid OpenPGP data|failed:|agent' | head -n 2 | tr '\n' ' ')
    echo "::error::gpg cannot import APT_SIGNING_KEY: ${why:-unknown error}"
    exit 1
fi
have=$(gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" {print $10; exit}')
if [ -z "$have" ]; then
    echo "::error::APT_SIGNING_KEY holds no private key."
    exit 1
fi
# The package ships the public key APT trusts. Until the repository has a copy, it comes from the
# secret; once it has one, the secret must match it.
if [ -f data/agent-center-archive-keyring.gpg ]; then
    want=$(gpg --batch --with-colons --show-keys data/agent-center-archive-keyring.gpg | awk -F: '$1 == "fpr" {print $10; exit}')
    if [ "$want" != "$have" ]; then
        echo "::error::APT_SIGNING_KEY ($have) is not the key in data/agent-center-archive-keyring.gpg ($want)."
        exit 1
    fi
else
    gpg --batch --export "$have" > data/agent-center-archive-keyring.gpg
fi
echo "Signing key: $have"
