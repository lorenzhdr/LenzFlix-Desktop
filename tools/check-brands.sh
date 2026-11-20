#!/bin/sh
# Prüft Dateien (Argumente oder alle getrackten) gegen tools/brand-denylist.txt.
# Treffer als ganzes Wort, case-insensitive. Exit 1 bei Treffer.
set -eu
root=$(git rev-parse --show-toplevel)
list="$root/tools/brand-denylist.txt"
pat=$(grep -v '^[[:space:]]*#' "$list" | grep -v '^[[:space:]]*$' | tr 'A-Za-z' 'N-ZA-Mn-za-m' | sed 's/[][\.*^$/]/\&/g' | paste -sd'|' -)
[ -n "$pat" ] || exit 0
if [ "$#" -gt 0 ]; then files="$*"; else files=$(git ls-files); fi
status=0
for f in $files; do
  case "$f" in
    tools/brand-denylist.txt|.githooks/*|*.png|*.jpg|*.ico|*.lock|Cargo.lock) continue ;;
  esac
  [ -f "$f" ] || continue
  if grep -I -n -i -E -w -e "($pat)" "$f" >/dev/null 2>&1; then
    echo "Marken-Treffer in $f:" >&2
    grep -I -n -i -E -w -e "($pat)" "$f" | head -5 >&2
    status=1
  fi
done
exit $status
