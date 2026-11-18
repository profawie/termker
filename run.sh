#!/usr/bin/env bash
# Build and run termker on macOS or Linux.
#
#   ./run.sh            debug build (compiles faster)
#   ./run.sh --release  optimized build
#
# On Windows, use run.ps1 instead.

set -euo pipefail
cd "$(dirname "$0")"

case "${1:-}" in
"") exec cargo run --locked ;;
--release) exec cargo run --locked --release ;;
-h | --help) sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//' ;;
*)
    sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
    ;;
esac
