"""Speculate on local software verification using recent tool results."""

def _shell_lines(command):
    import re

    if not isinstance(command, str):
        return
    # Consume multiple heredocs in declaration order, ignoring quoted shell text.
    declarations = re.compile(
        r'''\\.|'[^']*'|"(?:\\.|[^"\\])*"|(?<!\S)#.*|<<<|<<(?P<tabs>-?)\s*(?P<quote>['"]?)(?P<end>[A-Za-z_]\w*)(?P=quote)(?![\w'"])''')
    pending = []
    for line in command.splitlines():
        if pending:
            end, tabs = pending[0]
            if (line.lstrip('\t') if tabs else line) == end:
                pending.pop(0)
            continue
        pending = [(match['end'], match['tabs']) for match in declarations.finditer(line) if match['end']]
        yield line


def _commands(command):
    import re
    import shlex

    # Quoted, escaped, and redirection punctuation is not a terminator.
    operators = re.compile(
        r'''\\.|'[^']*'|"(?:\\.|[^"\\])*"|(?<!\S)#.*|[<>]&|&>>?|(?P<op>&&|\|\||\|&|[;&|])''')
    commands = []
    pipeline = []
    for line in _shell_lines(command):
        parsed = []
        try:
            start = 0
            separators = [(m.start(), m.end(), m['op']) for m in operators.finditer(line) if m['op']]
            for end, next_start, op in separators + [(len(line), len(line), None)]:
                words = shlex.split(line[start:end], comments=True)
                while words and re.match(r'^\w+=', words[0]):
                    words.pop(0)
                parsed.append((words, op))
                start = next_start
        except ValueError:
            pipeline = []
            continue
        for words, op in parsed:
            if words:
                pipeline.append((words, op))
            if op in ('|', '|&') or (op is None and not words):
                continue
            commands.extend((segment, '&' if op == '&' else terminator) for segment, terminator in pipeline)
            pipeline = []
    return commands + pipeline


def _name(word):
    return word.rsplit('/', 1)[-1]


def _unwrap(words):
    import re

    option_values = {
        'env': ('-u', '--unset'),
        'timeout': ('-k', '-s', '--kill-after', '--signal'),
        'nice': ('-n', '--adjustment'),
        'time': ('-f', '-o', '--format', '--output'),
        'nohup': (),
        'stdbuf': ('-i', '-o', '-e', '--input', '--output', '--error'),
        'unshare': (),
    }
    while words:
        name = _name(words[0])
        if name in ('uv', 'poetry', 'pipenv') and words[1:2] == ['run']:
            words = words[2:]
            if words[:1] == ['--']:
                words = words[1:]
            continue
        if name not in option_values:
            return words
        index = 1
        while index < len(words):
            word = words[index]
            if word == '--':
                index += 1
                break
            if not word.startswith('-') or word == '-':
                break
            index += 2 if word in option_values[name] else 1
        if name == 'env':
            while index < len(words) and re.match(r'^\w+=', words[index]):
                index += 1
        if name == 'timeout':
            index += 1  # duration
        words = words[index:]
    return words


def _python_runner(args):
    no_value = set('BusSEOqvIbdPRx')
    index = 0
    while index < len(args):
        word = args[index]
        index += 1
        if word.startswith('-c') or word == '-':
            return []
        if word in ('-m', '--'):
            return args[index:]
        if word.startswith('-m'):
            return [word[2:], *args[index:]]
        if word in ('-W', '-X', '--check-hash-based-pycs'):
            index += 1
        elif word.startswith(('-W', '-X', '--check-hash-based-pycs=')):
            continue
        elif not word.startswith('-'):
            return args[index - 1:]
        elif not set(word[1:]) <= no_value:
            return []
    return []


def _verification(command):
    import re

    runners = {
        'pytest', 'py.test', 'runtests', 'runtests.py', 'unittest', 'nosetests', 'tox', 'nox',
        'vitest', 'jest', 'mocha', 'ctest', 'rspec', 'mypy', 'pyright', 'ruff', 'flake8',
        'eslint', 'tsc',
    }
    subcommands = {
        'cargo': {'test', 'check', 'clippy', 'build'},
        'npm': {'test', 'build', 'check', 'lint'}, 'pnpm': {'test', 'build', 'check', 'lint'},
        'yarn': {'test', 'build', 'check', 'lint'}, 'go': {'test', 'build', 'vet'},
        'mvn': {'test', 'verify'}, 'gradle': {'test', 'check', 'build'},
        'gradlew': {'test', 'check', 'build'}, 'make': {'test', 'check', 'build'},
    }

    for segment, terminator in _commands(command):
        if terminator == '&':
            continue
        words = _unwrap(segment)
        if not words or any(flag in words for flag in ('--help', '-h', '--version', '--collect-only')):
            continue
        name, *args = words
        name = _name(name)
        if re.fullmatch(r'python(?:[23](?:\.\d+)?)?', name):
            invocation = _python_runner(args)
            if not invocation:
                continue
            name, *args = invocation
            name = _name(name)
        if name in runners:
            return True
        if name in ('npm', 'pnpm', 'yarn') and args[:1] == ['run']:
            args = args[1:]
        if args and args[0] in subcommands.get(name, ()):
            return True
    return False


