#!/bin/sh
# Re-records expected outputs for tests/cases/*.php with the reference PHP.
# Paths are stripped so the .out files do not depend on the checkout location.
set -e
cd "$(dirname "$0")/cases"
dir="$(pwd -P)"
for f in ${1:-*.php}; do
    php -n "$f" 2>/dev/null | sed "s#$dir/##g" > "${f%.php}.out" || true
done
