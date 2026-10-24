#!/bin/sh
# Re-records expected outputs for tests/cases/*.php with the reference PHP.
# Paths are stripped (see strip.php) so the .out files do not depend on the
# checkout location.
set -e
cd "$(dirname "$0")/cases"
dir="$(pwd -P)"
[ $# -eq 0 ] && set -- *.php
for f in "$@"; do
    php -n "$f" 2>/dev/null | php -n ../strip.php "$dir/" > "${f%.php}.out" || true
done
