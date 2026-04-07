#!/usr/bin/env bash

set -euo pipefail

mkdir -p .factory/library .factory/validation .factory/research
cargo metadata --format-version 1 --no-deps >/dev/null
