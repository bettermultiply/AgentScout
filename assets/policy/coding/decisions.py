"""Speculate on Authority tool calls or on pending local verification."""

def _words(command):
    import re
    import shlex

    try:
        words = shlex.split(command, comments=True)
    except ValueError:
        return []
    while words and (re.match(r'^\w+=', words[0]) or words[0] in ('env', 'command', '--')):
        words.pop(0)
    return words


def _narrow_verification(command):
    from ascout_policy.info_match import _verification_words

    import re

    name, *args = _verification_words(command)
    if re.fullmatch(r'python(?:[23](?:\.\d+)?)?', name.rsplit('/', 1)[-1]):
        if args[:1] == ['-m']:
            args = args[1:]
        if not args:
            return False
        name, *args = args
    if name.rsplit('/', 1)[-1] not in ('pytest', 'py.test'):
        return False

    # Options with a separate value must not be confused with test paths.
    value_options = {'--basetemp', '-p', '-c', '-o', '--override-ini', '--tb', '--rootdir',
                     '-m', '--maxfail', '--color', '--capture', '--import-mode', '--log-level'}
    paths, options = [], iter(args)
    for arg in options:
        if arg == '--':
            paths.extend(options)
            break
        if arg == '-k':
            return bool(next(options, ''))
        if arg.startswith('-k') and len(arg) > 2:
            return True
        if arg in value_options:
            next(options, None)
        elif not arg.startswith('-'):
            paths.append(arg)
    return bool(paths and all(re.search(r'::(?:[^:]+::)*test_[^:]+$', path) for path in paths))


def _bash_commands(command):
    from ascout_policy.info_match import _shell_tokens

    if not isinstance(command, str):
        return
    start = 0
    for match in _shell_tokens(command):
        if match['operator'] in (';', '&', '|', '(', ')', '\n', '`'):
            yield _words(command[start:match.start()])
            start = match.end()
    yield _words(command[start:])


def opportunity_pass(data, *, policy_config, capabilities):
    op = data['op']
    if op['kind'] == 'model_inference':
        return policy_config.get('fast_model_speculation', False) and op['authority']
    return (policy_config.get('tool_speculation', True) and op['kind'] == 'bash'
            and op['produces_result'] and op['result_pending'])


def feasibility_pass(data, *, policy_config, capabilities):
    from ascout_policy.info_match import is_verification_command as _verification

    if data['state']['assumption_depth'] >= policy_config.get('max_assumption_depth', 1):
        return False
    op = data['op']
    return op['kind'] == 'model_inference' or _verification(op.get('arguments', {}).get('command'))


def select(data, *, policy_config, capabilities):
    available = max(0, policy_config['max_active_candidates'] - data['scheduling_context']['active_candidate_count'])
    frontier = sorted(data['frontier_view'], key=lambda chance: chance['assumption_depth'])
    return [chance['chance_id'] for chance in frontier[:available]]


def launch(data, *, policy_config, capabilities):
    return not (data['chance_context'].get('kind') == 'model_inference'
                and policy_config.get('prior_readonly_exploration', False)
                and data.get('authority_state', {}).get('steps_completed', 0) == 0)


def _test_path(path):
    import re

    *directories, name = path.lower().split('/')
    return bool({'test', 'tests', 'spec', 'specs', '__tests__'}.intersection(directories)
                or re.search(r'^test[_\.]|(?:_test|_spec|\.test|\.spec)\.', name))


def _history_state(history, command, _verification):
    from ascout_policy.info_match import is_readonly_action, match_verification_calls

    edited_source = edited_tests = covered = source_seen = False
    last_command = None
    for entry in reversed(history):
        tool, args = entry.get('tool_name'), entry.get('arguments') or {}
        succeeded = (entry.get('status') == 'succeeded' and entry.get('is_error') is not True
                     and (entry.get('result') or {}).get('is_error') is not True)
        previous = args.get('command') if tool == 'bash' else None
        if _verification(previous):
            last_command = last_command or previous
            if not source_seen and succeeded and command and (previous == command or match_verification_calls(
                    [{'name': 'bash', 'arguments': {'command': previous}}],
                    [{'name': 'bash', 'arguments': {'command': command}}])['matched']):
                covered = True
            continue
        if not succeeded:
            continue
        if tool in ('edit', 'write') and isinstance(args.get('path'), str):
            if _test_path(args['path']):
                edited_tests |= last_command is None
                continue
        elif tool != 'bash' or is_readonly_action(tool, args):
            continue
        if not source_seen:
            edited_source = last_command is None
            source_seen = True
    return edited_source, edited_tests, last_command, source_seen and not covered


