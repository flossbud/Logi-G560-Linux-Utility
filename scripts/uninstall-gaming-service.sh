#!/usr/bin/env bash
set -euo pipefail

systemctl --user disable --now logig560-gaming.service
systemctl --user daemon-reload

echo "Gaming Mode service disabled and stopped."
