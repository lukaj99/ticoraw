#!/bin/sh
# Downloads the conformance samples listed in manifest.toml into a cache directory and
# verifies each one against its pinned SHA-256.
#
# Usage: conformance/fetch.sh <cache-dir> [manifest.toml]
#
# Each sample is saved as <cache-dir>/<id>.nef. Files that are already present with the
# right checksum are not downloaded again. Downloads go to a temporary name and are only
# renamed into place once their checksum matches; nothing downloaded is ever executed.
# Exit status: 0 when every sample is present and verified, 1 otherwise, 2 on usage errors.

set -eu

usage() {
  echo "usage: $0 <cache-dir> [manifest.toml]" >&2
  exit 2
}

[ "$#" -ge 1 ] && [ "$#" -le 2 ] || usage
cache=$1
manifest=${2:-$(dirname "$0")/manifest.toml}
[ -f "$manifest" ] || {
  echo "fetch.sh: manifest not found: $manifest" >&2
  exit 2
}

command -v curl >/dev/null 2>&1 || {
  echo "fetch.sh: curl is required" >&2
  exit 1
}

if command -v sha256sum >/dev/null 2>&1; then
  sha256_of() { sha256sum -- "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256_of() { shasum -a 256 -- "$1" | cut -d ' ' -f 1; }
elif command -v openssl >/dev/null 2>&1; then
  sha256_of() { openssl dgst -sha256 -r "$1" | cut -d ' ' -f 1; }
else
  echo "fetch.sh: need sha256sum, shasum or openssl" >&2
  exit 1
fi

# One line per sample: "<id> <nef_sha256> <url>". Ids are [a-z0-9_]+, hashes are hex and
# URLs are percent-encoded, so none contain spaces.
list=$(awk '
  function flush() {
    if (id != "" || sha != "" || url != "") {
      if (id == "" || sha == "" || url == "") {
        print "fetch.sh: incomplete [[sample]] ending at line " NR > "/dev/stderr"
        bad = 1
      } else {
        print id, sha, url
      }
    }
    id = ""; sha = ""; url = ""
  }
  function value(line) {
    sub(/^[^=]*=[ \t]*"/, "", line)
    sub(/".*$/, "", line)
    return line
  }
  /^\[\[sample\]\][ \t]*$/ { flush(); next }
  /^id[ \t]*=/ { id = value($0); next }
  /^nef_sha256[ \t]*=/ { sha = value($0); next }
  /^url[ \t]*=/ { url = value($0); next }
  END { flush(); exit bad }
' "$manifest")

mkdir -p -- "$cache"

agent="ticoraw-conformance-fetch (+https://github.com/lukaj99/ticoraw)"
part=
# Never leave a partial download behind, even when interrupted.
trap 'rm -f -- "$part"' EXIT
trap 'exit 130' INT TERM
total=0
fetched=0
failed=0
first=1

# Download $2 to $1 over HTTPS only. curl retries transient failures with exponential
# backoff; --fail turns HTTP errors into a non-zero exit.
download() {
  curl --fail --silent --show-error --location \
    --proto '=https' --proto-redir '=https' \
    --retry 5 --retry-connrefused --retry-max-time 600 \
    --connect-timeout 30 --max-time 1800 \
    --user-agent "$agent" \
    --output "$1" -- "$2"
}

while read -r id sha url; do
  [ -n "$id" ] || continue
  total=$((total + 1))
  case $id in
  *[!a-z0-9_]*)
    echo "fetch.sh: bad sample id: $id" >&2
    failed=$((failed + 1))
    continue
    ;;
  esac
  dest=$cache/$id.nef
  if [ -f "$dest" ] && [ "$(sha256_of "$dest")" = "$sha" ]; then
    echo "ok       $id (cached)"
    continue
  fi
  part=$dest.part
  ok=0
  for attempt in 1 2; do
    # Be polite to the archives: pause between downloads and before a second attempt.
    if [ "$first" -eq 0 ]; then
      sleep 2
    fi
    first=0
    rm -f -- "$part"
    if download "$part" "$url"; then
      got=$(sha256_of "$part")
      if [ "$got" = "$sha" ]; then
        mv -f -- "$part" "$dest"
        ok=1
        break
      fi
      echo "fetch.sh: $id: SHA-256 mismatch (attempt $attempt): got $got, want $sha" >&2
    else
      echo "fetch.sh: $id: download failed (attempt $attempt): $url" >&2
    fi
    [ "$attempt" -eq 2 ] || sleep 30
  done
  rm -f -- "$part"
  if [ "$ok" -eq 1 ]; then
    fetched=$((fetched + 1))
    echo "fetched  $id"
  else
    failed=$((failed + 1))
    echo "FAILED   $id" >&2
  fi
done <<EOF
$list
EOF

echo "fetch.sh: $total samples, $fetched downloaded, $failed failed, cache $cache"
[ "$total" -gt 0 ] || {
  echo "fetch.sh: no samples in $manifest" >&2
  exit 1
}
[ "$failed" -eq 0 ]