def predict(data, *, policy_config, capabilities):
    from ascout_policy.info_match import is_verification_command as _verification

    chance = data['chance_context']
    command = chance.get('arguments', {}).get('command')
    history = data.get('history', [])
    edited_source, edited_tests, last_command, source_uncovered = _history_state(history, command, _verification)
    if chance.get('kind') in ('model_inference', 'modelinference'):
        if (edited_source or edited_tests) and last_command and policy_config.get('active_post_edit_test', True):
            return {'generation': 'batch', 'proposals': [{
                'payload': {'kind': 'predicted_tool_calls', 'tool_calls': [
                    {'name': 'bash', 'arguments': {'command': last_command}}]},
                'provenance': {'kind': 'active_post_edit_test'},
            }]}
        raw = policy_config.get('candidate_thinking', 'off')
        thinking = 'off' if raw is False else raw
        result = {'predict': 'branch_model', 'model': policy_config['candidate_model'],
                  'thinking': thinking}
        if policy_config.get('candidate_count', 1) > 1:
            result['candidates'] = policy_config['candidate_count']
        return result
    if not _verification(command):
        return {'abstain': True, 'reason': 'unrecognized_command'}
    phase = 'post_source_edit' if source_uncovered or edited_source else 'baseline_check'
    failures = [False]
    if edited_tests and not edited_source:
        phase, failures = 'tdd_repro', [True]
    elif source_uncovered and not _narrow_verification(command) and policy_config.get('adaptive_dual_tool_speculation', True):
        failures.append(True)
    return {'generation': 'batch', 'proposals': [_verification_proposal(failure, phase) for failure in failures]}


def _verification_proposal(failure, phase):
    kind, message = ('expected_test_failure', 'the test fails as expected.') if failure else (
        'verification_pass', 'the command completes successfully.')
    return {
        'payload': {'content': [{'type': 'text', 'text': 'Predicted verification result: ' + message}], 'isError': failure},
        'provenance': {'kind': kind, 'phase': phase},
    }


def extent(data, *, policy_config, capabilities):
    from ascout_policy.degradation import should_stop_degraded
    from ascout_policy.info_match import is_readonly_action, is_verification_command as _verification

    state = data['process_state']
    facts = state.get('semantic_facts', {})
    results = state.get('result_facts', {})
    counters = state.get('resource_counters', {})
    if state['steps_completed'] >= policy_config.get('max_steps', 12):
        return 'stop'
    next_action = facts.get('next_action') or {}
    kind = next_action.get('kind')
    name, args = next_action.get('name'), next_action.get('arguments') or {}
    if facts.get('prediction_kind') == 'predicted_tool_calls' and (
            kind == 'model_inference' or state.get('phase') == 'model_inference'):
        return 'stop'
    if facts.get('speculation_kind') == 'model_inference':
        verification = name == 'bash' and _verification(args.get('command'))
        if (policy_config.get('stop_before_mutation', True) and kind == 'tool_execution'
                and not verification and not is_readonly_action(name, args)):
            return 'stop'
        if should_stop_degraded(results.get('completed_tools', []), next_action)[0]:
            return 'stop'
    elif (results.get('last_action', {}).get('is_error') is True
          or (kind == 'tool_execution' and not is_readonly_action(name, args))
          or (kind == 'model_inference' and counters.get('model_inferences', 0) >= 1)):
        return 'stop'
    return 'continue'


