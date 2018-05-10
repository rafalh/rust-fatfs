#!/bin/sh
set -e
cargo build --no-default-features --features core_io
cargo build --no-default-features --features core_io,alloc,core_io/collections
