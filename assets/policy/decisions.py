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


def predict(data, *, policy_config, capabilities):
    values = policy_config["candidates"]
    if "sample_count" in policy_config:
        import random
        values = random.SystemRandom().sample(values, policy_config["sample_count"])
    return {"proposals": [{"payload": value} for value in values], "generation": "batch"}


def launch(data, *, policy_config, capabilities):
    return True


def effect(data, *, policy_config, capabilities):
    return {"mode": "block"}


def match_proposal(data, *, policy_config, capabilities):
    evidence = data["evidence"]
    if evidence is None:
        return {"verdict": "indeterminate"}
    compatible = not evidence.get("is_error", False) and data["proposal"]["payload"] == evidence["payload"]
    return {"verdict": "compatible" if compatible else "incompatible"}


def dispose(data, *, policy_config, capabilities):
    if data["resolution"] in ("indeterminate", "error", "extent_reached"):
        return {"actions": []}
    if data["resolution"] == "accepted":
        return {"actions": [{"intent": "Promote"}]}
    return {"actions": [{"intent": "Discard"}]}