def effect(data, *, policy_config, capabilities):
    action = data['effect_descriptor']
    tool, args = action.get('tool'), action.get('arguments') or {}
    path = args.get('path')
    if tool in ('read', 'edit', 'write') and isinstance(path, str) and '..' in path.split('/'):
        return {'mode': 'deny', 'reason': 'outside_workspace_boundary'}
    if tool == 'bash':
        for words in _bash_commands(args.get('command')):
            if not words:
                continue
            name, *operands = words
            name = name.rsplit('/', 1)[-1]
            if (name in ('curl', 'wget', 'ssh', 'scp', 'rsync')
                    or name == 'git' and 'push' in operands
                    or name == 'gh' and 'pr' in operands
                    or name == 'twine' and 'upload' in operands
                    or name in ('npm', 'pnpm', 'yarn') and 'publish' in operands):
                return {'mode': 'block', 'reason': 'external_effect_boundary'}
            if (name == 'find' and '/' in operands) or '..' in '/'.join(operands).split('/'):
                return {'mode': 'deny', 'reason': 'outside_workspace_boundary'}
    return {'mode': 'allow'}


def rebalance(data, *, policy_config, capabilities):
    active = data['active_view']
    actions = [{'branch_id': branch['branch_id'], 'action': 'preempt'}
               for branch in active if branch['tool_counts']['failed'] >= 2]
    limit = policy_config['max_active_candidates']
    paused = sorted(data['paused_view'], key=lambda branch: -branch['age_ms'])
    waiting = bool(paused or data.get('scheduling_context', {}).get('waiting_chance'))
    excess = max(0, len(active) - len(actions) - limit + int(waiting and limit > 0))
    preempting = {action['branch_id'] for action in actions}
    yielding = sorted((branch for branch in active if branch.get('yield_only')
                       and branch['branch_id'] not in preempting), key=lambda branch: -branch['age_ms'])
    actions.extend({'branch_id': branch['branch_id'], 'action': 'preempt'} for branch in yielding[:excess])
    available = max(0, limit - len(active) + len(actions))
    return actions + [{'branch_id': branch['branch_id'], 'action': 'resume'} for branch in paused[:available]]


def _output(outcome):
    content = outcome.get('content', outcome.get('payload', ''))
    if isinstance(content, list):
        return '\n'.join(part['text'] for part in content if part.get('type') == 'text')
    return str(content or '')


def _verification_entry(entry):
    from ascout_policy.info_match import is_verification_command as _verification

    return entry.get('tool_name') == 'bash' and _verification((entry.get('arguments') or {}).get('command'))


def _safe_results(entries):
    from ascout_policy.info_match import _entry_output

    latest = {}
    for entry in entries:
        name, args = entry['tool_name'], entry['arguments']
        target = args.get('command') if name == 'bash' else args.get('path')
        if target:
            key = name, target, args.get('offset'), args.get('limit')
            latest.pop(key, None)
            latest[key] = entry
    results, remaining = [], 14_000
    for (name, target, _, _), entry in latest.items():
        output = entry.get('cropped_output', _entry_output(entry))
        if not output.strip():
            continue
        label = f'{name} {target[:240]}'
        if name == 'read':
            bounds = [f'{key}={entry["arguments"][key]}' for key in ('offset', 'limit') if key in entry['arguments']]
            if bounds:
                label += f" ({', '.join(bounds)})"
        label = entry.get('cropped_label', label)
        kind = 'verification' if _verification_entry(entry) else 'workspace'
        header = f'[Speculative {kind} output collected on current files]: {label}\n'
        marker = '\n[truncated]'
        budget = min(4_500, remaining) - len(header)
        if budget <= len(marker):
            break
        if len(output) > budget:
            output = output[:budget - len(marker)] + marker
        notification = header + output
        results.append({'id': entry['action_id'], 'files': [], 'notification': notification})
        remaining -= len(notification)
        if len(results) == 8:
            break
    return results