def _external(command):
    import re

    for segment, _ in _commands(command):
        words = _unwrap(segment)
        if not words:
            continue
        name, *args = words
        name = _name(name)
        if ((name == 'git' and 'push' in args) or (name == 'gh' and 'pr' in args)
                or (name == 'twine' and 'upload' in args)
                or (name in ('npm', 'pnpm', 'yarn') and 'publish' in args)):
            return True
        if name == 'curl' and any(
            re.fullmatch(r'(?:(?:-X|--request=)?(?:POST|PUT|PATCH|DELETE)|-d.*|--data.*|--upload-file.*|-T.*)', word)
            for word in args
        ):
            return True
    return False


def _mutation(command):
    import re

    if not isinstance(command, str):
        return False, []

    paths = set()
    writes = re.compile(
        r'''\b(?:apply_patch|write_text|write_bytes)\b|\bgit\s+apply\b|(?:^|[;&|\s])patch\s+|'''
        r'''\bopen\s*\(\s*[^,\n]+,\s*(?:mode\s*=\s*)?['"][rwaxbt+]*[wax+][rwaxbt+]*['"]''', re.M)
    mutates = bool(writes.search(command))
    file_path = r'[\w.@+-]+(?:/[\w.@+-]+)*\.[\w+-]+'
    for match in re.finditer(r'''(['"])(?P<path>''' + file_path + r''')\1''', command):
        paths.add(match['path'])
    for match in re.finditer(r'^\*{3} (?:Update File:|Add File:|Delete File:|Move to:)\s*([^\n]+)', command, re.M):
        paths.add(match[1].strip())

    for segment, _ in _commands(command):
        words = _unwrap(segment)
        if words and _name(words[0]) in ('sed', 'perl') and any(
            option.startswith('--in-place') or re.match(r'^-[A-Za-z]*i', option)
            for option in words[1:]
        ):
            mutates = True
            paths.update(word for word in words[1:] if re.fullmatch(file_path, word))

    redirects = re.compile(r'''\\.|'[^']*'|"(?:\\.|[^"\\])*"|(?<!\S)#.*|>&(?:\s*\d+|\s*-)|(?P<write>>>?\|?\s*(?:'(?P<single>[^']+)'|"(?P<double>[^"]+)"|(?P<bare>[^\s;&|<>]+)))''')
    for line in _shell_lines(command):
        for match in redirects.finditer(line):
            if not match['write']:
                continue
            target = match['single'] or match['double'] or match['bare']
            if target == '/dev/null' or (target.endswith('.log') and '$' not in target):
                continue  # Common output sinks for verification commands.
            mutates = True
            paths.add(target)
    return mutates, sorted(path for path in paths if not path.startswith('/') and '..' not in path.split('/'))


def opportunity_pass(data, *, policy_config, capabilities):
    op = data['op']
    return op['produces_result'] and op['result_pending'] and op['kind'] == 'bash'


def feasibility_pass(data, *, policy_config, capabilities):
    command = data['op'].get('arguments', {}).get('command')
    return (data['state']['assumption_depth'] < policy_config['max_assumption_depth']
            and _verification(command) and not _external(command) and not _mutation(command)[0])


def select(data, *, policy_config, capabilities):
    available = max(0, policy_config['max_active_candidates'] - data['scheduling_context']['active_candidate_count'])
    frontier = sorted(data['frontier_view'], key=lambda chance: chance['assumption_depth'])
    return [chance['chance_id'] for chance in frontier[:available]]


def launch(data, *, policy_config, capabilities):
    return True


def _test_path(path):
    import re

    parts = path.lower().replace('\\', '/').split('/')
    name = parts[-1]
    return (any(part in ('test', 'tests', 'spec', 'specs', '__tests__') for part in parts[:-1])
            or bool(re.search(r'^test[_\.]|(?:_test|_spec|\.test|\.spec)\.', name)))


