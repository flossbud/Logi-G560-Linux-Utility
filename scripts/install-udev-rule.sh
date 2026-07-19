#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source_rule="${script_dir}/../contrib/70-logilightshow-g560.rules"
destination_rule="/etc/udev/rules.d/70-logilightshow-g560.rules"

if [[ ! -f "${source_rule}" || "$(basename -- "${source_rule}")" != "70-logilightshow-g560.rules" ]]; then
    echo "Cannot find the expected 70-logilightshow-g560.rules source file." >&2
    exit 1
fi

pkexec install -o root -g root -m 0644 "${source_rule}" "${destination_rule}"
pkexec udevadm control --reload-rules
pkexec udevadm trigger --subsystem-match=usb --attr-match=idVendor=046d --attr-match=idProduct=0a78
