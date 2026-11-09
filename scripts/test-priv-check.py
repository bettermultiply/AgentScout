#!/usr/bin/env python3
"""Unprivileged regression checks; every privileged command is a local fake."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

SCRIPT = Path(__file__).with_name("priv-check.sh")
PREFIX = "runtime::backends::filesystem::tests::privileged::"
NAMES = [PREFIX + "priv_btrfs_fork_diff_accept_discard",
         PREFIX + "priv_direct_root_overlay_is_available_in_initial_userns"]
DISPATCH = r'''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
mode = os.environ['FAKE_MODE']
if name == 'id': print('1000' if mode == 'nonroot' else '0')
elif name == 'stat': print('ext4' if mode == 'nonbtrfs' else 'btrfs')
elif name == 'cargo':
    print(json.dumps({'reason':'compiler-artifact','target':{'kind':['lib']},'executable':os.environ['FAKE_BIN']}))
    sys.exit(1 if mode == 'build-fail' else 0)
elif name == 'fake-tests':
    with open(os.environ['FAKE_CALLS'], 'a') as f: f.write(' '.join(args)+'\n')
    names = json.loads(os.environ['FAKE_NAMES'])
    matched = [n for n in names if args[0] in n]
    if mode == 'zero': matched = []
    if '--list' in args:
        for n in matched: print(n+': test')
    else:
        if mode == 'skip': print('SKIP required capability')
        print('running %d tests' % len(matched))
        for n in matched: print('test '+n+' ... ok')
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
        for name in ["id", "stat", "cargo", "btrfs", "unshare", "bwrap", "chown", "rm", "sudo", "fake-tests"]:
            (fake / name).symlink_to(dispatcher)
        env = dict(os.environ, PATH=str(fake)+":"+os.environ["PATH"], FAKE_MODE=mode,
                   FAKE_BIN=str(fake / "fake-tests"), FAKE_NAMES=json.dumps(NAMES),
                   FAKE_READONLY=str(root / "readonly"),
                   FAKE_CALLS=str(root / "calls"), ASCOUT_PRIV_STATE_DIR=str(root / "state"),
                   TMPDIR=str(log_dir))
        result = subprocess.run(["bash", str(SCRIPT)], env=env, text=True, capture_output=True)
        log = log_dir / ("priv-check-"+mode+".log")
        log.write_text(result.stdout+result.stderr)
        calls = (root / "calls").read_text() if (root / "calls").exists() else ""
        return result, calls, log

if __name__ == "__main__":
    import sys
    configured_logs = os.environ.get("ASCOUT_PRIV_CHECK_TEST_LOG_DIR")
    log_dir = Path(configured_logs).resolve() if configured_logs else Path(tempfile.mkdtemp(prefix="priv-check-regression-"))
    log_dir.mkdir(parents=True, exist_ok=True)
    print(f"diagnostics retained in {log_dir}")
    modes = ["success", "zero", "skip", "build-fail", "nonroot", "nonbtrfs", "cleanup-fail"]
    for mode in modes:
        result, calls, log = run(mode, log_dir)
        if "--reproduce" in sys.argv:
            print(f"{mode}: exit={result.returncode}, all_passed={'ALL PRIVILEGED CHECKS PASSED' in result.stdout}, calls={calls.strip()!r}; {log}")
            continue
        if mode in ("success", "nonroot"):
            assert result.returncode == 0, (mode, log)
            assert calls.count("runtime::backends::filesystem::tests::privileged::priv_") == 2, (mode, calls)
        else:
            assert result.returncode != 0, (mode, log)
            assert "ALL PRIVILEGED CHECKS PASSED" not in result.stdout, (mode, log)
        if mode == "build-fail":
            assert not calls, calls
        print(f"PASS {mode}: {log}")