def predict(data, *, policy_config, capabilities):
    command = data['chance_context']['arguments'].get('command')
    if not _verification(command) or _external(command):
        return {'abstain': True, 'reason': 'unrecognized_command'}
    edited_source = edited_tests = False
    for entry in reversed(data['history']):
        arguments = entry.get('arguments') or {}
        tool = entry.get('tool_name')
        if tool == 'bash':
            previous = arguments.get('command')
            if _verification(previous):
                if (entry.get('status') == 'succeeded' and entry.get('is_error') is not True
                        and not any(_verification_failures(_output(entry)))):
                    break
                continue
            mutates, paths = _mutation(previous)
            if not mutates:
                continue
        elif tool in ('edit', 'write') and isinstance(arguments.get('path'), str):
            paths = [arguments['path']]
        else:
            continue
        if entry.get('status') != 'succeeded':
            continue
        for path in paths:
            if _test_path(path):
                edited_tests = True
            else:
                edited_source = True
    failure = edited_tests and not edited_source
    if failure:
        kind, phase = 'expected_test_failure', 'tdd_repro'
        text = 'Predicted verification result: the new regression test fails as expected.'
    else:
        kind = 'verification_pass'
        phase = 'post_source_edit' if edited_source else 'baseline_check'
        text = 'Predicted verification result: the command completes successfully.'
    return {'generation': 'batch', 'proposals': [{
        'payload': {'content': [{'type': 'text', 'text': text}], 'isError': failure},
        'provenance': {'kind': kind, 'phase': phase},
    }]}


def extent(data, *, policy_config, capabilities):
    state = data['process_state']
    if state['steps_completed'] >= policy_config['max_steps']:
        return 'stop'
    if state.get('result_facts', {}).get('last_action', {}).get('is_error') is True:
        return 'stop'
    action = state.get('semantic_facts', {}).get('next_action', {})
    if action.get('name') == 'bash' and _verification(action.get('arguments', {}).get('command')):
        return 'stop'
    return 'continue'


def effect(data, *, policy_config, capabilities):
    action = data['effect_descriptor']
    if action.get('tool') == 'bash' and _external((action.get('arguments') or {}).get('command')):
        return {'mode': 'block', 'reason': 'external_effect_boundary'}
    return {'mode': 'allow'}


def rebalance(data, *, policy_config, capabilities):
    active = data['active_view']
    actions = [{'branch_id': branch['branch_id'], 'action': 'preempt'}
               for branch in active if branch['tool_counts']['failed'] >= 2]
    available = max(0, policy_config['max_active_candidates'] - len(active) + len(actions))
    paused = sorted(data['paused_view'], key=lambda branch: -branch['age_ms'])
    return actions + [{'branch_id': branch['branch_id'], 'action': 'resume'} for branch in paused[:available]]


def _output(outcome):
    if not isinstance(outcome, dict):
        return outcome if isinstance(outcome, str) else ''
    content = outcome.get('content')
    if isinstance(content, list):
        return '\n'.join(part['text'] for part in content if part.get('type') == 'text')
    if isinstance(content, str):
        return content
    return str(outcome.get('payload') or '')


def _verification_failures(text):
    import re

    execution_failure = re.search(
        r'\b(?:SyntaxError|IndentationError|ImportError|ModuleNotFoundError|command not found|'
        r'No such file|Permission denied|segmentation fault|fatal error|error collecting)\b|^\s*fatal:',
        text, re.I | re.M)
    failed_suite = re.search(
        r'\bFAILED\s*\(|\b[1-9]\d*\s+(?:failed|errors?)\b|^\s*(?:FAIL|ERROR):\s+\w+|\bAssertionError\b',
        text, re.I | re.M)
    return execution_failure, failed_suite, re.search(r'\b(?:TypeError|AttributeError):', text)


def match_proposal(data, *, policy_config, capabilities):

    kind = (data['proposal'].get('provenance') or {}).get('kind')
    evidence = data.get('evidence') or {}
    outcome = evidence.get('outcome') or data.get('outcome')
    if not isinstance(outcome, dict):
        return {'verdict': 'indeterminate'} if outcome is None else {'verdict': 'incompatible', 'score': 0.0}
    status = outcome.get('status') or data['match_context']['authority'].get('status')
    is_error = outcome.get('is_error')
    execution_failure, failed_suite, test_exception = _verification_failures(_output(outcome))
    if not execution_failure:
        if kind == 'verification_pass' and status == 'succeeded' and is_error is False and not failed_suite:
            return {'verdict': 'compatible', 'score': 1.0}
        if kind == 'expected_test_failure' and (is_error is True or failed_suite or test_exception):
            return {'verdict': 'compatible', 'score': 0.9}
    return {'verdict': 'incompatible', 'score': 0.0}


def dispose(data, *, policy_config, capabilities):
    resolution, context = data['resolution'], data['context']
    outcome = context.get('authority_outcome')
    if resolution == 'accepted':
        summary = '\n'.join(_output(outcome).strip().splitlines()[-20:])[-1200:]
        return {'actions': [{'intent': 'Promote', 'notification':
                             'Real verification command finished matching prediction:\n' + summary}]}
    if resolution in ('extent_reached', 'indeterminate') and outcome is None:
        return {'actions': []}
    actions = []
    if policy_config.get('salvage_independent_results'):
        safe = (context.get('match') or {}).get('details', {}).get('safe_result_ids', [])
        results = [{'id': action_id, 'notification': 'Independent candidate result transferred.'}
                   for action_id in dict.fromkeys(safe)]
        if results:
            actions.append({'intent': 'Salvage', 'results': results})
    return {'actions': actions + [{'intent': 'Discard'}]}
