#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
unit="${repo_root}/systemd/logilightshow-gaming.service"
binary="${repo_root}/target/release/logilightshow"

if [[ ! -x "${binary}" ]]; then
    echo "Release binary not found at ${binary}; run cargo build --release first." >&2
    exit 1
fi

systemd-analyze --user verify "${unit}"
systemctl --user enable "${unit}"
systemctl --user daemon-reload

echo "Gaming Mode service enabled. It will start with gamescope-session-plus@steam.service."
