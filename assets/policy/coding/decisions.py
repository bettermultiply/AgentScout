"""Coding policy for the explore → modify → verify cycle."""

def _narrow_verification(command):
    import re
    from ascout_policy.info_match import _verification_words

    name, *args = _verification_words(command)
    if re.fullmatch(r'python(?:[23](?:\.\d+)?)?', name.rsplit('/', 1)[-1]):
        if args[:1] == ['-m']:
            args = args[1:]
        if not args:
            return False
        name, *args = args
    if name.rsplit('/', 1)[-1] not in ('pytest', 'py.test'):
        return False

    value_options = {'--basetemp', '-p', '-c', '-o', '--override-ini', '--tb', '--rootdir',
                     '-m', '--maxfail', '--color', '--capture', '--import-mode', '--log-level'}
    paths, options = [], iter(args)
    keyword = ''
    for arg in options:
        if arg == '--':
            paths.extend(options)
            break
        if arg == '-k':
            keyword = next(options, '')
        elif arg.startswith('-k'):
            keyword = arg[2:].removeprefix('=')
        elif arg in value_options:
            next(options, None)
        elif not arg.startswith('-'):
            paths.append(arg)
    # A leading exclusion does not identify a small set of tests.
    return bool((keyword.strip() and not re.match(r'[\s(]*not(?=[\s(]|$)', keyword))
                or paths and all(re.search(r'::(?:[^:]+::)*test_[^:]+$', path) for path in paths))


def _verification_entry(entry):
    from ascout_policy.info_match import is_verification_command

    return entry.get('tool_name') == 'bash' and is_verification_command(
        (entry.get('arguments') or {}).get('command'))


def _workspace_verification(command):
    from ascout_policy.info_match import _verification_words

    directories = []
    _verification_words(command, directories=directories)
    for path in directories:
        if path.startswith('/') or any(part == '..' or part.startswith('.') and part != '.'
                                       for part in path.split('/')):
            return False
    return True


def _succeeded(entry):
    return (entry.get('status') == 'succeeded' and entry.get('is_error') is not True
            and (entry.get('result') or {}).get('is_error') is not True)


def _scratch_path(path, workspace, directory):
    from posixpath import normpath, relpath

    if not isinstance(path, str) or any(char in path for char in '$`'):
        return False
    path = normpath(path)
    if workspace and path.startswith('/'):
        relative = relpath(path, workspace)
        if relative != '..' and not relative.startswith('../'):
            path = relative
    first, sep, _ = path.partition('/')
    return (path == '/tmp' or path.startswith('/tmp/')
            or first.startswith('.') and first not in ('.', '..') and (sep or directory))


def _python_writes(script, workspace):
    import ast

    def writes(path):
        if isinstance(path, ast.Call) and getattr(path.func, 'id', None) == 'Path':
            path = path.args[0] if len(path.args) == 1 else None
        return not _scratch_path(getattr(path, 'value', None), workspace, False)

    try:
        syntax = ast.parse(script)
    except SyntaxError:
        return False
    for call in ast.walk(syntax):
        if not isinstance(call, ast.Call):
            continue
        if isinstance(call.func, ast.Name):
            name = call.func.id
            path_method = False
        elif isinstance(call.func, ast.Attribute):
            name = call.func.attr
            path_method = not (isinstance(call.func.value, ast.Name) and call.func.value.id in ('io', 'builtins'))
            receiver = call.func.value
            if name in {'write_text', 'write_bytes', 'unlink', 'mkdir', 'rmdir', 'touch', 'chmod'}:
                path = call.args[0] if isinstance(receiver, ast.Name) and receiver.id == 'os' and call.args else receiver
                if writes(path):
                    return True
            if name in {'remove', 'rename', 'replace'} and (
                    isinstance(receiver, ast.Name) and receiver.id == 'os'
                    or isinstance(receiver, ast.Call) and getattr(receiver.func, 'id', None) == 'Path'):
                paths = call.args[:2] if isinstance(receiver, ast.Name) else [receiver, *call.args[:1]]
                if not paths or any(map(writes, paths)):
                    return True
            if (isinstance(receiver, ast.Name) and receiver.id == 'shutil'
                    and name in {'copy', 'copy2', 'copyfile', 'copyfileobj', 'copymode', 'copystat',
                                 'copytree', 'move', 'rmtree', 'make_archive', 'unpack_archive', 'chown'}):
                return True
        else:
            continue
        if name == 'open':
            mode_index = 0 if path_method else 1
            mode = next((kw.value for kw in call.keywords if kw.arg == 'mode'),
                        call.args[mode_index] if len(call.args) > mode_index else None)
            if isinstance(mode, ast.Constant) and isinstance(mode.value, str) and any(
                    flag in mode.value for flag in 'wax+'):
                path = call.func.value if path_method else next(
                    (kw.value for kw in call.keywords if kw.arg == 'file'), call.args[0] if call.args else None)
                if writes(path):
                    return True
    return False


