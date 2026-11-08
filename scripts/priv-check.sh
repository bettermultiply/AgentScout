#!/usr/bin/env bash
# Privileged validation for the AScout filesystem backend.
#
# Run this as a normal user.  Steps that need privilege use sudo internally;
# cargo and the test binary are built/run as the invoking user.
set -uxo pipefail
cd "$(dirname "$0")/.."

SUMMARY_FAIL=0
step() {
    echo
    echo "===== $* ====="
}
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; SUMMARY_FAIL=1; }
skip() { echo "SKIP: $*"; }

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
    $SUDO chown -R "$(id -u):$(id -g)" "$WORK" 2>/dev/null || true

    step "btrfs readonly snapshots"
    echo base > "$WORK/sub/base.txt"
    if $SUDO btrfs subvolume snapshot -r "$WORK/sub" "$WORK/snap1"; then
        pass "first readonly snapshot"
    else
        fail "first readonly snapshot"
    fi
    $SUDO chown -R "$(id -u):$(id -g)" "$WORK" 2>/dev/null || true
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
    $SUDO chown -R "$(id -u):$(id -g)" "$WORK" 2>/dev/null || true

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
        echo "--- send stderr ---"; cat "$WORK/send.err" || true
        echo "--- receive stderr ---"; cat "$WORK/receive.err" || true
    fi

    step "btrfs subvolume delete"
    if $SUDO btrfs subvolume delete "$WORK/snap2" "$WORK/snap1" "$WORK/sub"; then
        pass "strict subvolume delete"
    else
        fail "strict subvolume delete"
    fi
    rm -rf "$WORK"
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

# ---------------------------------------------------------------------------
# Build the privileged Rust tests as the invoking user, then run as root.
# ---------------------------------------------------------------------------
step "build test binary as the invoking user"
BUILD_LOG="$(mktemp "${TMPDIR:-/tmp}/ascout-priv-build.XXXXXX")"
if cargo test --lib --no-run --message-format=json >"$BUILD_LOG" 2>&1; then
    pass "cargo test --lib --no-run"
else
    fail "cargo test --lib --no-run"
    echo "--- build log ---"
    cat "$BUILD_LOG"
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
elif [ "$(id -u)" -eq 0 ]; then
    ASCOUT_PRIV_TESTS=1 ASCOUT_PRIV_STATE_DIR="$STATE" \
        "$TEST_BIN" filesystem::tests::priv_ --nocapture || fail "priv_ tests"
elif [ -z "$SUDO" ]; then
    skip "privileged Rust tests need root or sudo"
else
    $SUDO env ASCOUT_PRIV_TESTS=1 ASCOUT_PRIV_STATE_DIR="$STATE" \
        "$TEST_BIN" filesystem::tests::priv_ --nocapture || fail "priv_ tests"
fi

echo
echo "==============================="
if [ "$SUMMARY_FAIL" -eq 0 ]; then
    echo "SUMMARY: ALL PRIVILEGED CHECKS PASSED"
else
    echo "SUMMARY: PRIVILEGED CHECKS FAILED"
fi
exit "$SUMMARY_FAIL"
