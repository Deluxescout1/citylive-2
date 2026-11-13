#!/bin/sh
# Export a clean tree for the public repository (B97). Nothing is published by this script.
#
#   bin/make-public-tree.sh <empty-or-new-dir> [--verify]
#
# The export is an ALLOWLIST of tracked/new-but-not-ignored files: the crates and the build
# and packaging files, plus the docs for users. Everything else stays out: the planning files
# (PLAN, DECISIONS, LOG, AGENTS, CHARTER, NOTES), briefs/, handbacks/, notes/, logs/, reference/,
# vm/, design/, tools/, .claude/, .lanes/, measure/, screenshots, *.sav, and the internal bin/
# scripts. Then it
#   * drops the tools/ members from the workspace Cargo.toml and prunes Cargo.lock,
#   * rewrites the owner's first name in comments to "the owner",
#   * checks the result: no forbidden path, no home paths, e-mail addresses, IP addresses,
#     key or token patterns, no mention of the owner's home town, and detect-secrets (when
#     installed: `pip install detect-secrets`, or set DETECT_SECRETS=/path/to/detect-secrets),
#   * with --verify: builds the exported tree's tests (cargo check --tests, offline, in an 8 GB scope).
# Exit status is 1 when any check finds something.
set -eu
cd "$(dirname "$0")/.."
SRC=$PWD
DEST=${1:-}
VERIFY=0
[ "${2:-}" = "--verify" ] && VERIFY=1
[ -n "$DEST" ] || { echo "usage: $0 <dir> [--verify]" >&2; exit 2; }
mkdir -p "$DEST"
DEST=$(CDPATH= cd -- "$DEST" && pwd)
if [ -n "$(ls -A "$DEST")" ]; then
    echo "refusing to export into a non-empty directory: $DEST" >&2
    exit 2
fi
case "$DEST" in "$SRC"|"$SRC"/*) echo "export outside the source tree" >&2; exit 2 ;; esac

# --------------------------------------------------------------------- the allowlist
INCLUDE="crates assets plasma gnome-extension packaging .github docs/windows-signing.md docs/RELEASING.md docs/screenshots
install.sh uninstall.sh README.md CHANGELOG.md LICENSE Cargo.toml Cargo.lock
bin/build-windows.sh bin/build-installer.sh bin/package-linux.sh bin/make-public-tree.sh"

LIST=$(mktemp)
trap 'rm -f "$LIST"' EXIT INT TERM
# shellcheck disable=SC2086
git ls-files -co --exclude-standard -z -- $INCLUDE > "$LIST"
# Copy (skip anything that looks like a save, a screenshot dump or build output).
tr '\0' '\n' < "$LIST" | while IFS= read -r f; do
    case "$f" in
        *.sav|*.sav.*|*.bak|*.orig|*.log|*.ppm|*/target/*|*/dist/*) continue ;;
    esac
    [ -f "$f" ] || continue
    mkdir -p "$DEST/$(dirname "$f")"
    cp -p "$f" "$DEST/$f"
done

cat > "$DEST/.gitignore" <<'EOF'
target/
target-*/
dist/
.xwin-cache/
*.sav
*.sav.*
EOF

# --------------------------------------------------------------------- workspace
# tools/ is not exported: drop its members, then let cargo prune the lock file (offline).
perl -0pi -e 's/, "tools\/[^"]*"//g; s/"tools\/[^"]*", //g' "$DEST/Cargo.toml"
if grep -q 'tools/' "$DEST/Cargo.toml"; then echo "tools/ members are still in Cargo.toml" >&2; exit 1; fi
( cd "$DEST" && PATH="$HOME/.cargo/bin:$PATH" cargo metadata --offline --format-version 1 >/dev/null )

# --------------------------------------------------------------------- scrub
# The owner's first name appears in code comments; say "the owner" instead (comments only).
find "$DEST" -type f \( -name '*.rs' -o -name '*.cpp' -o -name '*.h' -o -name '*.md' -o -name '*.sh' -o -name '*.toml' \
    -o -name '*.js' -o -name '*.json' -o -name '*.yml' -o -name '*.nsi' -o -name '*.txt' -o -name '*.qml' -o -name '*.cjs' \) \
    -exec perl -pi -e "s/\bNick's\b/the owner's/g; s/\bNick\b/the owner/g" {} +

# --------------------------------------------------------------------- checks
FAIL=0
bad() { echo "FINDING: $*"; FAIL=1; }

echo "== forbidden paths"
for p in PLAN.md DECISIONS.md LOG.md AGENTS.md CHARTER.md NOTES.md FIRST-PROMPT.txt briefs handbacks notes logs reference vm \
         design tools measure .claude .lanes .builder-session target dist; do
    [ -e "$DEST/$p" ] && bad "$p is in the export"