def _shell_writes(command, workspace):
    import re
    import shlex

    if not isinstance(command, str):
        return False
    # Separate heredoc data from shell syntax and inspect Python bodies as code.
    heredoc = re.compile(r'''(?m)^(?P<head>[^\n]*?)<<(?P<tabs>-?)[ \t]*
        (?P<quote>['"]?)(?P<marker>\w+)(?P=quote)(?P<tail>[^\n]*)\n
        (?P<body>[\s\S]*?)^\t*(?P=marker)[ \t]*(?=\n|$)''', re.X)
    python = re.compile(r'python(?:[23](?:\.\d+)?)?$')
    for match in heredoc.finditer(command):
        commands = list(_bash_commands(match['head']))
        if commands and commands[-1] and python.fullmatch(commands[-1][0].rsplit('/', 1)[-1]):
            body = match['body']
            if match['tabs']:
                body = re.sub(r'(?m)^\t+', '', body)
            if _python_writes(body, workspace):
                return True
    command = heredoc.sub(lambda match: match['head'] + ' ' + match['tail'], command)
    word = r'''(?:\\.|'[^']*'|"(?:\\.|[^"\\])*"|[^\s;&|<>()'"\\])+'''
    tokens = re.compile(r'''\\.|'[^']*'|"(?:\\.|[^"\\])*"|(?<!\S)\#.*|'''
                        r'''(?:(?<!\S)\d+)?(?:>&[\d-]+|(?:&?>>?|>\|)[ \t]*(?P<target>'''
                        + word + r''')?)''')
    for token in tokens.finditer(command):
        if target := token['target']:
            path = shlex.split(target)[0]
            if path != '/dev/null' and not _scratch_path(path, workspace, False):
                return True
    # Remove redirects before inspecting command operands.
    command = tokens.sub(lambda token: ' ' if token['target'] else token[0], command)
    for words in _bash_commands(command):
        if not words:
            continue
        name, *args = words
        name = name.rsplit('/', 1)[-1]
        if (name in {'apply_patch', 'patch'} or name == 'git' and args[:1] == ['apply']
                or name in {'sed', 'perl'} and any(
                    re.match(r'^-[a-zA-Z]*i|^--in-place(?:=|$)', arg) for arg in args)):
            return True
        if name in {'tee', 'rm', 'mv', 'cp', 'mkdir', 'touch', 'chmod', 'rmdir'}:
            paths = [arg for arg in args if not arg.startswith('-')]
            if name == 'cp':
                paths = paths[-1:]
            if name == 'chmod':
                paths = paths[1:]
            directory = name in {'mkdir', 'rmdir'} or name == 'rm' and any(
                arg == '--recursive' or re.fullmatch(r'-[a-zA-Z]*[rR][a-zA-Z]*', arg) for arg in args)
            if any(not _scratch_path(path, workspace, directory) for path in paths):
                return True
        if python.fullmatch(name) and '-c' in args:
            index = args.index('-c') + 1
            if index < len(args) and _python_writes(args[index], workspace):
                return True
    return False


