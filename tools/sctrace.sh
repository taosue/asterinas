#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

# Search all SCML files
scml_files=$(find /root/asterinas -type f -name "*.scml" | tr '\n' ' ')

# Run sctrace with all arguments passed to this script
cargo run -q --manifest-path "/root/asterinas/tools/sctrace/Cargo.toml" -- $scml_files "$@"
