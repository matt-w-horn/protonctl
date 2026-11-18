#!/bin/sh
# The plugin's gates. They read the index, not the working tree: an install
# copies the files at one commit, so they check what the next commit will hold.
# check.sh runs this.
set -eu
cd "$(dirname "$0")/.."

# Every file under 5 MiB, and every file that is not an image or a font under
# 256 KiB. The sizes are those of the staged blobs.
git -c core.quotePath=false ls-files -s | while read -r _ blob _ path; do
    printf '%s %s\n' "$(git cat-file -s "$blob")" "$path"
done | awk '{
    path = substr($0, length($1) + 2)
    media = tolower(path) ~ /\.(png|jpe?g|gif|webp|woff2?|ttf|otf)$/
    limit = media ? 5 * 1024 * 1024 : 256 * 1024
    if ($1 >= limit) {
        printf "plugin-check.sh: %s is %d bytes; the limit is %d\n", path, $1, limit > "/dev/stderr"
        bad = 1
    }
} END { exit bad }'

# The plugin's version equals the crate's, so a release raises both.
# No jq, which macOS lacks: the first "version" key in plugin.json, and the
# version line of Cargo.toml's [package] table.
plugin=$(git show :plugin/.claude-plugin/plugin.json |
    sed -n 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
crate=$(git show :Cargo.toml |
    awk -F '"' '/^\[/ { package = ($0 == "[package]") } package && /^version[[:space:]]*=/ { print $2; exit }')
if [ -z "$plugin" ] || [ "$plugin" != "$crate" ]; then
    echo "plugin-check.sh: plugin/.claude-plugin/plugin.json has version \"$plugin\", but Cargo.toml has \"$crate\"" >&2
    exit 1
fi

# Schema and directory checks on an export of the index, so local untracked
# files or links raise no warning: validate both the marketplace at the root
# and the plugin subfolder under the directory profile.
if command -v claude >/dev/null; then
    tree=$(mktemp -d)
    trap 'rm -rf "$tree"' EXIT
    git checkout-index -a --prefix="$tree/"
    claude plugin validate --strict "$tree"
    claude plugin validate --profile directory --strict "$tree/plugin"
else
    echo "plugin-check.sh: skipped claude plugin validate (needs Claude Code's \`claude\` command)" >&2
fi
echo "plugin-check.sh: all plugin gates passed"