def _mutation(entry, workspace):
    import re
    from ascout_policy.info_match import is_readonly_action

    tool, args = entry.get('tool_name'), entry.get('arguments') or {}
    if not _succeeded(entry) or _verification_entry(entry):
        return None
    if tool in ('edit', 'write') and isinstance(path := args.get('path'), str):
        if _scratch_path(path, workspace, False):
            return None
        *directories, name = path.lower().split('/')
        if ({'test', 'tests', 'spec', 'specs', '__tests__'}.intersection(directories)
                or re.search(r'^test[_\.]|(?:_test|_spec|\.test|\.spec)\.', name)):
            return 'test'
        return 'source'
    if tool == 'bash' and not is_readonly_action(tool, args) and _shell_writes(args.get('command'), workspace):
        return 'source'
    return None


_VERIFICATION_HEADER = '[Speculative verification output collected on current files]: bash '


def _completed_verification(entry):
    return (entry.get('status') in ('succeeded', 'failed') and _verification_entry(entry)
            and _workspace_verification(entry['arguments']['command'])
            and not _invocation_error(entry.get('result') or entry))


def _history_verifications(entry):
    import re
    from ascout_policy.info_match import is_verification_command

    if _completed_verification(entry):
        yield entry
    if entry.get('role') == 'custom' and entry.get('custom_type') == 'notification':
        for match in re.finditer(r'(?m)^(?:Accepted result: )?' + re.escape(_VERIFICATION_HEADER) + r'([^\n]+)',
                                 _output(entry)):
            command = match[1]
            if is_verification_command(command) and _workspace_verification(command):
                yield {'tool_name': 'bash', 'arguments': {'command': command}}


def _covers_verification(entry, command):
    from ascout_policy.info_match import match_verification_calls

    previous = entry['arguments']['command']
    return previous == command or match_verification_calls(
        [{'name': 'bash', 'arguments': {'command': previous}}],
        [{'name': 'bash', 'arguments': {'command': command}}])['matched']


def _started_after(entry, edited):
    started = entry.get('started_at_ms')
    return started is None or edited is None or started >= edited


def _detect_stage(history, current_op, readonly_reset_steps, workspace):
    from ascout_policy.info_match import is_readonly_action, is_verification_command

    command = (current_op.get('arguments') or {}).get('command')
    verifying = is_verification_command(command)
    verifications = [list(_history_verifications(entry)) for entry in history]
    commands = [entry['arguments']['command'] for group in reversed(verifications) for entry in group]
    main_command = next((previous for previous in commands if not _narrow_verification(previous)),
                        next(iter(commands), None))
    required = command if verifying else main_command
    edits, covering = set(), []
    for entry, checks in reversed(list(zip(history, verifications))):
        covering.extend(check for check in checks if required and _covers_verification(check, required))
        if mutation := _mutation(entry, workspace):
            if any(_started_after(check, entry.get('timestamp_ms')) for check in covering):
                break
            edits.add(mutation)

    if verifying:
        return 'verify', main_command, edits
    tools = [entry for entry in history if entry.get('tool_name')]
    recent = tools[-readonly_reset_steps:] if readonly_reset_steps > 0 else []
    if not edits or (recent and len(recent) == readonly_reset_steps and all(
            _succeeded(entry) and is_readonly_action(entry['tool_name'], entry.get('arguments'))
            for entry in recent)):
        return 'explore', main_command, edits
    return ('modify_ready' if main_command else 'modify_early'), main_command, edits


def opportunity_pass(data, *, policy_config, capabilities):
    op = data['op']
    if op['kind'] == 'model_inference':
        return policy_config.get('fast_model_speculation', True) and op['authority']
    return (policy_config.get('tool_speculation', True) and op['kind'] == 'bash'
            and op['produces_result'] and op['result_pending'])


def feasibility_pass(data, *, policy_config, capabilities):
    from ascout_policy.info_match import is_verification_command

    if data['state']['assumption_depth'] >= policy_config.get('max_assumption_depth', 1):
        return False
    op = data['op']
    return op['kind'] == 'model_inference' or is_verification_command(op.get('arguments', {}).get('command'))


def select(data, *, policy_config, capabilities):
    available = max(0, policy_config['max_active_candidates'] - data['scheduling_context']['active_candidate_count'])
    frontier = sorted(data['frontier_view'], key=lambda chance: chance['assumption_depth'])
    return [chance['chance_id'] for chance in frontier[:available]]


