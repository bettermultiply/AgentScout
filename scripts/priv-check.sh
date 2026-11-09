#!/usr/bin/env bash
# Privileged validation for the AScout filesystem backend.
#
# Run this as a normal user. Steps that need privilege use sudo internally;
# cargo and the test binary are built as the invoking user.
set -euxo pipefail
cd "$(dirname "$0")/.."

SUMMARY_FAIL=0
EXPECTED_TESTS=2
LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ascout-priv-check.XXXXXX")"
exec > >(tee "$LOG_DIR/check.log") 2>&1
echo "diagnostics retained in $LOG_DIR"
WORK=""
OVER=""
finish() {
    local status=$?
    trap - EXIT
    if [ -n "$OVER" ]; then
        rm -rf "$OVER" || { echo "FAIL: overlay cleanup: $OVER"; status=1; }
    fi
    # Do not recursively remove subvolumes after a failed strict delete.
    if [ -n "$WORK" ]; then
        echo "FAIL: raw checks incomplete; retained for cleanup: $WORK"
        status=1
    fi
    if [ "$status" -ne 0 ] || [ "$SUMMARY_FAIL" -ne 0 ]; then
        echo "SUMMARY: PRIVILEGED CHECKS FAILED (diagnostics: $LOG_DIR)"
        exit 1
    fi
    echo "SUMMARY: ALL PRIVILEGED CHECKS PASSED (diagnostics: $LOG_DIR)"
}
trap finish EXIT
step() {
    echo
    echo "===== $* ====="
}
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; SUMMARY_FAIL=1; }
skip() { echo "SKIP: $*"; SUMMARY_FAIL=1; }

step "environment"
id
command -v cargo || true
command -v bwrap || true
command -v btrfs || true
command -v findmnt || true
command -v sudo || true

# ---------------------------------------------------------------------------
# Choose a writable directory on a btrfs mount.
# ---------------------------------------------------------------------------
step "select btrfs state directory"
if [ -n "${ASCOUT_PRIV_STATE_DIR:-}" ]; then
    STATE="$ASCOUT_PRIV_STATE_DIR"
    echo "using ASCOUT_PRIV_STATE_DIR=$STATE"
else
    STATE=""
    while read -r target fstype options _; do
        [ "$fstype" = "btrfs" ] || continue
        case ",$options," in
            *,rw,*) ;;
            *) continue ;;
        esac
        [ -w "$target" ] || continue
        STATE="$target/.ascout-priv-check"
        break
    done < <(findmnt -rn -o TARGET,FSTYPE,OPTIONS)
    if [ -z "$STATE" ]; then
        echo "NO_WRITABLE_BTRFS_MOUNT: set ASCOUT_PRIV_STATE_DIR to a directory on btrfs"
        SUMMARY_FAIL=1
    fi
fi

BTRFS_AVAILABLE=0
if [ -n "$STATE" ]; then
    mkdir -p "$STATE"
    echo "state directory: $STATE"
    stat -f -c 'state fs type: %T' "$STATE"
    if [ "$(stat -f -c %T "$STATE" 2>/dev/null)" = "btrfs" ]; then
        BTRFS_AVAILABLE=1
        pass "state directory is btrfs"
    else
        fail "state directory is not btrfs"
    fi
fi

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
    if command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
    else
        echo "NEEDS_ROOT: sudo is unavailable; privileged checks cannot run"
    fi
fi
if [ "$(id -u)" -eq 0 ]; then
    SUDO=""
fi

