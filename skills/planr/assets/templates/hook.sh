#!/bin/sh
entry="/*ENTRY*/"
[ -f "$entry" ] || exit 0
node "$entry" check
