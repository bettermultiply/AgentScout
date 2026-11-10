#!/usr/bin/env bash
# Privileged validation for the AScout filesystem backend.
#
# Run this as a normal user. Steps that need privilege use sudo internally;
# cargo and the test binaries are always executed as the invoking user
# (except the privileged Rust tests, which need root).
set -euxo pipefail
cd "$(dirname "$0")/.."

SUMMARY_FAIL=0
LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ascout-priv-check.XXXXXX")"
exec > >(tee "$LOG_DIR/check.log") 2>&1
echo "diagnostics retained in $LOG_DIR"

WORKSPACE=""
STATE=""
OVER=""
SUDO=""
BTRFS_AVAILABLE=0
BTRFS_BIN=""
TEST_BIN=""
PROCESS_RUN_COUNT=0
PROCESS_PASS_COUNT=0
PROCESS_IGNORED_COUNT=0
PRIV_RUN_COUNT=0
PRIV_PASS_COUNT=0
PRIV_IGNORED_COUNT=0
ACTUAL_FREEZE_MECHANISM="unknown"

PRIV_TESTS=(
    priv_btrfs_chain_layout
    priv_btrfs_flatten_layout
    priv_btrfs_mixed_generations
    priv_direct_root_overlay_is_available_in_initial_userns
)
PROCESS_TESTS=(
    runtime::process::tests::terminate_uses_cgroup_or_group_kill_and_cleanup_removes_cgroup
    runtime::process::tests::failed_launch_cleans_process_pending_status_and_cgroup
    runtime::process::tests::stop_reports_cgroup_kill_write_failure
    runtime::process::tests::stop_unknown_or_exited_process_is_not_an_error
    runtime::process::tests::exited_process_is_reported_as_not_running
    runtime::backends::filesystem::tests::process::process_freeze_thaw_stops_and_resumes_writer
    runtime::backends::filesystem::tests::process::process_group_fallback_freezes_and_reports_mechanism
)
PRIV_PREFIX="runtime::backends::filesystem::tests::privileged::"
PRIV_FILTER="${PRIV_PREFIX}priv_"
TEST_OUTPUT_CHECK="scripts/check-test-output.py"

CHECKS=(
    environment
    layout
    btrfs-raw
    direct-root-overlay
    rust-build
    process-freeze
    privileged-rust
)
declare -A CHECK_STATUS
declare -A CHECK_DETAIL
for name in "${CHECKS[@]}"; do
    CHECK_STATUS[$name]=UNRUN
    CHECK_DETAIL[$name]=""
done
CURRENT_CHECK=""
CURRENT_FAILED=0

begin_check() {
    CURRENT_CHECK="$1"
    CURRENT_FAILED=0
}
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; CURRENT_FAILED=1; SUMMARY_FAIL=1; }
skip() { echo "SKIP: $*"; CURRENT_FAILED=1; SUMMARY_FAIL=1; }
complete_check() {
    if [ "$CURRENT_FAILED" -eq 0 ]; then
        CHECK_STATUS[$CURRENT_CHECK]=PASS
    else
        CHECK_STATUS[$CURRENT_CHECK]=FAIL
    fi
}
step() {
    echo
    echo "===== $* ====="
}

make_dir() {
    local dir="$1"
    if [ -d "$dir" ]; then
        return 0
    fi
    if mkdir -p "$dir" 2>/dev/null; then
        return 0
    fi
    [ -n "$SUDO" ] || return 1
    $SUDO mkdir -p "$dir" || return 1
    if [ "$(id -u)" -ne 0 ]; then
        $SUDO chown "$(id -u):$(id -g)" "$dir" || return 1
    fi
}

