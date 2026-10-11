#!/bin/bash
# Builds Agent Center's APT repository in DIR, which must end in /download: the given packages,
# plus the newest packages of Dockling and Tileling from their GitHub releases, in one index
# signed with the key gpg holds. Machines that have Agent Center's repository
# (/etc/apt/sources.list.d/agent-center.sources) get all three with `apt upgrade`.
#
#   apt-index.sh DIR [DEB...]
#
# It is a flat repository: its files sit next to each other in a GitHub release, and "download/"
# is the last part of their URL (the Suites value of agent-center.sources).
set -euo pipefail

dir=${1:?usage: apt-index.sh DIR [DEB...]}
shift
[ "$(basename "$dir")" = download ] || { echo "apt-index.sh: $dir must end in /download" >&2; exit 2; }
mkdir -p "$dir"
for deb in "$@"; do
    cp "$deb" "$dir/"
done

# A token raises GitHub's limit of 60 API requests per hour; it is optional.
auth=()
if [ -n "${GH_TOKEN:-}" ]; then
    auth=(-H "Authorization: Bearer $GH_TOKEN")
fi
for repo in alexandreravelli/dockling alexandreravelli/tileling; do
    name=${repo#*/}
    # The versioned package (name_1.2.3-1_amd64.deb), not its copy under a fixed name.
    url=$(curl -fsSL "${auth[@]}" "https://api.github.com/repos/$repo/releases/latest" \
        | grep -o "\"browser_download_url\": *\"[^\"]*/${name}_[0-9][^\"/]*_amd64\.deb\"" \
        | sed 's/.*"\(https:[^"]*\)"/\1/' | head -n 1)
    [ -n "$url" ] || { echo "apt-index.sh: no package in the latest release of $repo" >&2; exit 1; }
    echo "Adding ${url##*/}"
    curl -fsSL -o "$dir/${url##*/}" "$url"
done

cd "$dir/.."
apt-ftparchive packages download > download/Packages
gzip -9kn -f download/Packages
# Origin and Label stay "Agent Center": APT refuses a repository whose Origin changes.
apt-ftparchive -o APT::FTPArchive::Release::Origin="Agent Center" \
    -o APT::FTPArchive::Release::Label="Agent Center" \
    -o APT::FTPArchive::Release::Suite=download release download > Release
mv Release download/Release
gpg --batch --yes --clearsign -o download/InRelease download/Release
gpg --batch --yes --armor --detach-sign -o download/Release.gpg download/Release
