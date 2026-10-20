#!/usr/bin/env bash
# Remote-friendly updater: fetches the canonical installer and invokes update mode.
set -Eeuo pipefail
REPO="${YTGET_REPO:-yashkgupta928-blip/ytget-rs}"
if [[ "$REPO" == *YOUR_GITHUB_USERNAME* || "$REPO" != */* ]]; then
  echo "Set YTGET_REPO=owner/repository if you are using a fork." >&2
  exit 2
fi
curl --fail --location --silent --show-error --retry 3 "https://raw.githubusercontent.com/${REPO}/main/install.sh" | bash -s -- update
