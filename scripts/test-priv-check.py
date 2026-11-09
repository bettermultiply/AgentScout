#!/usr/bin/env python3
"""Unprivileged regression checks; every privileged command is a local fake."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

SCRIPT = Path(__file__).with_name("priv-check.sh")
PRIV_PREFIX = "runtime::backends::filesystem::tests::privileged::"
PRIV_TESTS = [
    "priv_btrfs_chain_layout",
    "priv_btrfs_flatten_layout",
    "priv_btrfs_mixed_generations",
    "priv_direct_root_overlay_is_available_in_initial_userns",
]
PROCESS_TESTS = [
    "runtime::process::tests::terminate_uses_cgroup_or_group_kill_and_cleanup_removes_cgroup",
    "runtime::process::tests::failed_launch_cleans_process_pending_status_and_cgroup",
    "runtime::process::tests::stop_reports_cgroup_kill_write_failure",
    "runtime::process::tests::stop_unknown_or_exited_process_is_not_an_error",
    "runtime::process::tests::exited_process_is_reported_as_not_running",
    "runtime::backends::filesystem::tests::process::process_freeze_thaw_stops_and_resumes_writer",
    "runtime::backends::filesystem::tests::process::process_group_fallback_freezes_and_reports_mechanism",
]
NAMES = [PRIV_PREFIX + name for name in PRIV_TESTS] + PROCESS_TESTS
CHECKS = [
    "environment",
    "layout",
    "btrfs-raw",
    "direct-root-overlay",
    "rust-build",
    "process-freeze",
    "privileged-rust",
]

DISPATCH = r'''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
mode = os.environ['FAKE_MODE']
if name == 'id':
    if '-u' in args or '-g' in args:
        print('1000' if mode == 'nonroot' else '0')
    else:
        print('uid=1000 gid=1000' if mode == 'nonroot' else 'uid=0 gid=0 root')
elif name == 'stat':
    print('ext4' if mode == 'nonbtrfs' else 'btrfs')
elif name == 'findmnt':
    option = args[args.index('-o') + 1] if '-o' in args else ''
    if '-T' not in args:
        if mode == 'no-layout':
            print('/ ext4 rw')
            print('/tmp tmpfs rw')
        else:
            print('/ btrfs rw')
            print('/tmp tmpfs rw')
            print('/home btrfs rw')
    else:
        target = args[args.index('-T') + 1]
        state_root = os.environ.get('FAKE_STATE_ROOT', '')
        workspace_root = os.environ.get('FAKE_WORKSPACE_ROOT', '')
        if state_root and (target == state_root or target.startswith(state_root + '/')):
            mount = os.environ.get('FAKE_STATE_MOUNT', '/')
            fstype = 'btrfs'
        elif workspace_root and (target == workspace_root or target.startswith(workspace_root + '/')):
            mount = os.environ.get('FAKE_WORKSPACE_MOUNT', '/tmp')
            fstype = 'tmpfs'
        elif target == '/tmp' or target.startswith('/tmp/'):
            mount = '/tmp'
            fstype = 'tmpfs'
        elif target == '/home' or target.startswith('/home/'):
            mount = '/home'
            fstype = 'btrfs'
        else:
            mount = '/'
            fstype = 'btrfs'
        if option == 'TARGET,FSTYPE,OPTIONS':
            print(mount + ' ' + fstype + ' rw')
        elif option == 'TARGET,FSTYPE':
            print(mount + ' ' + fstype)
        else:
            print(mount)
elif name == 'cargo':
    print(json.dumps({'reason':'compiler-artifact','target':{'kind':['lib']},'executable':os.environ['FAKE_BIN']}))
    sys.exit(1 if mode == 'build-fail' else 0)
elif name == 'fake-tests':
    with open(os.environ['FAKE_CALLS'], 'a') as f: f.write(' '.join(args)+'\n')
    names = json.loads(os.environ['FAKE_NAMES'])
    matched = [n for n in names if args[0] in n]
    if mode == 'zero' and 'privileged::priv_' in args[0]: matched = []
    if '--list' in args:
        for n in matched: print(n+': test')
    else:
        if mode == 'skip' and 'privileged::priv_' in args[0]:
            print('SKIP required privileged capability')
        print('running %d tests' % len(matched))
        mechanism = {'freeze-none':'none', 'freeze-pg':'process_group'}.get(mode, 'cgroup')
        for n in matched:
            if '::process::' in n:
                short = n.split('::')[-1]
                print('ASCOUT_FREEZE_MECHANISM=%s test=%s' % (mechanism, short))
            print('test '+n+' ... ok')
        print('test result: ok. %d passed; 0 failed; 0 ignored; 0 measured; 0 filtered out' % len(matched))
elif name == 'btrfs':
    if args[:2] == ['subvolume','create']: pathlib.Path(args[2]).mkdir()
    elif args[:2] == ['subvolume','snapshot']:
        shutil.copytree(args[-2],args[-1])
        if '-r' in args:
            with open(os.environ['FAKE_READONLY'], 'a') as f:
                f.write(str(pathlib.Path(args[-1]).resolve())+'\n')
    elif args[:2] == ['subvolume','delete']:
        for p in args[2:]: shutil.rmtree(p)
    elif args[:2] == ['subvolume','show']:
        sys.exit(1)
    elif args[:2] == ['receive','--dump']: print('modified.txt base.txt renamed')
elif name == 'rm':
    if mode == 'cleanup-fail' and any('/overlay-' in a for a in args): sys.exit(1)
    os.execv('/usr/bin/rm', ['rm']+args)
elif name == 'sudo':
    # Never invoke the real sudo, even when exercising the escalation branch.
    os.execvp(args[0], args)
elif name == 'chown':
    registry = pathlib.Path(os.environ['FAKE_READONLY'])
    readonly = [pathlib.Path(p) for p in registry.read_text().splitlines()] if registry.exists() else []
    recursive = '-R' in args
    for arg in args:
        if arg.startswith('-') or ':' in arg: continue
        target = pathlib.Path(arg).resolve()
        for snapshot in readonly:
            if snapshot.exists() and (target == snapshot or snapshot in target.parents or
                                      (recursive and target in snapshot.parents)):
                print(f'chown: {snapshot}: Read-only file system', file=sys.stderr)
                sys.exit(1)
elif name in ('unshare','bwrap'): pass
else: sys.exit('unexpected fake command '+name)
'''


def run(mode, log_dir):
    with tempfile.TemporaryDirectory(prefix="priv-check-fake-") as tmp:
        root = Path(tmp)
        fake = root / "bin"
        fake.mkdir()
        dispatcher = fake / "dispatch"
        dispatcher.write_text(DISPATCH)
        dispatcher.chmod(0o755)
        for name in ["id", "stat", "findmnt", "cargo", "btrfs", "unshare",
                     "bwrap", "chown", "rm", "sudo", "fake-tests"]:
            (fake / name).symlink_to(dispatcher)
        names = list(NAMES)
        if mode == "missing-test":
            names.remove(PRIV_PREFIX + "priv_btrfs_flatten_layout")
        env = dict(
            os.environ,
            PATH=str(fake) + ":" + os.environ["PATH"],
            FAKE_MODE=mode,
            FAKE_BIN=str(fake / "fake-tests"),
            FAKE_NAMES=json.dumps(names),
            FAKE_READONLY=str(root / "readonly"),
            FAKE_CALLS=str(root / "calls"),
            FAKE_STATE_ROOT=str(root / "state"),
            FAKE_WORKSPACE_ROOT=str(root / "workspace"),
            FAKE_STATE_MOUNT="/home" if mode == "overlap-layout" else "/",
            FAKE_WORKSPACE_MOUNT="/tmp",
            TMPDIR=str(log_dir),
        )
        env.pop("ASCOUT_PRIV_STATE_DIR", None)
        env.pop("ASCOUT_PRIV_WORKSPACE_DIR", None)
        if mode != "no-layout":
            env["ASCOUT_PRIV_STATE_DIR"] = str(root / "state")
            env["ASCOUT_PRIV_WORKSPACE_DIR"] = str(root / "workspace")
        result = subprocess.run(
            ["bash", str(SCRIPT)],
            env=env,
            text=True,
            capture_output=True,
            timeout=120,
        )
        log = log_dir / ("priv-check-" + mode + ".log")
        log.write_text(result.stdout + result.stderr)
        calls = (root / "calls").read_text() if (root / "calls").exists() else ""
        return result, calls, log


if __name__ == "__main__":
    import sys
    configured_logs = os.environ.get("ASCOUT_PRIV_CHECK_TEST_LOG_DIR")
    log_dir = Path(configured_logs).resolve() if configured_logs else Path(tempfile.mkdtemp(prefix="priv-check-regression-"))
    log_dir.mkdir(parents=True, exist_ok=True)
    print(f"diagnostics retained in {log_dir}")
    modes = [
        "success",
        "nonroot",
        "freeze-pg",
        "freeze-none",
        "zero",
        "missing-test",
        "skip",
        "build-fail",
        "nonbtrfs",
        "no-layout",
        "overlap-layout",
        "cleanup-fail",
    ]
    success_modes = {"success", "nonroot", "freeze-pg"}
    for mode in modes:
        result, calls, log = run(mode, log_dir)
        output = result.stdout + result.stderr
        if "--reproduce" in sys.argv:
            print(f"{mode}: exit={result.returncode}, "
                  f"all_passed={'ALL PRIVILEGED CHECKS PASSED' in output}, "
                  f"calls={calls.strip()!r}; {log}")
            continue
        if mode in success_modes:
            assert result.returncode == 0, (mode, log)
            assert "ALL PRIVILEGED CHECKS PASSED" in output, (mode, log)
            assert "coverage summary" in output, (mode, log)
            for check in CHECKS:
                assert check in output, (mode, check, log)
            assert "4/4 executed" in output, (mode, log)
            assert "7/7 executed" in output, (mode, log)
            expected_mechanism = "process_group" if mode == "freeze-pg" else "cgroup"
            assert f"mechanism={expected_mechanism}" in output, (mode, log)
            assert calls.count("--list") >= 1, (mode, calls)
            assert calls.count("runtime::backends::filesystem::tests::privileged::priv_") >= 2, (mode, calls)
            assert calls.count("runtime::process::tests::") >= 1, (mode, calls)
            assert calls.count("runtime::backends::filesystem::tests::process::") >= 1, (mode, calls)
        else:
            assert result.returncode != 0, (mode, log)
            assert "ALL PRIVILEGED CHECKS PASSED" not in output, (mode, log)
            assert "coverage summary" in output, (mode, log)
            if mode == "build-fail":
                assert not calls, calls
            if mode == "skip":
                assert "SKIP required privileged capability" in output, (mode, log)
            if mode == "freeze-none":
                assert "freeze mechanism none" in output, (mode, log)
            if mode == "missing-test":
                assert "expected privileged test is missing" in output, (mode, log)
            if mode == "no-layout":
                assert "NO_VALID_STATE_LAYOUT" in output, (mode, log)
            if mode == "overlap-layout":
                assert "inside a mount the backend isolates" in output, (mode, log)
        print(f"PASS {mode}: {log}")
