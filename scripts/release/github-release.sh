#!/usr/bin/env bash
# Create the GitHub release for a tag and attach the published crates.
#
# The release stays a draft until its assets are attached, and running this
# again completes a draft or leaves a public release alone, so a failed run can
# be repeated.

set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: github-release.sh <tag> <candidate-directory>" >&2
    exit 2
fi
tag=$1
candidate=$2

# SemVer marks a pre-release with a hyphen after the version core.
prerelease=()
if [[ $tag == *-* ]]; then
    prerelease=(--prerelease)
fi

if state=$(gh release view "$tag" --json isDraft --jq .isDraft 2>/dev/null); then
    if [[ $state == false ]]; then
        echo "github release: $tag is already public"
        exit 0
    fi
else
    gh release create "$tag" --draft --verify-tag --generate-notes \
        --title "$tag" "${prerelease[@]}"
fi

(
    cd "$candidate"
    sha256sum -- *.crate >SHA256SUMS
)
gh release upload "$tag" --clobber "$candidate"/*.crate "$candidate/SHA256SUMS"
gh release edit "$tag" --draft=false
echo "github release: $tag is public"