remove_tree() {
    local dir="$1"
    [ -n "$dir" ] || return 0
    [ -e "$dir" ] || return 0
    if [ -n "$BTRFS_BIN" ]; then
        # Delete descendant btrfs subvolumes deepest-first; plain rm cannot
        # remove a subvolume root. Failures here are retried by rm below.
        # The btrfs path is passed as an argument, not through sudo's
        # environment, because sudo may reset the environment.
        $SUDO find "$dir" -depth -type d -exec sh -c \
            'bin="$1"; target="$2"; [ -x "$bin" ] && "$bin" subvolume show "$target" >/dev/null 2>&1 && "$bin" subvolume delete "$target" >/dev/null 2>&1; true' \
            sh "$BTRFS_BIN" {} \; >/dev/null 2>&1 || true
    fi
    $SUDO rm -rf "$dir"
}

mount_lines() {
    findmnt -rn -o TARGET,FSTYPE,OPTIONS 2>/dev/null
}

mount_target_of() {
    findmnt -n -o TARGET -T "$1" 2>/dev/null | head -n1
}

is_pseudo_fstype() {
    case "$1" in
        proc|sysfs|devtmpfs|devpts|cgroup|cgroup2|securityfs|pstore|bpf|configfs|debugfs|tracefs|fusectl|mqueue|hugetlbfs|autofs|binfmt_misc|efivarfs|fuse.portal|nsfs|squashfs|iso9660|ramfs|overlay|fuse|vfat)
            return 0 ;;
    esac
    return 1
}

# Shell approximation of the backend's isolated set: the backend isolates
# every writable, non-pseudo mount except "/".  The privileged test asserts the
# real condition again through the backend.
is_isolatable() {
    local target="$1" fstype="$2" options="$3"
    if [ "$target" = "/" ]; then
        return 1
    fi
    case ",$options," in *,rw,*) ;; *) return 1 ;; esac
    if is_pseudo_fstype "$fstype"; then
        return 1
    fi
    case "$target" in /proc*|/sys*|/dev*) return 1 ;; esac
    return 0
}

mount_description() {
    local path="$1"
    local line
    line="$(findmnt -n -o TARGET,FSTYPE,OPTIONS -T "$path" 2>/dev/null | head -n1 || true)"
    [ -n "$line" ] && printf '%s' "$line" || printf 'unknown'
}

print_covered_checks() {
    echo
    echo "===== covered checks ====="
    echo "1. environment and required tools"
    echo "2. non-overlapping state directory on btrfs + workspace on another mount"
    echo "3. raw btrfs: subvolume create, readonly snapshots, send -p | receive --dump, strict delete"
    echo "4. direct lower=/ overlay in the initial user namespace"
    echo "5. Rust test binary build (invoking user)"
    echo "6. process-group freeze tests: runtime::process + filesystem::tests::process"
    echo "7. privileged Rust tests: priv_btrfs_chain_layout, priv_btrfs_flatten_layout,"
    echo "   priv_btrfs_mixed_generations, priv_direct_root_overlay_is_available_in_initial_userns"
}

print_summary() {
    echo
    echo "===== coverage summary ====="
    printf '%-28s %-6s %s\n' "CHECK" "RESULT" "DETAIL"
    for name in "${CHECKS[@]}"; do
        printf '%-28s %-6s %s\n' "$name" "${CHECK_STATUS[$name]}" "${CHECK_DETAIL[$name]}"
    done
    printf '%-28s %-6s %s\n' "process tests executed" "$PROCESS_RUN_COUNT" "freeze mechanism: $ACTUAL_FREEZE_MECHANISM"
    printf '%-28s %-6s %s\n' "process tests passed" "$PROCESS_PASS_COUNT" "ignored: $PROCESS_IGNORED_COUNT"
    printf '%-28s %-6s %s\n' "privileged tests executed" "$PRIV_RUN_COUNT" "expected ${#PRIV_TESTS[@]}"
    printf '%-28s %-6s %s\n' "privileged tests passed" "$PRIV_PASS_COUNT" "ignored: $PRIV_IGNORED_COUNT"
}

