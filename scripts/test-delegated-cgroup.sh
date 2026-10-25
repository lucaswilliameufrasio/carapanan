#!/usr/bin/env bash
# Explicit developer opt-in. Creates only a transient, test-owned user unit; never called by the app.
set -euo pipefail

project_root=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)

if [[ ${1:-} == --inside ]]; then
    unit=${2:?missing test unit identity}
    if [[ ! $unit =~ ^carapana-cgroup-test-[0-9]+-[0-9]+-[0-9]+$ ]]; then
        echo 'Refusing an unexpected test unit identity.' >&2
        exit 1
    fi
    cgroup_path=
    while IFS= read -r record; do
        if [[ $record == 0::* ]]; then
            cgroup_path=${record#0::}
        fi
    done < /proc/self/cgroup
    if [[ $cgroup_path != /*/"$unit.service"/runner ]]; then
        echo 'Refusing cgroup setup outside this transient test unit.' >&2
        exit 1
    fi
    parent=/sys/fs/cgroup${cgroup_path%/runner}
    if [[ ! -O $parent || ! -f $parent/cgroup.controllers ]]; then
        echo 'Transient test parent was not delegated to this user.' >&2
        exit 1
    fi
    # This parent belongs only to the fresh unit. The test runner is already in its runner subgroup.
    printf '+memory +pids\n' > "$parent/cgroup.subtree_control"
    export CARAPANA_TEST_CGROUP_PARENT=$parent
    export CARAPANA_REQUIRE_CGROUP=1
    cd -- "$project_root"
    mise exec -- bash scripts/rust-toolchain-command.sh cargo test -p carapana-sandbox delegated_cgroup:: --locked -- --nocapture
    mise exec -- bash scripts/rust-toolchain-command.sh cargo test -p carapana-cli --test captured_cgroup --locked -- --nocapture
    exec mise exec -- bash scripts/rust-toolchain-command.sh cargo test -p carapana-cli --test image_launcher --locked -- --nocapture
fi

if (( $# != 0 )); then
    echo 'Usage: bash scripts/test-delegated-cgroup.sh' >&2
    exit 2
fi
if [[ $(uname -s) != Linux || $(id -u) == 0 ]]; then
    echo 'This fixture requires a non-root Linux user and a running systemd user manager.' >&2
    exit 1
fi
unit=carapana-cgroup-test-$(id -u)-$$-$(date +%s%N)
cleanup() {
    # Exact unique unit owned by this invocation; never stop any other user service.
    systemctl --user stop "$unit.service" >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

systemd-run --user --quiet --wait --collect --pipe \
    --unit="$unit" --working-directory="$project_root" \
    --property='Delegate=memory pids' --property=DelegateSubgroup=runner \
    --setenv="PATH=$PATH" --setenv="TMPDIR=${TMPDIR:-/var/tmp}" \
    --setenv=CARGO_BUILD_JOBS=2 \
    /usr/bin/bash "$project_root/scripts/test-delegated-cgroup.sh" --inside "$unit"
