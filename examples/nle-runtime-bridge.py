#!/usr/bin/env python3
"""A small JSON-lines NLE workload bridge.

One NetHack world lives in this process.  The Runtime may name several
environments, but every name refers to the same world: ``branch`` creates a
view, not a copy.  Speculation is staged outside the environment -- a branch
never acts, and the world advances only on the authority's actions.

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
import json
import re
import sys
import time
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
    for line in sys.stdin:
        request = json.loads(line)
        request_id = request.get("id")
        try:
            operation = request.get("op")
            name = request.get("environment")
            if operation == "establish":
                if name in worlds:
                    raise ValueError("environment already exists")
                environment = gym.make(args.env_id)
                seed = request.get("seed")
                # NLE ignores reset(seed=...); it only reseeds through its own
                # seed() method.
                if seed is not None:
                    environment.unwrapped.seed(int(seed))
                observation, _ = environment.reset()
                worlds[name] = World(environment, digest(observation))
                reply(sys.stdout, request_id, ok=True, state="available", observation=worlds[name].observation)
            elif operation == "branch":
                world = require(worlds, name)
                child = request["child"]
                if child in worlds:
                    raise ValueError("child environment already exists")
                world.references += 1
                worlds[child] = world
                reply(sys.stdout, request_id, ok=True, state=world.state, child=child)
            elif operation == "operate":
                world = require(worlds, name)
                if world.state != "available":
                    raise ValueError("environment is not available")
                action = request["action"]
                if (
                    isinstance(action, bool)
                    or not isinstance(action, int)
                    or not 0 <= action < world.environment.action_space.n
                ):
                    raise ValueError("action is outside the NLE action space")
                if args.delay > 0:
                    time.sleep(args.delay)
                observation, reward, terminated, truncated, _ = world.environment.step(action)
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
                    if world.state != "available":
                        raise ValueError("environment is not available")
                    action = arguments.get("action")
                    if (
                        isinstance(action, bool)
                        or not isinstance(action, int)
                        or not 0 <= action < world.environment.action_space.n
                    ):
                        raise ValueError("action is outside the NLE action space")
                    before = world.observation
                    if args.delay > 0:
                        time.sleep(args.delay)
                    observation, _reward, _terminated, _truncated, _ = world.environment.step(action)
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
                reply(sys.stdout, request_id, ok=True, state=world.state)
            elif operation == "observe":
                world = require(worlds, name)
                reply(sys.stdout, request_id, ok=True, state=world.state, observation=world.observation)
            elif operation in {"suspend", "resume"}:
                world = require(worlds, name)
                if world.state not in {"available", "suspended"}:
                    raise ValueError("environment is not available for suspend/resume")
                world.state = "suspended" if operation == "suspend" else "available"
                reply(sys.stdout, request_id, ok=True, state=world.state)
            elif operation == "terminate":
                world = require(worlds, name)
                release(worlds, name)
                reply(sys.stdout, request_id, ok=True, state="terminated")
            elif operation == "cleanup":
                if name in worlds:
                    release(worlds, name)
                reply(sys.stdout, request_id, ok=True, state="cleaned")
            else:
                raise ValueError("unsupported operation")
        except Exception as error:
            reply(sys.stdout, request_id, ok=False, error=f"{type(error).__name__}: {error}")
    for name in list(worlds):
        release(worlds, name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