finish() {
    local status=$?
    set +ex
    trap - EXIT
    if [ "$status" -ne 0 ]; then
        SUMMARY_FAIL=1
    fi
    for name in "${CHECKS[@]}"; do
        if [ "${CHECK_STATUS[$name]}" != PASS ]; then
            SUMMARY_FAIL=1
        fi
    done
    if [ "$SUMMARY_FAIL" -eq 0 ]; then
        if [ -n "$WORKSPACE" ] && ! remove_tree "$WORKSPACE"; then
            echo "FAIL: workspace cleanup: $WORKSPACE"
            SUMMARY_FAIL=1
        fi
        if [ -n "$STATE" ] && ! remove_tree "$STATE"; then
            echo "FAIL: state cleanup: $STATE"
            SUMMARY_FAIL=1
        fi
    fi
    print_summary
    if [ "$SUMMARY_FAIL" -ne 0 ]; then
        if [ -n "$WORKSPACE" ]; then
            echo "retained workspace for inspection: $WORKSPACE"
        fi
        if [ -n "$STATE" ]; then
            echo "retained state directory for inspection: $STATE"
        fi
        echo "SUMMARY: PRIVILEGED CHECKS FAILED (diagnostics: $LOG_DIR)"
        exit 1
    fi
    echo "SUMMARY: ALL PRIVILEGED CHECKS PASSED (diagnostics: $LOG_DIR)"
}
trap finish EXIT

print_covered_checks

# ---------------------------------------------------------------------------
# Environment.
# ---------------------------------------------------------------------------
step "environment"
begin_check environment
id
for tool in cargo bwrap btrfs findmnt python3; do
    if command -v "$tool" >/dev/null 2>&1; then
        echo "found: $tool -> $(command -v "$tool")"
        if [ "$tool" = "btrfs" ]; then
            BTRFS_BIN="$(command -v btrfs)"
        fi
    else
        fail "required tool not found: $tool"
    fi
done
if [ "$(id -u)" -eq 0 ]; then
    SUDO=""
    echo "running as root; sudo not needed"
elif command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
    echo "running as user $(id -u); privileged steps use sudo"
else
    SUDO=""
    fail "sudo is unavailable and the caller is not root"
fi
CHECK_DETAIL[environment]="uid=$(id -u) gid=$(id -g)"
complete_check

# ---------------------------------------------------------------------------
# Choose state and workspace directories.
# ---------------------------------------------------------------------------
step "state/workspace layout"
begin_check layout
STATE_MOUNT=""
if [ -n "${ASCOUT_PRIV_STATE_DIR:-}" ]; then
    STATE_BASE="$ASCOUT_PRIV_STATE_DIR"
    echo "using ASCOUT_PRIV_STATE_DIR=$STATE_BASE"
    make_dir "$STATE_BASE" || fail "cannot create ASCOUT_PRIV_STATE_DIR=$STATE_BASE"
else
    STATE_BASE=""
    while read -r target fstype options; do
        [ "$fstype" = "btrfs" ] || continue
        if is_isolatable "$target" "$fstype" "$options"; then
            continue
        fi
        STATE_BASE="$target"
        break
    done < <(mount_lines)
    if [ -z "$STATE_BASE" ]; then
        fail "NO_VALID_STATE_LAYOUT: need a writable btrfs directory outside every mount the backend isolates (on this host usually directly under the btrfs filesystem mounted at /); set ASCOUT_PRIV_STATE_DIR"
    else
        echo "auto-selected btrfs state base: $STATE_BASE"
    fi
fi

if [ -n "$STATE_BASE" ]; then
    STATE="$STATE_BASE/.ascout-priv-check-state-$$"
    make_dir "$STATE" || fail "cannot create state run directory $STATE"
fi

