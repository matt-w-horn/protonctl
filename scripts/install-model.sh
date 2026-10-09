#!/bin/sh
# Install the name model's two files (RFC Q23, M5.1) into protonctl's model
# folder, from a local folder (--from DIR) or a URL base (--url BASE), or
# report whether they are in place (--check). A file is moved into place only
# after its SHA-256 matches the value pinned in src/privacy/detect/model.rs,
# so a wrong file is refused and the folder is left as it was. protonctl
# itself never fetches anything; this script is the only fetch, and only when
# it is run by hand.
#
#   scripts/install-model.sh --from DIR
#   scripts/install-model.sh --url BASE     (fetches BASE/otter.onnx, BASE/tokenizer.json)
#   scripts/install-model.sh --check        (exit 0 when both files are in place)
#
# Who may write: the model folder is 0700 and its files 0600, so only the
# user who runs protonctl can read or change them.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
model_rs="$root/src/privacy/detect/model.rs"

die() {
    echo "install-model.sh: $*" >&2
    exit 1
}

usage() {
    echo "install-model.sh: choose --from DIR (a folder holding otter.onnx and tokenizer.json) or --url BASE (a URL folder holding both)" >&2
    exit 2
}

# The pinned hash, read from its constant: the first 64-hex string on the
# line that starts the constant, or on the line after it when rustfmt wrapped it.
pinned() {
    value=$(awk -v name="pub const $1:" 'index($0, name) == 1 { on = 1 } on { print; if ($0 ~ /;/) exit }' "$model_rs" |
        grep -Eo '[0-9a-f]{64}' || true)
    if [ "$(printf '%s\n' "$value" | grep -c .)" != 1 ]; then
        die "cannot read exactly one $1 in $model_rs"
    fi
    printf '%s\n' "$value"
}

want_onnx=$(pinned ONNX_SHA256)
want_tokenizer=$(pinned TOKENIZER_SHA256)

if [ "$(uname -s)" = Darwin ] && command -v shasum >/dev/null; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    die "needs shasum or sha256sum to check the files"
fi

# The folder protonctl reads (platform::data_dir, model.rs's dir()): the
# Application Support folder on macOS; on Linux $XDG_DATA_HOME when it is an
# absolute path, else ~/.local/share.
case "$(uname -s)" in
    Darwin) data="$HOME/Library/Application Support/protonctl" ;;
    *)
        xdg="${XDG_DATA_HOME:-}"
        case "$xdg" in
            /*) ;;
            *) xdg="$HOME/.local/share" ;;
        esac
        data="$xdg/protonctl"
        ;;
esac
model="$data/model"

# ok, missing or mismatch, for a file in the model folder against its pin.
state() {
    if [ ! -f "$1" ]; then
        echo missing
    elif [ "$(sha256 "$1")" = "$2" ]; then
        echo ok
    else
        echo mismatch
    fi
}

mode=
src=
base=
while [ $# -gt 0 ]; do
    case "$1" in
        --from | --url)
            [ -z "$mode" ] && [ $# -ge 2 ] || usage
            mode=${1#--}
            case "$mode" in
                from) src=$2 ;;
                url) base=${2%/} ;;
            esac
            shift 2
            ;;
        --check)
            [ -z "$mode" ] || usage
            mode=check
            shift
            ;;
        *) usage ;;
    esac
done

if [ "$mode" = check ]; then
    onnx=$(state "$model/otter.onnx" "$want_onnx")
    tokenizer=$(state "$model/tokenizer.json" "$want_tokenizer")
    if [ "$onnx" = ok ] && [ "$tokenizer" = ok ]; then
        echo "install-model.sh: the name model is in $model, and both files match their pinned SHA-256"
        exit 0
    fi
    echo "install-model.sh: the name model is not installed in $model (otter.onnx: $onnx, tokenizer.json: $tokenizer)" >&2
    exit 1
fi

[ -n "$mode" ] || usage

tmp=$(mktemp -d "${TMPDIR:-/tmp}/install-model.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

# Stage and verify both files before touching the model folder, so a refusal
# leaves it as it was.
for name in otter.onnx tokenizer.json; do
    if [ "$name" = otter.onnx ]; then pin=$want_onnx; else pin=$want_tokenizer; fi
    if [ "$(state "$model/$name" "$pin")" = ok ]; then
        echo "install-model.sh: $name is already in place, and matches its pinned SHA-256; skipping it"
        continue
    fi
    if [ "$mode" = from ]; then
        cp "$src/$name" "$tmp/$name" || die "cannot read $src/$name; the model folder is unchanged"
    else
        /usr/bin/curl -fsSL -o "$tmp/$name" "$base/$name" ||
            die "cannot fetch $base/$name; the model folder is unchanged"
    fi
    got=$(sha256 "$tmp/$name")
    if [ "$got" != "$pin" ]; then
        die "refusing $name: its SHA-256 is $got, not the pinned $pin; the model folder is unchanged"
    fi
    chmod 600 "$tmp/$name"
    echo "install-model.sh: $name verified (SHA-256 $pin)"
    echo "$name" >>"$tmp/staged"
done

if [ -f "$tmp/staged" ]; then
    if [ ! -d "$model" ]; then
        mkdir -p "$data"
        mkdir -m 700 "$model"
    fi
    while read -r name; do
        mv -f "$tmp/$name" "$model/$name"
        echo "install-model.sh: installed $model/$name"
    done <"$tmp/staged"
fi
echo "install-model.sh: the name model is in $model; run protonctl doctor to load it"
