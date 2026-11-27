#!/usr/bin/env bash
# Creates xReveal's stable, self-signed code-signing certificate, "xReveal Self-Signed", in the
# login keychain (#162). Signing every build with it keeps macOS's Camera and Microphone access
# across rebuilds and updates: the access is tied to the signature's designated requirement,
# which an ad-hoc signature changes on every build and this certificate doesn't.
#
# Run once on the Mac that signs releases. It refuses to run if the certificate is already there.
# With --export <file.p12>, it also writes the certificate and its key, protected by a password it
# asks for, to store as GitHub Actions secrets for release.yml.
#
# It is not notarised: a first install from the web still gets macOS's "unidentified developer"
# warning. Updates xReveal installs itself aren't quarantined, so they don't.
set -euo pipefail

name="xReveal Self-Signed"
keychain="$HOME/Library/Keychains/login.keychain-db"

if security find-certificate -c "$name" "$keychain" >/dev/null 2>&1; then
  # Already made: --export can still write it out for release.yml's secrets. `security export`
  # takes every identity in the keychain, so only when this is the only one; otherwise export it
  # from Keychain Access (My Certificates › xReveal Self-Signed › Export).
  if [[ "${1:-}" == "--export" && -n "${2:-}" ]]; then
    if [[ "$(security find-identity "$keychain" | grep -c '^ *[0-9]*)')" != 1 ]]; then
      echo "make-signing-cert: the login keychain has other identities too; export \"$name\" from Keychain Access." >&2
      exit 1
    fi
    read -r -s -p "Password for $2: " export_password
    echo
    security export -k "$keychain" -t identities -f pkcs12 -P "$export_password" -o "$2"
    echo "make-signing-cert: wrote $2 (keep it private; it's what release.yml signs with)."
    exit 0
  fi
  echo "make-signing-cert: \"$name\" is already in the login keychain." >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -r "$work"' EXIT
# Ten years; code signing only; not a CA.
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -subj "/CN=$name" \
  -addext "basicConstraints=critical,CA:false" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" \
  -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
password="$(/usr/bin/openssl rand -hex 16)"
/usr/bin/openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -name "$name" -out "$work/cert.p12" -passout "pass:$password"
# -T: codesign may use the key without asking each time.
security import "$work/cert.p12" -k "$keychain" -P "$password" -T /usr/bin/codesign >/dev/null
echo "make-signing-cert: added \"$name\" to the login keychain."

if [[ "${1:-}" == "--export" && -n "${2:-}" ]]; then
  read -r -s -p "Password for $2: " export_password
  echo
  /usr/bin/openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -name "$name" -out "$2" -passout "pass:$export_password"
  echo "make-signing-cert: wrote $2 (keep it private; it's what release.yml signs with)."
fi
