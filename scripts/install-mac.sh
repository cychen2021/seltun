#!/bin/bash
# Installs seltun on macOS and signs the daemon with a stable self-signed certificate.
#
# An ad-hoc signed binary is identified by its hash, so each build looks like a new program to macOS. The launchd background item and each permission that macOS keeps for the daemon then belong to the old build. Signed with a fixed certificate and identifier, the daemon keeps its identity across reinstalls.
#
# Usage: scripts/install-mac.sh [--no-install]
#   --no-install  only sign the daemon that is already installed
# Environment: SELTUN_CERT names the certificate (default "seltun Code Signing").
set -euo pipefail

CERT="${SELTUN_CERT:-seltun Code Signing}"
IDENTIFIER="io.github.cychen2021.seltund"
LABEL="io.github.cychen2021.seltund"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ "$(uname)" != Darwin ]]; then
    echo "This script is for macOS only." >&2
    exit 1
fi

install=1
for arg in "$@"; do
    case "$arg" in
        --no-install) install=0 ;;
        -h|--help) sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

create_cert() {
    echo "Creating the self-signed certificate \"$CERT\" in the login keychain..."
    local dir
    dir="$(mktemp -d)"
    trap 'rm -rf "$dir"' RETURN
    cat > "$dir/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $CERT
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
CNF
    # The system LibreSSL writes a PKCS#12 file that `security import` can read. OpenSSL 3 needs `-legacy`.
    /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$dir/cert.cnf" -keyout "$dir/key.pem" -out "$dir/cert.pem" 2>/dev/null
    /usr/bin/openssl pkcs12 -export -inkey "$dir/key.pem" -in "$dir/cert.pem" -out "$dir/cert.p12" -passout pass:seltun
    # -T lets codesign use the key with no prompt.
    security import "$dir/cert.p12" -k "$KEYCHAIN" -P seltun -T /usr/bin/codesign >/dev/null
}

if ! security find-certificate -c "$CERT" "$KEYCHAIN" >/dev/null 2>&1; then
    create_cert
fi

if [[ $install == 1 ]]; then
    # The build hook (`hatch_build.py`) signs the daemon. `--reinstall-package` makes uv build again, also when a cached wheel has the old signature.
    uv tool install --force --reinstall-package seltun "$ROOT"
fi

BIN="$(find "$(uv tool dir)/seltun" -path '*/seltun/bin/seltund' -type f 2>/dev/null | head -n 1)"
if [[ -z "$BIN" ]]; then
    echo "The installed daemon is not found under $(uv tool dir)/seltun." >&2
    exit 1
fi

echo "Signing $BIN..."
codesign --force --sign "$CERT" --identifier "$IDENTIFIER" "$BIN"
codesign --display --requirements - "$BIN" 2>&1 | grep designated

if [[ -f "/Library/LaunchDaemons/$LABEL.plist" ]]; then
    # The sudoers rule of `seltun autostart enable` permits this command with no password.
    echo "Restarting the daemon..."
    sudo /bin/launchctl kickstart -k "system/$LABEL"
fi