def launch(data, *, policy_config, capabilities):
    return True


def _candidate_settings(config, prefix):
    model = config.get(prefix + 'candidate_model', 'deepseek/deepseek-flash')
    if not model:
        return {}
    thinking = config.get(prefix + 'candidate_thinking', 'off')
    settings = {'model': model, 'thinking': 'off' if thinking is False else thinking}
    guidance = config.get(prefix + 'candidate_guidance')
    if guidance:
        settings['candidate_guidance'] = guidance
    return settings


def _share_commands(prediction, branches, config):
    if config.get('share_inflight_commands', False):
        commands = dict.fromkeys(
            command for branch in branches for tool in branch.get('active_tools', [])
            if isinstance(command := (tool.get('arguments') or {}).get('command'), str) and command.strip())
        if commands:
            listing = '\n'.join(f'- {command[:240]}' for command in list(commands)[:8])
            guidance = prediction.get('candidate_guidance', '')
            prediction['candidate_guidance'] = (
                f'{guidance}\n\nAvoid duplicating commands already running in other candidates:\n{listing}').strip()
    return prediction


def predict(data, *, policy_config, capabilities):
    chance = data['chance_context']
    process = data.get('process_context') or {}
    branches = process.get('active_branches', [])
    stage, main_command, edits = _detect_stage(
        data.get('history', []), chance, policy_config.get('readonly_reset_steps', 2), process.get('workspace'))
    if stage == 'verify':
        command = chance['arguments']['command']
        phase, failures = 'baseline_check', [False]
        if _workspace_verification(command) and not _narrow_verification(command):
            if edits == {'test'}:
                phase, failures = 'tdd_repro', [True]
            elif 'source' in edits:
                phase = 'post_source_edit'
                if policy_config.get('adaptive_dual_tool_speculation', True):
                    failures.append(True)
        proposals = [_verification_proposal(failure, phase, policy_config) for failure in failures]
        for proposal in proposals:
            if proposal.get('candidate_guidance'):
                _share_commands(proposal, branches, policy_config)
        return {'generation': 'batch', 'proposals': proposals}

    if chance.get('kind') not in ('model_inference', 'modelinference'):
        return {'abstain': True, 'reason': 'unrecognized_command'}
    if stage == 'modify_ready':
        if (not policy_config.get('active_post_edit_test', True)
                or any(map(_verification_entry, process.get('authority_active_tools', [])))
                or any((branch.get('proposal_provenance') or {}).get('kind') == 'active_post_edit_test'
                       for branch in branches)):
            return {'abstain': True, 'reason': 'verification_disabled_or_running'}
        return {'generation': 'batch', 'proposals': [{
            'payload': {'kind': 'predicted_tool_calls', 'tool_calls': [
                {'name': 'bash', 'arguments': {'command': main_command}}]},
            'provenance': {'kind': 'active_post_edit_test'},
        }]}
    if stage == 'modify_early' and not policy_config.get('speculate_early_modify', False):
        return {'abstain': True, 'reason': stage}
    if stage == 'explore' and any(
            not branch.get('yield_only') and branch.get('speculation_kind') == 'model_inference'
            and (branch.get('proposal_provenance') or {}).get('kind') != 'active_post_edit_test'
            for branch in branches):
        return {'abstain': True, 'reason': 'active_explore_branch'}
    settings = _candidate_settings(policy_config, '')
    if not settings:
        return {'abstain': True, 'reason': 'candidate_model_disabled'}
    result = {'predict': 'branch_model', **settings}
    if policy_config.get('candidate_count', 1) > 1:
        result['candidates'] = policy_config['candidate_count']
    return _share_commands(result, branches, policy_config)


def _verification_proposal(failure, phase, config):
    kind, message = ('expected_test_failure', 'Command exited with non-zero status.') if failure else (
        'verification_pass', 'All tests passed.')
    settings = _candidate_settings(config, 'fail_') if failure else {}
    return {
        'payload': {'content': [{'type': 'text', 'text': message}], 'isError': failure},
        'provenance': {'kind': kind, 'phase': phase, 'uses_candidate_model': bool(settings)},
        **settings,
    }