if [ -n "$STATE" ]; then
    STATE_MOUNT="$(mount_target_of "$STATE" || true)"
    STATE_LINE="$(mount_description "$STATE")"
    echo "state directory: $STATE"
    echo "state mount: $STATE_LINE"
    STATE_FS="$(stat -f -c %T "$STATE" 2>/dev/null || true)"
    echo "state fs type: $STATE_FS"
    if [ "$STATE_FS" != "btrfs" ]; then
        fail "state directory is not btrfs: $STATE"
    else
        BTRFS_AVAILABLE=1
        pass "state directory is btrfs"
    fi
    state_fs_type="$(printf '%s\n' "$STATE_LINE" | awk '{print $2}')"
    state_options="$(printf '%s\n' "$STATE_LINE" | awk '{print $3}')"
    if [ -z "$STATE_MOUNT" ]; then
        fail "cannot determine the mount of $STATE (findmnt failed)"
    elif is_isolatable "$STATE_MOUNT" "${state_fs_type:-unknown}" "${state_options:-rw}"; then
        fail "state directory is inside a mount the backend isolates: $STATE_MOUNT; move ASCOUT_PRIV_STATE_DIR outside every isolated mount"
    fi
fi

if [ -n "${ASCOUT_PRIV_WORKSPACE_DIR:-}" ]; then
    WORKSPACE_BASE="$ASCOUT_PRIV_WORKSPACE_DIR"
    echo "using ASCOUT_PRIV_WORKSPACE_DIR=$WORKSPACE_BASE"
    make_dir "$WORKSPACE_BASE" || fail "cannot create ASCOUT_PRIV_WORKSPACE_DIR=$WORKSPACE_BASE"
elif [ -n "$STATE" ]; then
    WORKSPACE_BASE=""
    for candidate in /tmp /var/tmp /home /srv; do
        [ -d "$candidate" ] || continue
        candidate_mount="$(mount_target_of "$candidate" || true)"
        [ -n "$candidate_mount" ] || continue
        if [ -n "$STATE_MOUNT" ] && [ "$candidate_mount" = "$STATE_MOUNT" ]; then
            continue
        fi
        trial="$candidate/.ascout-priv-workspace-$$"
        if make_dir "$trial"; then
            WORKSPACE_BASE="$candidate"
            break
        fi
    done
    if [ -z "$WORKSPACE_BASE" ]; then
        fail "NO_VALID_WORKSPACE_MOUNT: need a writable mount different from the state mount; set ASCOUT_PRIV_WORKSPACE_DIR (for example a directory under /tmp)"
    else
        echo "auto-selected workspace base: $WORKSPACE_BASE"
    fi
fi

if [ -n "$WORKSPACE_BASE" ]; then
    WORKSPACE="$WORKSPACE_BASE/.ascout-priv-workspace-$$"
    make_dir "$WORKSPACE" || fail "cannot create workspace run directory $WORKSPACE"
fi

if [ -n "$WORKSPACE" ]; then
    WORKSPACE_MOUNT="$(mount_target_of "$WORKSPACE" || true)"
    WORKSPACE_LINE="$(mount_description "$WORKSPACE")"
    echo "workspace directory: $WORKSPACE"
    echo "workspace mount: $WORKSPACE_LINE"
    if [ -n "$STATE_MOUNT" ] && [ "$WORKSPACE_MOUNT" = "$STATE_MOUNT" ]; then
        fail "workspace and state directory are on the same mount: $WORKSPACE_MOUNT"
    fi
    workspace_fs_type="$(printf '%s\n' "$WORKSPACE_LINE" | awk '{print $2}')"
    workspace_options="$(printf '%s\n' "$WORKSPACE_LINE" | awk '{print $3}')"
    if [ -z "$WORKSPACE_MOUNT" ]; then
        fail "cannot determine the mount of workspace $WORKSPACE (findmnt failed)"
    elif ! is_isolatable "$WORKSPACE_MOUNT" "${workspace_fs_type:-unknown}" "${workspace_options:-rw}"; then
        fail "workspace is not on a writable mount the backend isolates: $WORKSPACE_MOUNT"
    fi
