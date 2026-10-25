#!/usr/bin/env bash
# Always use the repository's installed numeric pin, even behind a mise shim/global override.
set -euo pipefail
project_root=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
toolchain=$(sed -n 's/^channel = "\([0-9][0-9.]*\)"$/\1/p' "$project_root/rust-toolchain.toml")
if [[ ! $toolchain =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || $# == 0 ]]; then
    echo 'A single numeric repository toolchain pin and a command are required.' >&2
    exit 2
fi
# rustup run fails if the toolchain is missing; this helper does not install or change defaults.
exec rustup run "$toolchain" "$@"