def extent(data, *, policy_config, capabilities):
    from ascout_policy.degradation import should_stop_degraded

    state = data['process_state']
    facts = state.get('semantic_facts', {})
    next_action = facts.get('next_action') or {}
    kind = next_action.get('kind') or state.get('phase')
    model_boundary = state.get('phase') == 'model_inference' or kind == 'model_inference'
    if facts.get('yield_only') is True:
        return 'stop' if model_boundary else 'continue'
    if state['steps_completed'] >= policy_config.get('max_steps', 12):
        return 'stop'
    completed = state.get('result_facts', {}).get('completed_tools', [])
    if facts.get('prediction_kind') == 'predicted_tool_calls' and model_boundary:
        return 'stop'
    if facts.get('speculation_kind') == 'tool_execution' and kind == 'tool_execution':
        if _verification_entry({'tool_name': next_action.get('name'), 'arguments': next_action.get('arguments')}):
            for index in range(len(completed) - 1, -1, -1):
                mutation = completed[index]
                if _mutation(mutation, None):
                    verified = any(_passing_verification(entry, mutation) for entry in completed[index + 1:])
                    return 'stop' if verified else 'continue'
            return 'stop'
    if completed and should_stop_degraded(completed, next_action)[0]:
        return 'stop'
    return 'continue'


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
    active, paused = data['active_view'], data['paused_view']
    context = data['scheduling_context']
    last_edit = max((entry['timestamp_ms'] for entry in context.get('authority_recent_actions', [])
                     if _mutation(entry, None)), default=0)
    failed_limit = policy_config.get('preempt_failed_tools', 2)
    preempt = [branch for branch in active + paused if branch['tool_counts']['failed'] >= failed_limit
               or (branch.get('yield_only') and branch.get('speculation_kind') == 'model_inference'
                   and last_edit > branch['started_at_ms'])]
    removed = {branch['branch_id'] for branch in preempt}
    waiting = context.get('waiting_chance')
    limit = policy_config['max_active_candidates']
    running = [branch for branch in active if branch['branch_id'] not in removed]
    excess = max(0, len(running) - limit + int(bool(waiting and limit > 0)))
    yielding = sorted((branch for branch in running if branch.get('yield_only')),
                      key=lambda branch: (branch['started_at_ms'], branch['branch_id']))
    evicted = yielding[:excess]
    available = max(0, limit - len(running) + len(evicted) - int(bool(waiting)))
    resumable = sorted((branch for branch in paused if branch['branch_id'] not in removed),
                      key=lambda branch: (branch['started_at_ms'], branch['branch_id']))
    return ([{'branch_id': branch['branch_id'], 'action': 'preempt'} for branch in preempt + evicted]
            + [{'branch_id': branch['branch_id'], 'action': 'resume'} for branch in resumable[:available]])


def _output(outcome):
    content = outcome.get('content', outcome.get('payload', ''))
    if isinstance(content, list):
        return '\n'.join(part['text'] for part in content if part.get('type') == 'text')
    return str(content or '')


def _result_key(entry):
    from posixpath import join, normpath
    from ascout_policy.info_match import normalize_verification_command

    name, args = entry['tool_name'], entry['arguments']
    if _verification_entry(entry):
        directories = []
        command = args['command']
        signature = normalize_verification_command(command, directories=directories)
        return 'verification', normpath(join('.', *directories)), signature or command
    return name, args.get('command') or args.get('path'), args.get('offset'), args.get('limit')


