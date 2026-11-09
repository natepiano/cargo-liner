#!/bin/sh
# cargo-tile-capture-shim
# cargo-tile-shim-version: 4
# The native runner owns setup, publication, and cleanup.
capture_parent=/tmp/cargo-tile
capture_entry=@CAPTURE_SHIM_ARGUMENT@
if [ ! -e /dev/fd/1 ] || [ ! -e /dev/fd/2 ]; then
    capture_entry=@CAPTURE_SHIM_PASSTHROUGH_ARGUMENT@
fi
exec @CARGO_TILE_EXECUTABLE@ "$capture_entry" "$0" "$capture_parent" "$@"
