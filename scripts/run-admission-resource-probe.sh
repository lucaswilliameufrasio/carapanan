#!/usr/bin/env bash
# One warmup + three serial samples of a fixed trusted control-plane probe.
set -euo pipefail
project_root=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
binary=$project_root/target/release/examples/admission_resource_probe
if [[ ! -x $binary ]]; then
    echo 'Build the release admission_resource_probe example with the repository toolchain first.' >&2
    exit 1
fi
samples() {
    for sample in 0 1 2 3; do
        python3 -I "$project_root/scripts/measure-admission-probe.py" --binary "$binary"
    done
}
if [[ ${1:-} == --inside ]]; then
    unit=${2:?missing unit identity}
    if [[ $# != 2 || ! $unit =~ ^carapana-admission-bench-[0-9]+-[0-9]+-[0-9]+$ ]]; then
        exit 2
    fi
    membership=$(cat /proc/self/cgroup)
    if [[ $membership != 0::/*/"$unit.service"/runner ]]; then
        echo 'Refusing benchmark outside the expected transient unit.' >&2
        exit 1
    fi
    samples
    exit
fi
if (( $# != 0 )); then
    echo 'Usage: bash scripts/run-admission-resource-probe.sh' >&2
    exit 2
fi
case $(uname -s) in
    Linux)
        if [[ $(id -u) == 0 ]]; then
            echo 'Use a non-root user with a running systemd user manager.' >&2
            exit 1
        fi
        unit=carapana-admission-bench-$(id -u)-$$-$(date +%s%N)
        cleanup() { systemctl --user stop "$unit.service" >/dev/null 2>&1 || true; }
        trap cleanup EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM
        systemd-run --user --quiet --wait --collect --pipe --unit="$unit" \
            --working-directory="$project_root" \
            --property='Delegate=memory pids' --property=DelegateSubgroup=runner \
            --property=MemoryMax=256M --property=MemorySwapMax=0 --property=TasksMax=32 \
            --property=CPUQuota=100% --property=RuntimeMaxSec=60 \
            /usr/bin/env -i PATH=/usr/local/bin:/usr/bin:/bin TMPDIR=/var/tmp \
            /usr/bin/bash "$project_root/scripts/run-admission-resource-probe.sh" --inside "$unit"
        ;;
    Darwin)
        # No claim of cgroup/hard memory enforcement on macOS; the operator runs this fixed small probe.
        samples
        ;;
    *)
        echo 'Unsupported resource probe platform.' >&2
        exit 1
        ;;
esac
