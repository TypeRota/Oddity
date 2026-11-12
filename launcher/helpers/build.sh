#!/bin/sh
set -eu
cd "$(dirname "$0")"
# The probes are diagnostic tools and intentionally keep a console.
for src in nte-pipe-probe.c nte-env-probe.c nte-window-probe.c; do
    exe=$(basename "$src" .c).exe
    echo "building $exe"
    winegcc -b x86_64-windows -mconsole -O2 -o "$exe" "$src"
    file "$exe"
done