fi

if [ "$CURRENT_FAILED" -eq 0 ]; then
    pass "non-overlapping state/workspace layout"
fi
CHECK_DETAIL[layout]="state=${STATE:-<none>} workspace=${WORKSPACE:-<none>}"
complete_check
LAYOUT_OK=0
if [ "${CHECK_STATUS[layout]}" = PASS ]; then
    LAYOUT_OK=1
fi

# ---------------------------------------------------------------------------
# Raw btrfs capability checks.
# ---------------------------------------------------------------------------
step "btrfs raw capabilities"
begin_check btrfs-raw
if [ "$LAYOUT_OK" -ne 1 ] || [ "$BTRFS_AVAILABLE" -ne 1 ]; then
    skip "btrfs raw checks (no valid btrfs state directory)"
elif [ -z "$BTRFS_BIN" ]; then
    fail "btrfs tool not found"
else
    WORK="$STATE/raw-$$"
    if $SUDO rm -rf "$WORK" && make_dir "$WORK"; then
        :
    else
        fail "cannot prepare raw btrfs work directory $WORK"
    fi

    step "btrfs subvolume create"
    if $SUDO "$BTRFS_BIN" subvolume create "$WORK/sub"; then
        pass "btrfs subvolume create"
    else
        fail "btrfs subvolume create"
    fi
    $SUDO chown -R "$(id -u):$(id -g)" "$WORK/sub"

    step "btrfs readonly snapshots"
    echo base > "$WORK/sub/base.txt"
    if $SUDO "$BTRFS_BIN" subvolume snapshot -r "$WORK/sub" "$WORK/snap1"; then
        pass "first readonly snapshot"
    else
        fail "first readonly snapshot"
    fi
    echo changed > "$WORK/sub/modified.txt"
    rm -f "$WORK/sub/base.txt"
    mkdir -p "$WORK/sub/dir"
    echo nested > "$WORK/sub/dir/nested.txt"
    mv "$WORK/sub/dir" "$WORK/sub/renamed"
    if $SUDO "$BTRFS_BIN" subvolume snapshot -r "$WORK/sub" "$WORK/snap2"; then
        pass "second readonly snapshot"
    else
        fail "second readonly snapshot"
    fi

    step "btrfs send -p | btrfs receive --dump"
    if $SUDO "$BTRFS_BIN" send -p "$WORK/snap1" "$WORK/snap2" 2>"$WORK/send.err" \
        | $SUDO "$BTRFS_BIN" receive --dump 2>"$WORK/receive.err" >"$WORK/dump.txt"; then
        pass "send -p | receive --dump"
        echo "--- first 80 dump lines ---"
        sed -n '1,80p' "$WORK/dump.txt"
        for expected in modified.txt base.txt renamed; do
            if grep -q "$expected" "$WORK/dump.txt"; then
                pass "dump mentions $expected"
            else
                fail "dump does not mention $expected"
            fi
        done
    else
        fail "send -p | receive --dump"
        echo "--- send stderr ---"; cat "$WORK/send.err" || true
        echo "--- receive stderr ---"; cat "$WORK/receive.err" || true
    fi
    cp "$WORK/send.err" "$WORK/receive.err" "$WORK/dump.txt" "$LOG_DIR/" 2>/dev/null || true

    step "btrfs subvolume delete"
    if $SUDO "$BTRFS_BIN" subvolume delete "$WORK/snap2" "$WORK/snap1" "$WORK/sub"; then
        pass "strict subvolume delete"
        $SUDO rm -rf "$WORK"
        WORK=""
    else
        fail "strict subvolume delete"
    fi
fi
CHECK_DETAIL[btrfs-raw]="create/snapshot/send-dump/strict-delete"
complete_check