done
for b in tell-builder.sh go-usage.sh deploy-desktop.sh start-iso-planner.sh watch.py screen-check.sh measure.sh env.sh env.fish; do
    [ -e "$DEST/bin/$b" ] && bad "bin/$b is in the export"
done
if [ -n "$(find "$DEST" \( -name '*.sav' -o -name '*.sav.*' -o \( -name '*.png' -a ! -path '*/docs/screenshots/*' \) -o -name '*.jpg' -o -name '*.ppm' -o -name '*.log' \) -print -quit)" ]; then
    find "$DEST" \( -name '*.sav' -o -name '*.sav.*' -o \( -name '*.png' -a ! -path '*/docs/screenshots/*' \) -o -name '*.jpg' -o -name '*.ppm' -o -name '*.log' \) -print | while read -r f; do echo "FINDING: unexpected file $f"; done
    FAIL=1
fi

echo "== text greps"
# Known harmless strings: documentation-style fixtures, the GNOME extension id, a version resource.
ALLOW='noreply\.github\.com|/home/user/|C:/Users/N/|c:\\users\\n\\|"1\.2\.3\.4"|"5\.6\.7\.8"|"9\.9\.9\.9"|"8\.8\.4\.4"|citylive-iso@citylive\.org|version="1\.0\.0\.0"'
G() { # G label pattern [extra grep args]
    label=$1; pat=$2; shift 2
    if grep -rIniE "$@" -e "$pat" "$DEST" --exclude-dir=.git --exclude=Cargo.lock --exclude=make-public-tree.sh 2>/dev/null \
        | grep -vE "$ALLOW" | head -n 20 | grep .; then
        bad "$label"
    fi
}
# Owner-specific words (home town, coordinates, names, machines) live OUTSIDE the repo so this script never
# publishes them itself: one "label<TAB>regex" per line in ~/.config/citylive-publish/private-patterns.
PRIV=${CITYLIVE_PRIVATE_PATTERNS:-$HOME/.config/citylive-publish/private-patterns}
if [ -r "$PRIV" ]; then
    while IFS=$'\t' read -r plabel ppat; do [ -n "$ppat" ] && G "$plabel" "$ppat" < /dev/null; done < "$PRIV"
else
    bad "private pattern file $PRIV missing: refusing to publish without the owner-specific checks"
fi
G "home paths and LAN addresses" '/home/[a-z]{2,}|/Users/[A-Za-z]{2,}|C:\\\\Users\\\\[A-Za-z]{2,}|192\.168\.[0-9]+\.[0-9]+'
G "e-mail addresses" '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' --exclude-dir=gnome-extension
G "IPv4 addresses" '(^|[^0-9.])((25[0-5]|2[0-4][0-9]|1?[0-9]?[0-9])\.){3}(25[0-5]|2[0-4][0-9]|1?[0-9]?[0-9])([^0-9.]|$)' --exclude=installer.nsi
G "key, token or secret patterns" 'AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|gho_[A-Za-z0-9]{30,}|xox[baprs]-[A-Za-z0-9-]{10,}|sk-[A-Za-z0-9]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY|AIza[0-9A-Za-z_-]{35}|api[_-]?key *[:=] *["'"'"'][A-Za-z0-9]{16,}|secret *[:=] *["'"'"'][A-Za-z0-9]{16,}|password *[:=] *["'"'"'][^"'"'"']+'
G "AI attribution" 'co-authored-by|claude-session|generated (with|by) (claude|an? ai|chatgpt|gpt|copilot)|🤖|anthropic|openai|chatgpt'

echo "== detect-secrets"
DS=${DETECT_SECRETS:-$(command -v detect-secrets || true)}
if [ -n "$DS" ]; then
    OUT=$(cd "$DEST" && "$DS" scan --all-files --exclude-files 'Cargo\.lock|\.nsi$' 2>/dev/null)
    COUNT=$(printf '%s' "$OUT" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(sum(len(v) for v in d["results"].values()))')
    echo "detect-secrets: $COUNT finding(s)"
    if [ "$COUNT" != 0 ]; then
        printf '%s' "$OUT" | python3 -c 'import json,sys
d=json.load(sys.stdin)
for f,v in d["results"].items():
    for s in v: print("FINDING:", f, s["line_number"], s["type"])'
        FAIL=1
    fi
else
    echo "detect-secrets not installed (pip install detect-secrets); skipped"
fi

if [ "$VERIFY" = 1 ]; then
    echo "== building the exported tree's tests (cargo check --tests, offline)"
    ( cd "$DEST" && systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 -p CPUQuota=400% \
        env PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SRC/target-export}" \
        cargo check --offline --tests --workspace --exclude present_gnome ) || { echo "FINDING: the exported tree does not build"; FAIL=1; }
fi

COUNT=$(find "$DEST" -type f | wc -l)
echo "== exported $COUNT files to $DEST"
if [ "$FAIL" = 0 ]; then echo "== clean: zero findings"; else echo "== FINDINGS above"; fi
exit "$FAIL"
