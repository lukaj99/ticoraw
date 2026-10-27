#!/usr/bin/env bash
# Builds the static library, a synthetic NEF, tests/c/test_capi.c and examples/convert.c,
# then runs the C test and converts the synthetic NEF with the example.
#
#   crates/ticoraw-capi/scripts/c-test.sh
#   SANITIZE=address crates/ticoraw-capi/scripts/c-test.sh   # AddressSanitizer (+ UBSan)
#   TICORAW_CORPUS=/dir crates/ticoraw-capi/scripts/c-test.sh  # also up to
#       TICORAW_CAPI_LIMIT (default 3) NEFs from that folder, which is only read
#
# CC picks the C compiler (default: cc; clang on Windows). Output goes to
# target/capi-test unless CAPI_OUT names another directory.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
crate="$root/crates/ticoraw-capi"
out=${CAPI_OUT:-$root/target/capi-test}
target_dir=${CARGO_TARGET_DIR:-$root/target}
mkdir -p "$out"

case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*) windows=1 ;;
*) windows=0 ;;
esac

# Build the static library and ask rustc which system libraries it needs.
log="$out/cargo-rustc.log"
# --color never: CI sets CARGO_TERM_COLOR=always, whose escape codes would end up in the
# library names parsed below.
if ! cargo rustc --color never --manifest-path "$crate/Cargo.toml" --release --lib \
  --crate-type staticlib -- --print native-static-libs >"$log" 2>&1; then
  cat "$log" >&2
  exit 1
fi
native=$(sed -n 's/.*native-static-libs: //p' "$log" | tail -n 1 | sed 's/\x1b\[[0-9;]*m//g')

if [ "$windows" = 1 ]; then
  cc=${CC:-clang}
  lib="$target_dir/release/ticoraw_capi.lib"
  exe=.exe
  # Rust links the dynamic CRT. MSVC linker flags (`/defaultlib:msvcrt`) are passed as
  # `-defaultlib:msvcrt`, which link.exe also accepts and MSYS does not mistake for a path.
  cflags=(-fms-runtime-lib=dll -D_CRT_SECURE_NO_WARNINGS)
  libs=()
  for l in $native; do
    case "$l" in
    /*) libs+=(-Xlinker "-${l#/}") ;;
    # `kernel32.lib` as a bare name is an input-file path to clang; `-lkernel32` makes the
    # linker search the Windows SDK library path instead.
    *.lib) libs+=("-l${l%.lib}") ;;
    *) libs+=("$l") ;;
    esac
  done
else
  cc=${CC:-cc}
  lib="$target_dir/release/libticoraw_capi.a"
  exe=
  # Windows SDK headers are not -Wextra clean, so warnings are fatal elsewhere only.
  cflags=(-Werror)
  read -r -a libs <<<"$native"
fi

cflags+=(-std=c99 -Wall -Wextra -g -I "$crate/include")
case "${SANITIZE:-}" in
"") ;;
address) cflags+=(-fsanitize=address -fsanitize=undefined -fno-omit-frame-pointer -fno-sanitize-recover=all) ;;
*)
  echo "c-test: unknown SANITIZE=${SANITIZE}" >&2
  exit 2
  ;;
esac

# `${a[@]+"${a[@]}"}` keeps an empty array legal under `set -u` in bash 3.2 (macOS).
"$cc" "${cflags[@]}" "$crate/tests/c/test_capi.c" "$lib" ${libs[@]+"${libs[@]}"} -o "$out/test_capi$exe"
"$cc" "${cflags[@]}" "$crate/examples/convert.c" "$lib" ${libs[@]+"${libs[@]}"} -o "$out/convert$exe"

read -r width height hash < <(cargo run --quiet --manifest-path "$crate/Cargo.toml" --release \
  --example synthetic_nef -- "$out/synthetic.NEF")

extra=()
if [ -n "${TICORAW_CORPUS:-}" ]; then
  limit=${TICORAW_CAPI_LIMIT:-3}
  while IFS= read -r f; do
    extra+=("$f")
  done < <(find "$TICORAW_CORPUS" -maxdepth 1 -type f \( -iname '*.nef' \) | sort | head -n "$limit")
fi

"$out/test_capi$exe" "$out/synthetic.NEF" "$width" "$height" "$hash" ${extra[@]+"${extra[@]}"}
"$out/convert$exe" "$out/synthetic.NEF" "$out/synthetic.dng"
"$out/convert$exe" --uncompressed --no-preview "$out/synthetic.NEF" "$out/synthetic-plain.dng"