# ---------------------------------------------------------------------------
# Direct lower=/ overlay in the initial user namespace.
# ---------------------------------------------------------------------------
step "direct lower=/ overlay in the initial user namespace"
begin_check direct-root-overlay
if [ "$LAYOUT_OK" -ne 1 ]; then
    fail "direct lower=/ requires a valid state directory"
elif [ "$(id -u)" -ne 0 ] && [ -z "$SUDO" ]; then
    fail "direct lower=/ needs root/initial userns (sudo unavailable)"
else
    OVER="$WORKSPACE/overlay-$$"
    if ! make_dir "$OVER/upper" || ! make_dir "$OVER/work"; then
        fail "cannot prepare overlay upper/work under $OVER"
    elif [ "$(id -u)" -ne 0 ]; then
        echo "NEEDS_ROOT: running direct lower=/ check through sudo"
        if $SUDO unshare --mount --propagation private \
            bwrap --die-with-parent --ro-bind / / --proc /proc --dev /dev \
            --overlay-src / --overlay "$OVER/upper" "$OVER/work" / \
            --chdir / -- /usr/bin/true; then
            pass "direct lower=/ overlay"
        else
            fail "direct lower=/ overlay"
        fi
    else
        if unshare --mount --propagation private \
            bwrap --die-with-parent --ro-bind / / --proc /proc --dev /dev \
            --overlay-src / --overlay "$OVER/upper" "$OVER/work" / \
            --chdir / -- /usr/bin/true; then
            pass "direct lower=/ overlay"
        else
            fail "direct lower=/ overlay"
        fi
    fi
    if ! remove_tree "$OVER"; then
        fail "overlay cleanup: $OVER"
    fi
    OVER=""
fi
CHECK_DETAIL[direct-root-overlay]="lower=/ in initial userns"
complete_check

# ---------------------------------------------------------------------------
# Build the test binary as the invoking user.
# ---------------------------------------------------------------------------
step "build test binary as the invoking user"
begin_check rust-build
BUILD_LOG="$LOG_DIR/build.log"
BUILD_OK=0
if cargo test --lib --no-run --message-format=json >"$BUILD_LOG" 2>&1; then
    pass "cargo test --lib --no-run"
    BUILD_OK=1
else
    fail "cargo test --lib --no-run"
    echo "--- build log ---"
    cat "$BUILD_LOG" || true
fi
TEST_BIN=""
if [ "$BUILD_OK" -eq 1 ]; then
    TEST_BIN="$(python3 - "$BUILD_LOG" <<'PY'
import json, sys
path = sys.argv[1]
found = []
try:
    lines = open(path, encoding="utf-8", errors="replace")
except OSError:
    lines = []
for line in lines:
    line = line.strip()
    if not line.startswith("{"):
        continue
    try:
        value = json.loads(line)
    except Exception:
        continue
    if value.get("reason") != "compiler-artifact":
        continue
    target = value.get("target", {})
    if "lib" in target.get("kind", []) and value.get("executable"):
        found.append(value["executable"])
print(found[-1] if found else "")
PY
)"
fi
echo "test binary: ${TEST_BIN:-<not found>}"
if [ -z "$TEST_BIN" ] || [ ! -x "$TEST_BIN" ]; then
    fail "lib test binary was not found"
fi
CHECK_DETAIL[rust-build]="lib tests"
complete_check

# ---------------------------------------------------------------------------
# Process-group freeze tests, run as the invoking user.
# ---------------------------------------------------------------------------
step "process-group freeze tests (invoking user)"
begin_check process-freeze
if [ -z "$TEST_BIN" ] || [ ! -x "$TEST_BIN" ]; then
    fail "test binary missing; process tests were not run"
