#!/usr/bin/env sh
set -eu

# Scan tracked text only and print filenames, never matching content.
personal_home_files=$(git grep -IlE '/Users/[A-Za-z0-9._-]+|/home/[A-Za-z0-9._-]+' -- . || true)

if [ -n "$personal_home_files" ]; then
  echo "Tracked files contain machine-specific absolute home paths:" >&2
  echo "$personal_home_files" >&2
  echo "Use \$HOME, a repo-relative path, or an environment variable instead." >&2
  exit 1
fi

echo "No machine-specific absolute home paths in tracked files."