def _match_model(proposal, outcome, context, config):
    from ascout_policy.info_match import (_entry_output, directional_overlap, filter_novel_readonly,
                                          is_readonly_action, match_verification_calls)

    if outcome is None:
        candidate = context['candidate']
        branch = candidate.get('branch') or {}
        if (not config.get('prior_readonly_exploration', False) or
                candidate.get('turn_active', branch.get('execution_state', {}).get('turn_active', False))
                and not candidate.get('blocked', isinstance(branch.get('status'), dict))):
            return {'verdict': 'indeterminate'}

    candidate = context['candidate']
    yielded = set(candidate.get('yielded_action_ids') or [])
    actions = [entry for entry in context.get('candidate_results', []) if entry.get('tool_name')]
    readonly = [entry for entry in actions if entry.get('status') == 'succeeded'
                and not (entry.get('result') or {}).get('is_error')
                and is_readonly_action(entry['tool_name'], entry.get('arguments'))]
    verification = [entry for entry in actions if _verification_entry(entry) and _entry_output(entry)]
    mutations = [entry for entry in actions if not _verification_entry(entry)
                 and not is_readonly_action(entry['tool_name'], entry.get('arguments'))]
    authority = outcome.get('tool_calls') if isinstance(outcome, dict) else None
    calls = (proposal.get('payload') or {}).get('tool_calls') or []
    completed = [{'name': entry['tool_name'], 'arguments': entry['arguments']} for entry in readonly]
    executed = [{'name': entry['tool_name'], 'arguments': entry['arguments']} for entry in verification]
    if not candidate.get('yield_only') and not mutations and (match_verification_calls(calls, authority)['matched']
                          or match_verification_calls(completed + executed, authority)['matched']):
        return {'verdict': 'compatible', 'score': 1.0, 'details': {
            'matched_tool_calls': True, 'match_kind': 'verification', 'coverage': 1.0}}
    novel = filter_novel_readonly(readonly, authority) if config.get('readonly_salvage', True) else []
    overlap = directional_overlap(completed, authority)
    anchored = overlap > 0 and overlap >= config.get('min_partial_coverage', 0.25)
    safe = [] if mutations else _safe_results(
        [entry for entry in verification + novel if entry['action_id'] not in yielded]
        if verification or anchored or outcome is None else [])
    if safe:
        return {'verdict': 'incompatible', 'score': overlap, 'details': {
            'match_kind': 'prior_readonly' if outcome is None else 'partial_info',
            'coverage': overlap, 'safe_results': safe}}
    return {'verdict': 'incompatible', 'score': 0.0}


def match_proposal(data, *, policy_config, capabilities):
    outcome = (data.get('evidence') or {}).get('outcome', data.get('outcome'))
    proposal = data['proposal']
    context = data.get('match_context') or {}
    if context.get('candidate', {}).get('speculation_kind') == 'model_inference':
        return _match_model(proposal, outcome, context, policy_config)
    if outcome is None:
        return {'verdict': 'indeterminate'}
    if isinstance(outcome, dict):
        kind = (proposal.get('provenance') or {}).get('kind')
        status = outcome.get('status') or context.get('authority', {}).get('status')
        if kind in ('verification_pass', 'expected_test_failure'):
            invocation_errors = (
                "can't open file", 'no such file or directory', 'command not found', 'permission denied',
                'error collecting', 'unrecognized arguments', 'no module named', 'syntaxerror',
                'importerror', 'usage:',
            )
            output = _output(outcome).lower()
            if any(marker in output for marker in invocation_errors):
                return {'verdict': 'incompatible', 'score': 0.0}
        if kind == 'verification_pass' and status == 'succeeded' and outcome.get('is_error') is False:
            return {'verdict': 'compatible', 'score': 1.0}
        if kind == 'expected_test_failure' and outcome.get('is_error') is True:
            return {'verdict': 'compatible', 'score': 0.9}
    return {'verdict': 'incompatible', 'score': 0.0}


def dispose(data, *, policy_config, capabilities):
    resolution, context = data['resolution'], data['context']
    outcome = context.get('authority_outcome')
    if resolution == 'accepted':
        action = {'intent': 'Promote'}
        if isinstance(outcome, dict) and not ((context.get('match') or {}).get('details') or {}).get('matched_tool_calls'):
            action['notification'] = 'Real verification matched the prediction.\n' + _output(outcome).strip()[-600:]
        return {'actions': [action]}
    if resolution in ('extent_reached', 'indeterminate') and outcome is None:
        return {'actions': []}
    safe = ((context.get('match') or {}).get('details') or {}).get('safe_results')
    discard = {'intent': 'Discard'}
    candidate = context.get('candidate') or {}
    if policy_config.get('yield_in_flight_verification', True) and resolution == 'rejected':
        if (candidate.get('yield_only') and safe
                or any(_verification_entry(tool) for tool in candidate.get('active_tools') or [])):
            return {'actions': [{'intent': 'Yield', 'results': safe or []}]}
    if resolution == 'rejected' and safe:
        return {'actions': [{'intent': 'Salvage', 'results': safe}, discard]}
    return {'actions': [discard]}