else
    PROCESS_COMBINED="$LOG_DIR/process-combined.log"
    : >"$PROCESS_COMBINED"
    for filter in 'runtime::process::tests::' 'runtime::backends::filesystem::tests::process::'; do
        EXPECTED_TESTS=()
        for test_name in "${PROCESS_TESTS[@]}"; do
            if [[ "$test_name" == "$filter"* ]]; then
                EXPECTED_TESTS+=("$test_name")
            fi
        done
        GROUP_LOG="$LOG_DIR/process-${#EXPECTED_TESTS[@]}"
        LIST_STATUS=0
        "$TEST_BIN" "$filter" --list >"$GROUP_LOG.list" 2>&1 || LIST_STATUS=$?
        if ! python3 "$TEST_OUTPUT_CHECK" list "$GROUP_LOG.list" "$LIST_STATUS" "${EXPECTED_TESTS[@]}"; then
            fail "process test discovery failed: $filter"
            cat "$GROUP_LOG.list"
            continue
        fi
        RUN_STATUS=0
        "$TEST_BIN" "$filter" --show-output --format=pretty --color=never --test-threads=1 \
            >"$GROUP_LOG.run" 2>&1 || RUN_STATUS=$?
        if COUNTS="$(python3 "$TEST_OUTPUT_CHECK" run "$GROUP_LOG.run" "$RUN_STATUS" "${EXPECTED_TESTS[@]}")"; then
            :
        else
            fail "process test execution failed: $filter"
        fi
        read -r EXECUTED PASSED IGNORED <<<"$COUNTS"
        PROCESS_RUN_COUNT=$((PROCESS_RUN_COUNT + EXECUTED))
        PROCESS_PASS_COUNT=$((PROCESS_PASS_COUNT + PASSED))
        PROCESS_IGNORED_COUNT=$((PROCESS_IGNORED_COUNT + IGNORED))
        cat "$GROUP_LOG.run" >>"$PROCESS_COMBINED"
    done
    cat "$PROCESS_COMBINED"

    AUTO_MECHANISMS=""
    while IFS= read -r marker; do
        [ -n "$marker" ] || continue
        mechanism="${marker#ASCOUT_FREEZE_MECHANISM=}"
        mechanism="${mechanism%% *}"
        test_name="${marker##* test=}"
        if [ "$mechanism" = "none" ]; then
            fail "process backend reported freeze mechanism none"
        fi
        if [ "$test_name" != "process_group_fallback_freezes_and_reports_mechanism" ]; then
            AUTO_MECHANISMS="$AUTO_MECHANISMS
$mechanism"
        fi
    done < <(grep -oE 'ASCOUT_FREEZE_MECHANISM=[a-z_]+ test=[A-Za-z0-9_]+' "$PROCESS_COMBINED" || true)
    AUTO_MECHANISMS="$(printf '%s\n' "$AUTO_MECHANISMS" | sed '/^$/d' | sort -u)"
    AUTO_COUNT="$(printf '%s\n' "$AUTO_MECHANISMS" | sed '/^$/d' | wc -l | tr -d ' ')"
    if [ "$AUTO_COUNT" -eq 1 ]; then
        ACTUAL_FREEZE_MECHANISM="$AUTO_MECHANISMS"
    elif [ "$AUTO_COUNT" -eq 0 ]; then
        fail "no ASCOUT_FREEZE_MECHANISM marker was printed by the auto-selecting process tests"
    else
        fail "process tests reported inconsistent freeze mechanisms: $AUTO_MECHANISMS"
    fi
    if [ "$PROCESS_RUN_COUNT" -ne "${#PROCESS_TESTS[@]}" ]; then
        fail "expected ${#PROCESS_TESTS[@]} process tests, executed $PROCESS_RUN_COUNT"
    fi
    for test_name in "${PROCESS_TESTS[@]}"; do
        short_name="${test_name##*::}"
        MARKER_COUNT="$(grep -cE "^ASCOUT_FREEZE_MECHANISM=(cgroup|process_group) test=${short_name}$" "$PROCESS_COMBINED" || true)"
        if [ "$MARKER_COUNT" -ne 1 ]; then
            fail "expected exactly one valid freeze marker for $short_name, found $MARKER_COUNT"
        fi
    done
    if [ "$CURRENT_FAILED" -eq 0 ]; then
        pass "${#PROCESS_TESTS[@]} process-group tests executed and passed (freeze mechanism: $ACTUAL_FREEZE_MECHANISM)"
    fi
