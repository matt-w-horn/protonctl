#!/bin/sh
# Build protonctl, sign it with a stable identity, and install it into
# ~/.cargo/bin (RFC Q12). Keychain items trust a signed binary by its
# identifier and certificate, so a rebuild signed here reads them with no
# new prompt, and a binary swapped in by anything else meets one.
#
# The first run makes the identity: a self-signed code-signing certificate
# named "protonctl" in the login keychain, its key non-extractable and
# trusted by no program. So every run asks before codesign uses the key.
# Choose Allow, never Always Allow: any process can run codesign, so
# Always Allow would let it sign a swapped binary with no prompt at all.
set -eu
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != Darwin ]; then
    echo "install.sh signs for the macOS Keychain; on Linux run: cargo install --path . --locked" >&2
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
    echo "install.sh: made the signing identity \"$id\" in the login keychain (valid 10 years)"
fi

cargo build --release --locked
echo "install.sh: macOS now asks to let codesign use the key \"$id\": choose Allow, not Always Allow"
codesign --force --sign "$id" --identifier protonctl target/release/protonctl
# Through a rename, so a server the Claude app is running keeps its own file.
mkdir -p "$HOME/.cargo/bin"
cp target/release/protonctl "$HOME/.cargo/bin/.protonctl.new"
mv -f "$HOME/.cargo/bin/.protonctl.new" "$HOME/.cargo/bin/protonctl"
codesign -d -r- "$HOME/.cargo/bin/protonctl" 2>&1 | grep designated
echo "install.sh: installed; restart Claude Code and Claude Desktop to run it"
