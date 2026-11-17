#!/usr/bin/env python3
"""A small JSON-lines NLE workload bridge.

One NetHack world lives in this process. Environments have distinct identities
and lifetimes, while ``branch`` retains a reference to the parent's world.
Speculation is staged outside the workload: candidates do not act before
promotion. Save/load reconstructs the saved world, not a world per branch.

Input commands:

  {"id":"1", "op":"establish", "environment":"parent", "seed":7}
  {"id":"2", "op":"branch", "environment":"parent", "child":"candidate-1"}
  {"id":"3", "op":"operate", "environment":"parent", "action":1}
  {"id":"4", "op":"observe", "environment":"parent"}
  {"id":"5", "op":"suspend", "environment":"parent"}
  {"id":"6", "op":"resume", "environment":"parent"}
  {"id":"7", "op":"terminate", "environment":"parent"}
  {"id":"8", "op":"cleanup", "environment":"parent"}
  {"id":"9", "op":"invoke", "environment":"parent", "tool":"nle_action", "arguments":{"action":1}}
  {"id":"10", "op":"invoke", "environment":"parent", "tool":"nethack_guide", "arguments":{"section":"3.3"}}

The bridge deliberately does not accept a hidden-map or state-inspection
command.  Agent permission and Policy admission remain outside this bridge.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import os
import re
import secrets
import sys
import time
import threading
import queue
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class World:
    environment: Any
    observation: dict[str, Any]
    references: int = 1
    state: str = "available"
    closed: bool = False
    initial: dict[str, Any] = field(default_factory=dict)
    actions: list[int] = field(default_factory=list)
    checks: list[str] = field(default_factory=list)


def fingerprint(environment, observation) -> str:
    """Check every public array plus the native RNG state, not just the screen."""
    result = hashlib.sha256()
    for key, value in sorted(observation.items()):
        result.update(key.encode())
        result.update(str(value.dtype).encode())
        result.update(str(value.shape).encode())
        result.update(value.tobytes())
    result.update(json.dumps(environment.unwrapped.get_seeds()).encode())
    return result.hexdigest()


def new_world(gym, env_id, seed=None, initial=None):
    # Installed NLE has no public native save/restore API. Disable reseeding
    # and derive time-dependent effects from the explicit initial RNGs.
    if initial is None:
        core = int(seed) if seed is not None else secrets.randbits(63)
        # None keeps level generation on the core RNG, as seed(core) did
        # before save/restore was added. A separate random lgen seed changes
        # the dungeon even when the configured core seed is unchanged.
        initial = {"env_id": env_id, "seeds": [core, secrets.randbits(63), False, None],
                   "kwargs": {"fix_moon_phase": True}, "nle_version": importlib.metadata.version("nle")}
    if initial["nle_version"] != importlib.metadata.version("nle"):
        raise ValueError("NLE version differs from saved environment")
    environment = gym.make(initial["env_id"], **initial["kwargs"])
    try:
        environment.unwrapped.seed(*initial["seeds"])
        observation, _ = environment.reset()
        config = {"options": environment.unwrapped.nethack.options,
                  "actions": [int(action) for action in environment.unwrapped.actions],
                  "max_episode_steps": environment.unwrapped._max_episode_steps}
        if "config" in initial and initial["config"] != config:
            raise ValueError("NLE configuration differs from saved environment")
        initial["config"] = config
        return World(environment, digest(observation), initial=initial,
                     checks=[fingerprint(environment, observation)])
    except Exception:
        environment.close()
        raise


def step(world, action):
    result = world.environment.step(action)
    world.actions.append(action)
    world.checks.append(fingerprint(world.environment, result[0]))
    world.observation = digest(result[0])
    return result


def restore_world(gym, record):
    world = new_world(gym, record["initial"]["env_id"], initial=record["initial"])
    try:
        if world.checks[0] != record["checks"][0]:
            raise ValueError("NLE initial state verification failed")
        if len(record["checks"]) != len(record["actions"]) + 1:
            raise ValueError("NLE action/check sequence is incomplete")
        for index, action in enumerate(record["actions"]):
            step(world, action)
            if world.checks[-1] != record["checks"][index + 1]:
                raise ValueError(f"NLE replay verification failed at action {index}")
        world.state = record["state"]
        return world
    except Exception:
        world.environment.close()
        raise


def digest(observation: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key in ("chars", "blstats", "message"):
        value = observation.get(key)
        if value is None:
            continue
        if key == "chars":
            result[key] = [row.tobytes().decode("latin-1") for row in value]
        elif key == "message":
            # NLE pads messages with NUL bytes; expose the visible text.
            data = value.tolist() if hasattr(value, "tolist") else list(value)
            result[key] = bytes(data).split(b"\0", 1)[0].decode("latin-1")
        else:
            result[key] = value.tolist()
    return result


def describe(observation: dict[str, Any]) -> str:
    """Render the public NLE observation for the Agent tool channel."""
    lines: list[str] = []
    stats = observation.get("blstats") or []
    if stats:
        values = {index: stats[index] for index in (0, 1, 10, 11, 12, 20, 21) if index < len(stats)}
        parts: list[str] = []
        if 0 in values and 1 in values:
            parts.append(f"position=({values[0]},{values[1]})")
        if 10 in values and 11 in values:
            parts.append(f"hp={values[10]}/{values[11]}")
        if 12 in values:
            parts.append(f"depth={values[12]}")
        if 20 in values:
            parts.append(f"turn={values[20]}")
        if 21 in values:
            parts.append(f"hunger={values[21]}")
        if parts:
            lines.append(" ".join(parts))
    message = observation.get("message")
    if isinstance(message, list):
        message = bytes(message).split(b"\0", 1)[0].decode("latin-1")
    if isinstance(message, str) and message:
        lines.append(f"message: {message}")
    rows = [str(row).rstrip() for row in observation.get("chars") or []]
    nonempty = [index for index, row in enumerate(rows) if row]
    if nonempty:
        lines.append("map:")
        lines.extend(rows[nonempty[0] : nonempty[-1] + 1])
    return "\n".join(lines)


def guide_sections(text: str) -> list[tuple[int, str, str]]:
    pattern = re.compile(r"^\s*(\d+(?:\.\d+)*)\.\s+(.+?)\s*$", re.MULTILINE)
    sections: list[tuple[int, str, str]] = []
    top_level = 0
    for match in pattern.finditer(text):
        number = match.group(1)
        head = int(number.split(".")[0])
        # Ignore numbered prose in the credits and option descriptions that
        # happens to look like a heading.
        if head < top_level:
            continue
        top_level = head
        sections.append((match.start(), number, match.group(2)))
    return sections


def guide_text(text: str, section: str | None) -> str:
    sections = guide_sections(text)
    if not section:
        return "NetHack 3.6.6 Guidebook\n\n" + "\n".join(
            f"{number}  {title}" for _, number, title in sections
        )
    for index, (start, number, _title) in enumerate(sections):
        if number != section:
            continue
        level = len(number.split("."))
        end = len(text)
        for next_start, next_number, _next_title in sections[index + 1 :]:
            if len(next_number.split(".")) <= level:
                end = next_start
                break
        return text[start:end].strip()
    raise ValueError(f"unknown Guidebook section: {section}")


def reply(output, request_id: Any, **payload: Any) -> None:
    output.write(json.dumps({"id": request_id, **payload}, separators=(",", ":")) + "\n")
    output.flush()


def require(worlds: dict[str, World], name: str) -> World:
    try:
        return worlds[name]
    except KeyError:
        raise ValueError(f"unknown environment: {name}")


def release(worlds: dict[str, World], name: str) -> None:
    world = worlds.pop(name)
    world.references -= 1
    if world.references <= 0 and not world.closed:
        world.closed = True
        world.environment.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--env-id", default="NetHackScore-v0")
    parser.add_argument("--delay", type=float, default=0.0)
    parser.add_argument("--guidebook", default="examples/nle/Guidebook.txt")
    args = parser.parse_args()

    guidebook = Path(args.guidebook)
    guidebook_text = guidebook.read_text(encoding="utf-8") if guidebook.is_file() else ""

    try:
        import gymnasium as gym
        import nle  # noqa: F401
    except ImportError as error:
        print(json.dumps({"ok": False, "error": f"NLE unavailable: {error}"}))
        return 2

    worlds: dict[str, World] = {}
    environment_states: dict[str, str] = {}
    requests = queue.Queue()
    cancellation = {}
    lock = threading.Lock()

    def receive():
        for line in sys.stdin:
            request = json.loads(line)
            with lock:
                if request.get("op") == "cancel":
                    event = cancellation.get(request.get("id"))
                    if event is not None:
                        event.set()
                    continue
                event = threading.Event()
                cancellation[request.get("id")] = event
            requests.put((request, event))
        requests.put(None)

    threading.Thread(target=receive, daemon=True).start()
    while True:
        item = requests.get()
        if item is None:
            break
        request, cancelled = item
        request_id = request.get("id")
        try:
            operation = request.get("op")
            name = request.get("environment")
            if operation == "establish":
                if name in worlds:
                    raise ValueError("environment already exists")
                worlds[name] = new_world(gym, args.env_id, request.get("seed"))
                environment_states[name] = "available"
                reply(sys.stdout, request_id, ok=True, state="available", observation=worlds[name].observation)
            elif operation == "branch":
                world = require(worlds, name)
                child = request["child"]
                if child in worlds:
                    raise ValueError("child environment already exists")
                world.references += 1
                worlds[child] = world
                environment_states[child] = "available"
                reply(sys.stdout, request_id, ok=True, state=world.state, child=child)
            elif operation == "operate":
                world = require(worlds, name)
                if environment_states.get(name) != "available":
                    raise ValueError("environment is not available")
                action = request["action"]
                if (
                    isinstance(action, bool)
                    or not isinstance(action, int)
                    or not 0 <= action < world.environment.action_space.n
                ):
                    raise ValueError("action is outside the NLE action space")
                if cancelled.wait(max(0, args.delay)):
                    reply(sys.stdout, request_id, ok=False, cancelled=True)
                    continue
                observation, reward, terminated, truncated, _ = step(world, action)
                world.observation = digest(observation)
                reply(
                    sys.stdout,
                    request_id,
                    ok=True,
                    operation="operate",
                    observation=world.observation,
                    reward=reward,
                    terminated=terminated,
                    truncated=truncated,
                )
            elif operation == "invoke":
                tool = request.get("tool")
                arguments = request.get("arguments") or {}
                if tool == "nle_action":
                    world = require(worlds, name)
                    if environment_states.get(name) != "available":
                        raise ValueError("environment is not available")
                    action = arguments.get("action")
                    if (
                        isinstance(action, bool)
                        or not isinstance(action, int)
                        or not 0 <= action < world.environment.action_space.n
                    ):
                        raise ValueError("action is outside the NLE action space")
                    before = world.observation
                    observation, _reward, _terminated, _truncated, _ = step(world, action)
                    after = digest(observation)
                    world.observation = after
                    reply(
                        sys.stdout,
                        request_id,
                        ok=True,
                        result=describe(after),
                        evidence={
                            "action": action,
                            "before": before,
                            "after": after,
                            "is_error": False,
                        },
                    )
                elif tool == "nethack_guide":
                    if not guidebook_text:
                        raise ValueError(f"Guidebook not found: {args.guidebook}")
                    reply(
                        sys.stdout,
                        request_id,
                        ok=True,
                        result=guide_text(guidebook_text, arguments.get("section")),
                        evidence={"is_error": False},
                    )
                else:
                    raise ValueError(f"unknown NLE tool: {tool}")
            elif operation == "promote":
                world = require(worlds, name)
                # NLE has one shared world, so promotion has no environment-side
                # state change. Keep the generic lifecycle operation explicit.
                reply(sys.stdout, request_id, ok=True, state=environment_states[name])
            elif operation == "observe":
                world = require(worlds, name)
                reply(sys.stdout, request_id, ok=True, state=environment_states[name], observation=world.observation)
            elif operation in {"suspend", "resume"}:
                world = require(worlds, name)
                if environment_states[name] not in {"available", "suspended"}:
                    raise ValueError("environment is not available for suspend/resume")
                environment_states[name] = "suspended" if operation == "suspend" else "available"
                reply(sys.stdout, request_id, ok=True, state=environment_states[name])
            elif operation == "save":
                unique = list({id(world): world for world in worlds.values()}.values())
                records = [{"initial": world.initial, "actions": world.actions,
                            "checks": world.checks, "state": world.state} for world in unique]
                record = {"version": 1, "worlds": records,
                          "environment_states": environment_states,
                          "environments": {key: unique.index(world) for key, world in worlds.items()}}
                # Verify reconstruction before replacing the previous save.
                for saved in records:
                    probe = restore_world(gym, saved)
                    probe.environment.close()
                path = Path(request["path"])
                temporary = path.with_name(path.name + ".tmp")
                with temporary.open("x") as output:
                    json.dump(record, output)
                    output.flush()
                    os.fsync(output.fileno())
                os.replace(temporary, path)
                reply(sys.stdout, request_id, ok=True)
            elif operation == "load":
                if worlds:
                    raise ValueError("load requires an empty bridge")
                record = json.loads(Path(request["path"]).read_text())
                if record["version"] != 1:
                    raise ValueError("unsupported NLE save version")
                restored = []
                try:
                    for saved in record["worlds"]:
                        restored.append(restore_world(gym, saved))
                    names = {key: restored[index] for key, index in record["environments"].items()}
                    for world in restored:
                        world.references = sum(value is world for value in names.values())
                        if not world.references:
                            raise ValueError("unreferenced saved NLE world")
                    worlds.update(names)
                    environment_states.update(record.get("environment_states", {}))
                except Exception:
                    for world in restored:
                        world.environment.close()
                    raise
                reply(sys.stdout, request_id, ok=True, states=environment_states)
            elif operation == "terminate":
                world = require(worlds, name)
                release(worlds, name)
                environment_states[name] = "terminated"
                reply(sys.stdout, request_id, ok=True, state="terminated")
            elif operation == "cleanup":
                if name in worlds:
                    release(worlds, name)
                environment_states[name] = "cleaned"
                reply(sys.stdout, request_id, ok=True, state="cleaned")
            else:
                raise ValueError("unsupported operation")
        except Exception as error:
            reply(sys.stdout, request_id, ok=False, error=f"{type(error).__name__}: {error}")
        finally:
            with lock:
                cancellation.pop(request_id, None)
    for name in list(worlds):
        release(worlds, name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
