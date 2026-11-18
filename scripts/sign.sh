#!/bin/sh
# Sign a built protonctl with a stable identity and put it in place.
# Keychain items trust a signed binary by its identifier and
# certificate, so a build signed here reads them with no new prompt, and a
# binary swapped in by anything else meets one.
#
#   sign.sh BUILT [INSTALLED]
#
# BUILT is the binary a build made. INSTALLED is where the signed copy goes;
# without it the signed copy takes BUILT's place. scripts/install.sh calls
# this after its build, and a Homebrew install calls it as `protonctl-sign`
# after each install or upgrade.
#
# The first run makes the identity: a self-signed code-signing certificate
# named "protonctl" in the login keychain, its key non-extractable and
# trusted by no program. So every run asks before codesign uses the key.
# Choose Allow, never Always Allow: any process can run codesign, so
# Always Allow would let it sign a swapped binary with no prompt at all.
set -eu

if [ "$(uname -s)" != Darwin ]; then
    echo "sign.sh signs for the macOS Keychain; Linux needs no signing" >&2
    exit 1
fi
if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "usage: sign.sh BUILT [INSTALLED]" >&2
    exit 2
fi
built=$1
installed=${2:-$1}
if [ ! -f "$built" ]; then
    echo "sign.sh: no file at $built" >&2
    exit 1
fi
# A rename onto a link would leave the link's target as it was, unsigned.
if [ -L "$installed" ]; then
    echo "sign.sh: $installed is a link; name the file it points to" >&2
    exit 1
fi

id=protonctl
login="$HOME/Library/Keychains/login.keychain-db"

if ! security find-identity -p codesigning "$login" | grep -q "\"$id\""; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    # The passphrase guards the transfer file for the second it exists.
    pass=$(/usr/bin/openssl rand -hex 16)
    /usr/bin/openssl req -x509 -newkey rsa:3072 -nodes -days 3650 -subj "/CN=$id" \
        -addext "extendedKeyUsage=critical,codeSigning" \
        -addext "keyUsage=critical,digitalSignature" \
        -addext "basicConstraints=critical,CA:false" \
        -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null
    PASS="$pass" /usr/bin/openssl pkcs12 -export -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
        -out "$tmp/id.p12" -passout env:PASS
    # -T "": without it the importing tool, /usr/bin/security, may sign
    # with the key unasked, and `security cms -S` can make a signature.
    security import "$tmp/id.p12" -k "$login" -P "$pass" -x -T "" >/dev/null
    rm -rf "$tmp"
    echo "sign.sh: made the signing identity \"$id\" in the login keychain (valid 10 years)"
fi

# A copy is signed and then renamed into place, so a server the Claude app
# is running keeps its own file, and a copy that fails the check below never
# takes the place of the one installed.
folder=$(dirname "$installed")
mkdir -p "$folder"
new="$folder/.protonctl.new"
cp "$built" "$new"
echo "sign.sh: macOS now asks to let codesign use the key \"$id\": choose Allow, not Always Allow"
# --options runtime: without the hardened runtime, dyld loads any library
# DYLD_INSERT_LIBRARIES names into the signed binary, and that code then
# reads the Keychain items as protonctl, with no prompt.
if ! codesign --force --sign "$id" --identifier protonctl --options runtime "$new"; then
    rm -f "$new"
    echo "sign.sh: codesign did not sign; $installed is as it was" >&2
    exit 1
fi
if ! codesign -d -v "$new" 2>&1 | grep -q 'flags=.*runtime'; then
    rm -f "$new"
    echo "sign.sh: the signed copy lacks the hardened runtime; $installed is as it was" >&2
    exit 1
fi
mv -f "$new" "$installed"
codesign -d -r- "$installed" 2>&1 | grep designated
echo "sign.sh: signed $installed; restart Claude Code and Claude Desktop to run it"