fi
CHECK_DETAIL[process-freeze]="${PROCESS_RUN_COUNT}/${#PROCESS_TESTS[@]} executed; ${PROCESS_PASS_COUNT}/${#PROCESS_TESTS[@]} passed; ${PROCESS_IGNORED_COUNT} ignored; mechanism=${ACTUAL_FREEZE_MECHANISM}"
complete_check

# ---------------------------------------------------------------------------
# Privileged Rust tests, run as root.
# ---------------------------------------------------------------------------
step "privileged Rust tests"
begin_check privileged-rust
if [ -z "$TEST_BIN" ] || [ ! -x "$TEST_BIN" ]; then
    fail "test binary missing; privileged tests were not run"
elif [ "$LAYOUT_OK" -ne 1 ]; then
    fail "layout is invalid; privileged tests were not run"
elif [ "$(id -u)" -ne 0 ] && [ -z "$SUDO" ]; then
    fail "privileged Rust tests need root or sudo"
else
    run_priv() {
        if [ -n "$SUDO" ]; then
            $SUDO env ASCOUT_PRIV_TESTS=1 \
                ASCOUT_PRIV_STATE_DIR="$STATE" \
                ASCOUT_PRIV_WORKSPACE_DIR="$WORKSPACE" \
                "$TEST_BIN" "$@"
        else
            env ASCOUT_PRIV_TESTS=1 \
                ASCOUT_PRIV_STATE_DIR="$STATE" \
                ASCOUT_PRIV_WORKSPACE_DIR="$WORKSPACE" \
                "$TEST_BIN" "$@"
        fi
    }
    PRIV_LIST_LOG="$LOG_DIR/tests.list"
    PRIV_RUN_LOG="$LOG_DIR/tests.run"
    EXPECTED_TESTS=()
    for test_name in "${PRIV_TESTS[@]}"; do
        EXPECTED_TESTS+=("${PRIV_PREFIX}${test_name}")
    done
    LIST_STATUS=0
    run_priv "$PRIV_FILTER" --list >"$PRIV_LIST_LOG" 2>&1 || LIST_STATUS=$?
    cat "$PRIV_LIST_LOG"
    if ! python3 "$TEST_OUTPUT_CHECK" list "$PRIV_LIST_LOG" "$LIST_STATUS" "${EXPECTED_TESTS[@]}"; then
        fail "privileged Rust test discovery failed"
    else
        RUN_STATUS=0
        run_priv "$PRIV_FILTER" --show-output --format=pretty --color=never --test-threads=1 \
            >"$PRIV_RUN_LOG" 2>&1 || RUN_STATUS=$?
        cat "$PRIV_RUN_LOG"
        if COUNTS="$(python3 "$TEST_OUTPUT_CHECK" run "$PRIV_RUN_LOG" "$RUN_STATUS" "${EXPECTED_TESTS[@]}")"; then
            :
        else
            fail "privileged Rust test execution failed"
        fi
        read -r PRIV_RUN_COUNT PRIV_PASS_COUNT PRIV_IGNORED_COUNT <<<"$COUNTS"
        if [ "$CURRENT_FAILED" -eq 0 ]; then
            pass "${#PRIV_TESTS[@]} privileged Rust tests executed and passed"
        fi
    fi
fi
CHECK_DETAIL[privileged-rust]="${PRIV_RUN_COUNT}/${#PRIV_TESTS[@]} executed; ${PRIV_PASS_COUNT}/${#PRIV_TESTS[@]} passed; ${PRIV_IGNORED_COUNT} ignored"
complete_check

exit "$SUMMARY_FAIL"