def _safe_results(entries, config, yielded):
    from ascout_policy.info_match import _entry_output

    latest, seen = {}, set()
    for entry in entries:
        key = _result_key(entry)
        if entry['action_id'] in yielded:
            seen.add(key)
        latest.pop(key, None)
        latest[key] = entry

    results = []
    remaining, count = (config.get(key) for key in ('safe_result_chars', 'safe_result_count'))
    remaining = remaining if type(remaining) is int and remaining > 0 else None
    count = count if type(count) is int and count > 0 else None
    for key, entry in latest.items():
        if count is not None and len(results) >= count:
            break
        if entry['action_id'] in yielded or (key[0] == 'verification' and key in seen):
            continue
        output = entry['cropped_output'] if 'cropped_output' in entry else _entry_output(entry)
        name, args = entry['tool_name'], entry['arguments']
        target = args.get('command') or args.get('path')
        if not target or not output.strip():
            continue
        label = f'{name} {target}'
        if name == 'read':
            bounds = [f'{key}={args[key]}' for key in ('offset', 'limit') if key in args]
            if bounds:
                label += f" ({', '.join(bounds)})"
        label = entry.get('cropped_label', label)
        header = _VERIFICATION_HEADER + target if key[0] == 'verification' else (
            f'[Speculative workspace output collected on current files]: {label}')
        notification = f'{header}\n{output}'
        if remaining is None or len(notification) <= remaining:
            results.append({'id': entry['action_id'], 'files': [], 'notification': notification})
            if remaining is not None:
                remaining -= len(notification)
    return results


def _readonly_results(actions, authority_calls, workspace):
    from ascout_policy.info_match import filter_novel_readonly, is_readonly_action

    readonly = []
    for entry in actions:
        # Results after a candidate-only write can describe files Authority has never seen.
        if _mutation(entry, workspace):
            break
        if _succeeded(entry) and is_readonly_action(entry['tool_name'], entry.get('arguments')):
            readonly.append(entry)
    return filter_novel_readonly(readonly, authority_calls)


def _match_model(proposal, outcome, context, config):
    from ascout_policy.info_match import match_verification_calls

    candidate = context.get('candidate') or {}
    calls = outcome.get('tool_calls') if isinstance(outcome, dict) else None
    actions = [entry for entry in context.get('candidate_results', []) if entry.get('tool_name')]
    predicted = (proposal.get('payload') or {}).get('tool_calls') or []
    proactive = (proposal.get('provenance') or {}).get('kind') == 'active_post_edit_test'
    if proactive and not candidate.get('yield_only'):
        if outcome is None:
            return {'verdict': 'indeterminate'}
        if match_verification_calls(predicted, calls)['matched']:
            return {'verdict': 'compatible', 'score': 1.0, 'details': {'matched_tool_calls': True}}

    workspace = context.get('workspace')
    verification = []
    if proactive:
        current_checks, last_edit = [], None
        for entry in reversed(context.get('authority_history') or []):
            if _mutation(entry, workspace):
                last_edit = entry.get('timestamp_ms')
                break
            current_checks.extend(_history_verifications(entry))
        for entry in actions:
            if _mutation(entry, workspace):
                break
            if _completed_verification(entry):
                command = entry['arguments']['command']
                if (_started_after(entry, last_edit)
                        and not any(_started_after(check, last_edit) and _covers_verification(check, command)
                                    for check in current_checks)):
                    verification.append(entry)
    safe = _safe_results(verification + _readonly_results(actions, calls, workspace), config,
                         set(candidate.get('yielded_action_ids') or []))
    return {'verdict': 'incompatible', 'score': 0.0, 'details': {'safe_results': safe}}


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
            if _invocation_error(outcome):
                return {'verdict': 'incompatible', 'score': 0.0}
        if kind == 'verification_pass' and status == 'succeeded' and outcome.get('is_error') is False:
            return {'verdict': 'compatible', 'score': 1.0}
        if kind == 'expected_test_failure' and outcome.get('is_error') is True:
            return {'verdict': 'compatible', 'score': 0.9}
    return {'verdict': 'incompatible', 'score': 0.0}


def _invocation_error(result):
    return any(marker in _output(result).lower() for marker in (
        "can't open file", 'no such file or directory', 'command not found', 'permission denied',
        'error collecting', 'unrecognized arguments', 'no module named', 'syntaxerror',
        'importerror', 'usage:',
    ))


def _passing_verification(entry, mutation):
    return _started_after(entry, mutation.get('timestamp_ms')) and _completed_verification(entry) and _succeeded(entry)


