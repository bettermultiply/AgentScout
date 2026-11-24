"""Detect observable stalls in a candidate's completed tool actions."""

from .info_match import (_aligned_info, _normalized_calls, _search_covered, _uncovered,
                         extract_info_set)


def should_stop_degraded(completed_tools, next_action=None, *, max_consecutive_errors=2,
                         max_repeated_actions=2, max_stagnant_steps=2) -> tuple[bool, str | None]:
    def call(entry):
        return {'name': entry.get('name') or entry.get('tool_name'),
                'arguments': entry.get('arguments') or {}}

    def failed(entry):
        return entry.get('status') != 'succeeded' or entry.get('is_error') is True

    def identity(action):
        info = extract_info_set([action])
        if info.pure_readonly:
            return action['name'], info
        return _normalized_calls([action], info) or None

    entries = list(completed_tools)
    tail = entries[-max_consecutive_errors:]
    if max_consecutive_errors > 0 and len(tail) == max_consecutive_errors and all(map(failed, tail)):
        return True, 'consecutive_errors'

    actions = list(map(call, entries))
    identities = list(map(identity, actions))
    upcoming = identity(call(next_action)) if next_action else None
    repeated = identities[-max_repeated_actions:]
    if (upcoming is not None and upcoming in identities or
            max_repeated_actions > 0 and len(repeated) == max_repeated_actions
            and repeated[0] is not None and all(item == repeated[0] for item in repeated)):
        return True, 'repeated_action'

    stagnant, earlier = 0, []
    for entry, action in zip(entries, actions):
        current = extract_info_set([action])
        if failed(entry) or not current.pure_readonly:
            stagnant = 0
            continue
        current, prior = _aligned_info(current, extract_info_set(earlier))
        searches = current.searches | {search for kind, search, _ in current.limited if kind == 'search'}
        progress = any(any(_uncovered(prior.file_ranges.get(path, ()), start, end))
                       for path, ranges in current.file_ranges.items() for start, end in ranges)
        progress |= any((1, None) not in prior.file_ranges.get(path, ())
                        and any(_uncovered(prior.tail_ranges.get(path, ()), start, end))
                        for path, ranges in current.tail_ranges.items() for start, end in ranges)
        progress |= any(not _search_covered(prior, search) and not any(
            kind == 'search' and previous == search for kind, previous, _ in prior.limited)
                        for search in searches)
        stagnant = 0 if progress else stagnant + 1
        earlier.append(action)
    if max_stagnant_steps > 0 and stagnant >= max_stagnant_steps:
        return True, 'stagnant_progress'
    return False, None
