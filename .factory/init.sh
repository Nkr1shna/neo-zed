#!/usr/bin/env bash

set -euo pipefail

mkdir -p .factory/library .factory/validation
cargo metadata --format-version 1 --no-deps >/dev/null