def _verified_files(actions, workspace, config):
    from posixpath import isabs, normpath, relpath
    from textwrap import shorten

    latest, mutation, verification = {}, None, None
    for entry in actions:
        name, args = entry['tool_name'], entry.get('arguments') or {}
        if name in ('edit', 'write') and not _succeeded(entry):
            return []
        if _mutation(entry, workspace):
            mutation, verification = entry, None
            if name in ('edit', 'write'):
                path = normpath(args['path'])
                if isabs(path):
                    if not workspace:
                        continue
                    path = relpath(path, workspace)
                if path != '..' and not path.startswith('../'):
                    latest[path] = entry
        elif mutation and _passing_verification(entry, mutation):
            verification = entry
    if not latest or verification is None:
        return []

    def preview(value):
        return shorten(str(value or ''), width=160, placeholder='…')

    results = []
    for path, entry in latest.items():
        args = entry['arguments']
        if entry['tool_name'] == 'edit':
            edits = args.get('edits') or [args]
            change = '; '.join(f'{preview(edit.get("oldText"))!r} → {preview(edit.get("newText"))!r}'
                               for edit in edits[:3])
        else:
            change = f'wrote {preview(args.get("content"))!r}'
        results.append({'id': entry['action_id'], 'notification': f'{path}: {change}'})
    command = verification['arguments']['command']
    output = _verification_notification(verification.get('result') or verification,
                                        config.get('verification_output_chars', 20_000))
    results.append({'id': verification['action_id'], 'files': [], 'notification':
                    'Changes produced by a fast speculative candidate passed local workspace verification. '
                    'Review logic and edge cases before finalizing.\n' + command + '\n' + output})
    return results


def _verification_notification(outcome, limit):
    output = _output(outcome)
    if len(output) > limit:
        marker = '\n…\n'
        budget = max(0, limit - len(marker))
        head, tail = budget // 2, budget - budget // 2
        output = output[:head] + marker + (output[-tail:] if tail else '')
    return 'Real verification output:\n' + output


def dispose(data, *, policy_config, capabilities):
    resolution, context = data['resolution'], data['context']
    outcome = context.get('authority_outcome')
    candidate = context.get('candidate') or {}
    proposal = context.get('candidate_proposal') or {}
    provenance = proposal.get('provenance') or {}
    details = (context.get('match') or {}).get('details') or {}
    discard = {'intent': 'Discard'}
    model = (proposal.get('model_options') or {}).get('model') or (proposal.get('payload') or {}).get('model') or proposal.get('model')
    authority_model = (outcome or {}).get('model') if isinstance(outcome, dict) else None
    fast = model != authority_model if model else bool(provenance.get('uses_candidate_model'))
    accepted = resolution == 'accepted'
    if accepted and not fast:
        action = {'intent': 'Promote'}
        if isinstance(outcome, dict) and not details.get('matched_tool_calls'):
            action['notification'] = _verification_notification(
                outcome, policy_config.get('verification_output_chars', 20_000))
        return {'actions': [action]}

    if resolution in ('extent_reached', 'indeterminate') and outcome is None and not candidate.get('yield_only'):
        return {'actions': []}

    actions = [entry for entry in context.get('candidate_results', []) if entry.get('tool_name')]
    safe = details.get('safe_results')
    if safe is None:
        safe = _match_model(proposal, outcome, context, policy_config).get('details', {}).get('safe_results', [])
    branch = candidate.get('branch') or {}
    stopped = (resolution in ('preempted', 'extent_reached')
               or branch.get('status') == {'blocked': 'extent_reached'}
               or (branch.get('execution_state') or {}).get('turn_active') is False)
    if not accepted and not stopped and candidate.get('speculation_kind') == 'model_inference':
        explore = provenance.get('kind') != 'active_post_edit_test'
        active_test = any(map(_verification_entry, candidate.get('active_tools') or []))
        if explore or candidate.get('yield_only') or active_test:
            return {'actions': [{'intent': 'Yield', 'results': safe}]}
    files = _verified_files(actions, context.get('workspace'), policy_config) if fast else []
    results = files + safe
    if not results:
        return {'actions': [discard]}
    return {'actions': [{'intent': 'Salvage', 'results': results}] + ([] if accepted else [discard])}