# ---------------------------------------------------------------------------
# Raw btrfs capability checks.
# ---------------------------------------------------------------------------
if [ "$BTRFS_AVAILABLE" -eq 1 ]; then
    WORK="$STATE/raw-$$"
    rm -rf "$WORK"
    mkdir -p "$WORK"

    step "btrfs subvolume create"
    if $SUDO btrfs subvolume create "$WORK/sub"; then
        pass "btrfs subvolume create"
    else
        fail "btrfs subvolume create"
    fi
    $SUDO chown -R "$(id -u):$(id -g)" "$WORK/sub"

    step "btrfs readonly snapshots"
    echo base > "$WORK/sub/base.txt"
    if $SUDO btrfs subvolume snapshot -r "$WORK/sub" "$WORK/snap1"; then
        pass "first readonly snapshot"
    else
        fail "first readonly snapshot"
    fi
    echo changed > "$WORK/sub/modified.txt"
    rm -f "$WORK/sub/base.txt"
    mkdir -p "$WORK/sub/dir"
    echo nested > "$WORK/sub/dir/nested.txt"
    mv "$WORK/sub/dir" "$WORK/sub/renamed"
    if $SUDO btrfs subvolume snapshot -r "$WORK/sub" "$WORK/snap2"; then
        pass "second readonly snapshot"
    else
        fail "second readonly snapshot"
    fi

    step "btrfs send -p | btrfs receive --dump"
    if $SUDO btrfs send -p "$WORK/snap1" "$WORK/snap2" 2>"$WORK/send.err" \
        | $SUDO btrfs receive --dump 2>"$WORK/receive.err" >"$WORK/dump.txt"; then
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
        echo "--- send stderr ---"; cat "$WORK/send.err"
        echo "--- receive stderr ---"; cat "$WORK/receive.err"
    fi
    cp "$WORK/send.err" "$WORK/receive.err" "$WORK/dump.txt" "$LOG_DIR/"

    step "btrfs subvolume delete"
    if $SUDO btrfs subvolume delete "$WORK/snap2" "$WORK/snap1" "$WORK/sub"; then
        pass "strict subvolume delete"
        rm -rf "$WORK"
        WORK=""
    else
        fail "strict subvolume delete"
    fi
else
    skip "btrfs raw checks (no btrfs state directory)"
fi

# ---------------------------------------------------------------------------
# Direct lower=/ overlay.  This needs the initial user namespace (root).
# ---------------------------------------------------------------------------
step "direct lower=/ overlay in the initial user namespace"
OVER="${STATE:-${TMPDIR:-/tmp}}/overlay-$$"
mkdir -p "$OVER/upper" "$OVER/work"
if [ "$(id -u)" -ne 0 ] && [ -z "$SUDO" ]; then
    skip "direct lower=/ needs root/initial userns (no sudo)"
elif [ "$(id -u)" -ne 0 ]; then
    echo "NEEDS_ROOT: rerunning direct lower=/ check through sudo"
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
rm -rf "$OVER"
OVER=""

# ---------------------------------------------------------------------------
# Build the privileged Rust tests as the invoking user, then run as root.
# ---------------------------------------------------------------------------
step "build test binary as the invoking user"
BUILD_LOG="$LOG_DIR/build.log"
if cargo test --lib --no-run --message-format=json >"$BUILD_LOG" 2>&1; then
    pass "cargo test --lib --no-run"
else
    fail "cargo test --lib --no-run"
    echo "--- build log ---"
    cat "$BUILD_LOG"
    exit 1
fi
TEST_BIN="$(python3 - "$BUILD_LOG" <<'PY'
import json, sys
path = sys.argv[1]
found = []
for line in open(path, encoding="utf-8", errors="replace"):
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
echo "test binary: ${TEST_BIN:-<not found>}"

step "run privileged Rust tests"
if [ -z "$TEST_BIN" ] || [ ! -x "$TEST_BIN" ]; then
    fail "privileged test binary was not found"
elif [ "$SUMMARY_FAIL" -ne 0 ]; then
    fail "preconditions failed; privileged Rust tests were not run"
