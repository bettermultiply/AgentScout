"""Example author algorithms, not system defaults or built-in strategies."""


def opportunity_pass(data, *, policy_config, capabilities):
    op = data["op"]
    return op["produces_result"] and op["result_pending"] and op["has_predictor"]


def feasibility_pass(data, *, policy_config, capabilities):
    if policy_config.get("example_tools_only", False):
        import re
        op = data["op"]
        if op["kind"] in ("read", "ls", "find", "grep"):
            return True
        command = data["contract_context"].get("arguments", {}).get("command", "")
        return op["kind"] == "bash" and (
            re.fullmatch(r"sleep [0-9]{1,2}(?:\.[0-9]+)?; printf '(?:example|true|false|different)'", command) is not None
            or command == "sleep 6; python3 -c 'import secrets; print(secrets.choice([\"example\", \"true\", \"false\"]), end=\"\")'"
        )
    return True


def select(data, *, policy_config, capabilities):
    return [edge["edge_id"] for edge in data["frontier_view"][:policy_config["max_edges"]]]


def rebalance(data, *, policy_config, capabilities):
    return []


def extent(data, *, policy_config, capabilities):
    return "stop" if data["process_state"]["steps_completed"] >= policy_config["max_steps"] else "continue"


# NLE's movement keys, as (dx, dy) offsets on the public character grid.
_MOVES = {
    1: (0, -1),  # k north
    2: (1, 0),  # l east
    3: (0, 1),  # j south
    4: (-1, 0),  # h west
    5: (1, -1),  # u northeast
    6: (1, 1),  # n southeast
    7: (-1, 1),  # b southwest
    8: (-1, -1),  # y northwest
}
_WALLS = set("|-")
_FLOOR = set(".#<>")


def predict(data, *, policy_config, capabilities):
    """Enumerate the possible outcomes of one plain move.

    Only movement actions are considered.  A tile the agent has already seen
    is decided exactly; an unseen tile yields every declared outcome, which is
    where speculation actually happens.
    """
    values = policy_config["enum_values"]
    if "candidates" in policy_config:
        import secrets
        candidates = list(policy_config["candidates"])
        count = min(policy_config.get("sample_count", len(candidates)), len(candidates))
        selected = secrets.SystemRandom().sample(candidates, count)
        return {"proposals": [{"payload": value} for value in selected], "generation": "batch"}
    context = data.get("process_context") or {}
    observation = context.get("observation") or {}
    try:
        action = int(context.get("action"))
    except (TypeError, ValueError):
        return {"proposals": [], "generation": "batch"}
    move = _MOVES.get(action)
    if move is None:
        return {"proposals": [], "generation": "batch"}
    chars = observation.get("chars")
    stats = observation.get("blstats")
    if not chars or not stats:
        return {"proposals": [], "generation": "batch"}
    x, y = int(stats[0]), int(stats[1])
    tx, ty = x + move[0], y + move[1]
    if not (0 <= ty < len(chars) and 0 <= tx < len(chars[ty])):
        return {"proposals": [], "generation": "batch"}
    tile = chars[ty][tx]
    if tile in _WALLS:
        outcomes = ["blocked"]
    elif tile in _FLOOR:
        outcomes = ["moved"]
    else:
        outcomes = list(values)
    return {"proposals": [{"payload": value} for value in outcomes], "generation": "batch"}


def launch(data, *, policy_config, capabilities):
    return True


def effect(data, *, policy_config, capabilities):
    return {"mode": "block"}


def _actual_outcome(evidence):
    """Read one real action's outcome from the observations around it."""
    start = (evidence.get("before") or {}).get("blstats")
    end = (evidence.get("after") or {}).get("blstats")
    if not isinstance(start, list) or not isinstance(end, list):
        return None
    if len(start) < 2 or len(end) < 2:
        return None
    return "moved" if (start[0], start[1]) != (end[0], end[1]) else "blocked"


def match_proposal(data, *, policy_config, capabilities):
    evidence = data["evidence"]
    if evidence is None:
        return {"verdict": "indeterminate"}
    if evidence.get("is_error", False):
        return {"verdict": "incompatible"}
    if "before" in evidence and "after" in evidence:
        # The real outcome is the verifier's job: AScout only ships the
        # observations, the Policy decides what counts as moved or blocked.
        actual = _actual_outcome(evidence)
    else:
        actual = evidence.get("payload")
    if actual is None:
        return {"verdict": "indeterminate"}
    if policy_config["value_type"] == "boolean" and actual in ("true", "false"):
        actual = actual == "true"
    elif policy_config["value_type"] == "enum":
        if actual not in policy_config["enum_values"]:
            return {"verdict": "incompatible"}
    compatible = data["proposal"]["payload"] == actual
    return {"verdict": "compatible" if compatible else "incompatible"}


def dispose(data, *, policy_config, capabilities):
    if data["resolution"] in ("indeterminate", "error", "extent_reached"):
        return {"actions": []}
    if data["resolution"] == "accepted":
        return {"actions": [{"intent": "Promote"}]}
    return {"actions": [{"intent": "Discard"}]}