elif [ "$(id -u)" -eq 0 ]; then
    TEST_FILTER='runtime::backends::filesystem::tests::privileged::priv_'
    TEST_LIST_LOG="$LOG_DIR/tests.list"
    if ! ASCOUT_PRIV_TESTS=1 ASCOUT_PRIV_STATE_DIR="$STATE" \
        "$TEST_BIN" "$TEST_FILTER" --list >"$TEST_LIST_LOG" 2>&1; then
        cat "$TEST_LIST_LOG"
        fail "listing privileged Rust tests"
    else
        cat "$TEST_LIST_LOG"
        TEST_COUNT="$(grep -Ec '^runtime::backends::filesystem::tests::privileged::priv_[^:]*: test$' "$TEST_LIST_LOG" || true)"
        if [ "$TEST_COUNT" -ne "$EXPECTED_TESTS" ] || \
            ! grep -qx "$TEST_FILTER"'btrfs_fork_diff_accept_discard: test' "$TEST_LIST_LOG" || \
            ! grep -qx "$TEST_FILTER"'direct_root_overlay_is_available_in_initial_userns: test' "$TEST_LIST_LOG"; then
            fail "expected $EXPECTED_TESTS privileged tests, found $TEST_COUNT"
        elif grep -q '^SKIP' "$TEST_LIST_LOG"; then
            fail "privileged test listing reported SKIP"
        elif ! ASCOUT_PRIV_TESTS=1 ASCOUT_PRIV_STATE_DIR="$STATE" \
            "$TEST_BIN" "$TEST_FILTER" --nocapture >"$TEST_LIST_LOG.run" 2>&1; then
            cat "$TEST_LIST_LOG.run"
            fail "privileged Rust tests"
        else
            cat "$TEST_LIST_LOG.run"
            RUN_COUNT="$(grep -Ec '^test runtime::backends::filesystem::tests::privileged::priv_[^ ]+ \.\.\. ok$' "$TEST_LIST_LOG.run" || true)"
            if [ "$RUN_COUNT" -ne "$EXPECTED_TESTS" ] || grep -q '^SKIP' "$TEST_LIST_LOG.run"; then
                fail "expected $EXPECTED_TESTS privileged tests to execute without SKIP"
            else
                pass "$EXPECTED_TESTS privileged Rust tests executed"
            fi
        fi
    fi
elif [ -z "$SUDO" ]; then
    fail "privileged Rust tests need root or sudo"
else
    TEST_FILTER='runtime::backends::filesystem::tests::privileged::priv_'
    TEST_LIST_LOG="$LOG_DIR/tests.list"
    if ! $SUDO env ASCOUT_PRIV_TESTS=1 ASCOUT_PRIV_STATE_DIR="$STATE" \
        "$TEST_BIN" "$TEST_FILTER" --list >"$TEST_LIST_LOG" 2>&1; then
        cat "$TEST_LIST_LOG"
        fail "listing privileged Rust tests"
    else
        cat "$TEST_LIST_LOG"
        TEST_COUNT="$(grep -Ec '^runtime::backends::filesystem::tests::privileged::priv_[^:]*: test$' "$TEST_LIST_LOG" || true)"
        if [ "$TEST_COUNT" -ne "$EXPECTED_TESTS" ] || \
            ! grep -qx "$TEST_FILTER"'btrfs_fork_diff_accept_discard: test' "$TEST_LIST_LOG" || \
            ! grep -qx "$TEST_FILTER"'direct_root_overlay_is_available_in_initial_userns: test' "$TEST_LIST_LOG"; then
            fail "expected $EXPECTED_TESTS privileged tests, found $TEST_COUNT"
        elif ! $SUDO env ASCOUT_PRIV_TESTS=1 ASCOUT_PRIV_STATE_DIR="$STATE" \
            "$TEST_BIN" "$TEST_FILTER" --nocapture >"$TEST_LIST_LOG.run" 2>&1; then
            cat "$TEST_LIST_LOG.run"
            fail "privileged Rust tests"
        else
            cat "$TEST_LIST_LOG.run"
            RUN_COUNT="$(grep -Ec '^test runtime::backends::filesystem::tests::privileged::priv_[^ ]+ \.\.\. ok$' "$TEST_LIST_LOG.run" || true)"
            if [ "$RUN_COUNT" -ne "$EXPECTED_TESTS" ] || grep -q '^SKIP' "$TEST_LIST_LOG.run"; then
                fail "expected $EXPECTED_TESTS privileged tests to execute without SKIP"
            else
                pass "$EXPECTED_TESTS privileged Rust tests executed"
            fi
        fi
    fi
fi

exit "$SUMMARY_FAIL"
