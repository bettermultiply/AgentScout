import runpy
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CODING_POLICY = ROOT / 'assets/policy/coding'
sys.path.insert(0, str(ROOT / 'policy_lib'))
from ascout_policy.info_match import (directional_overlap, extract_info_set, filter_novel_readonly,
                                      is_readonly_action, match_cumulative_info, match_tool_calls)


def bash(command, **args):
    return {'name': 'bash', 'arguments': {'command': command, **args}}


def read(path, **args):
    return {'name': 'read', 'arguments': {'path': path, **args}}


class InfoMatchTests(unittest.TestCase):
    def compare(self, candidate, authority, kind, coverage):
        result = match_tool_calls(candidate, authority)
        self.assertEqual(result['kind'], kind, result)
        self.assertEqual(result['coverage'], coverage)
        self.assertEqual(result['matched'], kind in ('exact', 'info_equivalent'))
        self.assertEqual(result['score'], 0.95 if kind == 'info_equivalent' else round(coverage, 3))

    def test_single_readonly_action_and_cumulative_coverage(self):
        for action in (read('f'), bash('cat f'), bash('git status')):
            self.assertTrue(is_readonly_action(action['name'], action['arguments']))
        for action in (bash('cd src'), bash('cat f > x'), bash('cat f; touch x'),
                       {'name': 'edit', 'arguments': {'path': 'f'}}, read(None)):
            self.assertFalse(is_readonly_action(action['name'], action['arguments']))
        completed = [read('f', limit=2), bash('cat f > x'), read('f', offset=3, limit=2), None]
        result = match_cumulative_info(completed, [read('f', limit=5)])
        self.assertEqual((result['kind'], result['coverage']), ('partial_info', 0.8))
        self.assertEqual(match_cumulative_info(completed, [read('f', limit=4)])['kind'], 'info_equivalent')
        self.assertEqual(match_cumulative_info(completed, [bash('cat f > x')])['coverage'], 0)

    def test_exact_normalization(self):
        self.compare([read('${PWD}/./a//b'), bash(' LC_ALL=C PYTHONDONTWRITEBYTECODE=1 cat f ', timeout=5)],
                     [{'function': {'name': 'bash', 'arguments': '{"command":"cat f"}'}}, read('a/b')], 'exact', 1)
        self.compare([read('~/a//b')], [read('~/a/./b')], 'exact', 1)
        self.compare([read('~/a')], [read('a')], 'mismatch', 0)
        self.compare([bash('cargo test'), read('f')], [read('f'), bash('cargo test')], 'mismatch', 0)
        self.compare([bash('cargo test')], [bash('cargo test')], 'exact', 1)
        self.compare([], [], 'mismatch', 0)
        self.compare([None], [None], 'mismatch', 0)

    def test_ranges_and_partial_coverage(self):
        calls = [bash("sed -n -e '2,4p;8p' -e '9,10p' f; head -n 2 ./f; tail -n +11 f"),
                 bash('nl x; cat y z; head -6 w; head v')]
        self.assertEqual(extract_info_set(calls).file_ranges, {
            'f': ((1, 4), (8, None)), 'x': ((1, None),), 'y': ((1, None),),
            'z': ((1, None),), 'w': ((1, 6),), 'v': ((1, 10),),
        })
        self.compare([read('./f', offset=1, limit=10)], [bash("sed -n '2,4p' f")], 'info_equivalent', 1)
        self.compare(calls, [read('f', limit=10)], 'partial_info', 0.7)
        self.compare(calls, [read('f')], 'mismatch', 0)
        self.compare(calls, [read('f', offset=8)], 'info_equivalent', 1)
        self.compare([bash('cd src && head -4 $PWD/f')], [read('src/f', limit=4)], 'info_equivalent', 1)

    def test_searches(self):
        self.compare([bash("rg -nHi -g '*.py' 'one|two|three' .")],
                     [bash("rg -i --glob '*.py' 'two|one' src")], 'info_equivalent', 1)
        self.compare([bash("grep -rEn -e one -e two .")], [bash("egrep -n 'two|one' src/f")], 'info_equivalent', 1)
        self.compare([bash("git grep -nE 'one|two' -- src")],
                     [bash('git grep -E -e two -- src/a')], 'info_equivalent', 1)
        self.compare([bash("rg 'one\\|two' src")], [bash('rg one src')], 'mismatch', 0)
        self.compare([bash("fgrep 'one|two' src/f")], [bash('fgrep one src/f')], 'mismatch', 0)
        self.compare([bash('rg one src')], [bash('rg -i one src')], 'mismatch', 0)
        self.compare([bash('LC_ALL=C grep -ri one src')], [bash('grep -ri one src')], 'mismatch', 0)
        self.compare([read('src/a'), read('src/b')], [bash('rg one src/a src/b')], 'info_equivalent', 1)
        self.compare([read('src/a', limit=10)], [bash('rg one src/a src/b')], 'mismatch', 0)

    def test_directional_overlap_does_not_equate_searches_or_partial_reads(self):
        candidate = [bash('grep -rn "delete_cookie" src/sub'), read('src/sub/a', limit=2)]
        search = [bash(r"rg -n 'delete_cookie\(' src")]
        self.compare(candidate, search, 'mismatch', 0)
        self.assertFalse(match_cumulative_info(candidate, search)['matched'])
        self.assertEqual(directional_overlap(candidate, search), 1.0)
        self.assertEqual(directional_overlap(candidate, search + [read('src/sub/a', limit=4)]), 0.75)
        self.assertEqual(directional_overlap(candidate, [read('src/sub/a')]), 1.0)
        self.assertEqual(directional_overlap(candidate, [read('src/sub/a', offset=3)]), 0.0)
        self.assertEqual(directional_overlap(candidate, [bash('rg delete_cookie other')]), 0.0)
        self.assertEqual(directional_overlap(candidate, [bash('rg cookie src')]), 0.0)
        self.assertEqual(directional_overlap([bash("git grep -n 'def classproperty'")],
                                             [bash("rg 'classmethod|classproperty' src")]), 1.0)

    def test_git_status_full_covers_short_only_with_matching_context(self):
        full = [bash('cd src && LC_ALL=C git status -- file')]
        short = [bash('cd src && LC_ALL=C git status -s -- file')]
        self.compare(full, short, 'info_equivalent', 1)
        self.compare(short, full, 'mismatch', 0)
        self.assertEqual(directional_overlap([bash('git status')], [bash('git status --short')]), 1.0)
        for command in ('LC_ALL=C git status -s -- file', 'cd src && git status -s -- file',
                        'cd src && LC_ALL=C git status -s -- other',
                        'cd src && LC_ALL=C git status --porcelain -- file',
                        'cd src && LC_ALL=C git status -s --untracked-files=no -- file'):
            self.compare(full, [bash(command)], 'mismatch', 0)

    def test_observations(self):
        self.compare([bash('git status -s; git diff --stat -- ./f; git log --oneline -3; ls -R src; find ./src -print')],
                     [bash('git status --short && git diff --stat -- f && git log --oneline -3'),
                      bash('ls src/lib; find src')], 'info_equivalent', 1)
        self.compare([bash('ls src')], [bash('ls src/lib')], 'mismatch', 0)
        self.compare([bash('git diff --cached')], [bash('git diff')], 'mismatch', 0)
        self.compare([bash('ls src | head -5')], [bash('ls src')], 'mismatch', 0)
        self.compare([bash('cat f | head -5')], [read('f', limit=5)], 'mismatch', 0)
        self.compare([bash("rg --files --glob '*.py' ./src")],
                     [bash("rg --files -g '*.py' src")], 'info_equivalent', 1)
        self.assertTrue(extract_info_set([bash('ls src | tail -15')]).pure_readonly)

    def test_workspace_alignment(self):
        absolute = '/tmp/run/workspace/pkg/sub/module.py'
        authority = [bash("sed -n '2,5p' pkg/sub/module.py"), bash('rg token pkg/sub')]
        candidate = [bash("cd /tmp/run/workspace && sed -n '2,5p' pkg/sub/module.py && rg token pkg/sub")]
        self.compare(candidate, authority, 'info_equivalent', 1)
        self.compare(authority, candidate, 'info_equivalent', 1)
        self.compare([read(absolute)], [read('pkg/sub/module.py')], 'info_equivalent', 1)
        self.assertTrue(match_cumulative_info(candidate, authority)['matched'])
        entry = {'tool_name': 'read', 'arguments': {'path': absolute}}
        self.assertEqual(filter_novel_readonly([entry], [read('pkg/sub/module.py')]), [])
        self.compare([read(absolute), read('/etc/hosts'), read('~/pkg/sub/module.py')],
                     [read('pkg/sub/module.py'), read('hosts')], 'partial_info', 0.5)
        self.compare([read('/one/pkg/a'), read('/two/pkg/b')],
                     [read('pkg/a'), read('pkg/b')], 'mismatch', 0)
        self.compare([read('/etc/hosts')], [read('etc/hosts')], 'mismatch', 0)

    def test_readonly_separators_and_environment_observations(self):
        self.compare([bash("sed -n '1,2p' a && echo '---' && printf '%s' '|' && pwd && true && cat b")],
                     [read('a', limit=2), read('b')], 'info_equivalent', 1)
        self.assertFalse(is_readonly_action('bash', {'command': 'echo harmless && true'}))
        self.compare([bash('python --version; python3 --version; git remote -v; git branch -a; git tag -l --contains HEAD')],
                     [bash('git branch --all; git remote -v; python3 --version; python --version; git tag --list --contains HEAD')],
                     'info_equivalent', 1)
        for command in ('git branch new', 'git branch -D old', 'git tag new', 'git tag -d old',
                        'git remote remove origin', 'printf -v PWD /etc && cat f', 'echo $(cat f)', 'echo hi > x'):
            self.assertFalse(extract_info_set([bash(command)]).pure_readonly, command)

    def test_search_options_globs_and_limiters(self):
        self.compare([bash('rg -n -A31 -B2 --context=3 --no-heading --color=never token src')],
                     [bash('rg --after-context 1 --before-context 2 token src')], 'info_equivalent', 1)
        self.compare([bash('rg -lSw -tpy -m4 token tests/test_sample*.py')],
                     [bash('rg --files-with-matches --smart-case --word-regexp --type py --max-count=4 token tests/test_sample*.py')],
                     'info_equivalent', 1)
        self.compare([bash('rg -w token src')], [bash('rg token src')], 'mismatch', 0)
        self.compare([bash('rg -m1 token src')], [bash('rg token src')], 'mismatch', 0)
        self.compare([bash('grep -rn --include=*.py token src')],
                     [bash("grep -r --include='*.py' token src")], 'info_equivalent', 1)
        self.assertTrue(extract_info_set([bash('ls tests/test_sample*.py')]).pure_readonly)
        self.compare([bash('rg token src/{first,second}.py')],
                     [bash('rg token src/first.py src/second.py')], 'info_equivalent', 1)
        for command in ('rg token src', 'grep -r token src', 'ls src', 'git log --oneline'):
            limited = bash(command + ' | head -80 | tail -2')
            self.assertTrue(is_readonly_action('bash', limited['arguments']))
            self.compare([limited], [bash(command)], 'mismatch', 0)
            self.assertFalse(match_cumulative_info([limited], [bash(command)])['matched'])
            self.assertEqual(directional_overlap([limited], [bash(command)]), 1.0)
        for command in ('rg pat* src', 'rg token src/{1..4}', 'rg token src/{a,$VAR}', 'rg token src/$DIR*.py',
                        'grep token --include=$(touch x)', 'rg token src | tail -f', 'ls src/* >> log'):
            self.assertFalse(extract_info_set([bash(command)]).pure_readonly, command)

    def test_filter_novel_readonly(self):
        candidate = [{'tool_name': 'read', 'arguments': {'path': 'same'}},
                     {'tool_name': 'read', 'arguments': {'path': 'extra', 'limit': 3}}]
        self.assertEqual(filter_novel_readonly(candidate, [read('same')]), [candidate[1]])
        full = [{'tool_name': 'read', 'arguments': {'path': 'f'}}]
        self.assertEqual(filter_novel_readonly(full, [read('f', limit=2)]), full)
        self.assertEqual(filter_novel_readonly(full, [bash('cargo test')]), full)

    def test_interval_difference_crops_output(self):
        for start, end, covered_start, covered_end, first, last in (
                (735, 805, 741, 805, 735, 740), (1, 200, 1, 125, 126, 200)):
            for action in (read('f', offset=start, limit=end - start + 1),
                           bash(f"sed -n '{start},{end}p' f")):
                entry = {'tool_name': action['name'], 'arguments': action['arguments'],
                         'result': {'content': '\n'.join(map(str, range(start, end + 1)))}}
                authority = [read('f', offset=covered_start, limit=covered_end - covered_start + 1)]
                result, = filter_novel_readonly([entry], authority)
                self.assertEqual(result['cropped_ranges'], [(first, last)])
                self.assertEqual(result['cropped_output'].splitlines(), list(map(str, range(first, last + 1))))
                self.assertEqual(filter_novel_readonly([entry], [read('f')]), [])

    def test_search_hit_difference(self):
        command = bash('rg -n needle src')
        entry = {'tool_name': 'bash', 'arguments': command['arguments'],
                 'result': {'content': 'src/f:1:needle\nsrc/f:5:needle\nsrc/g:9:needle'}}
        result, = filter_novel_readonly([entry], [read('src/f', limit=1)],
                                      authority_entries=[{'result': {'content': 'src/g:9:needle'}}])
        self.assertEqual(result['cropped_output'], 'src/f:5:needle')
        self.assertEqual(filter_novel_readonly([entry], [command]), [])

    def test_verification_equivalence(self):
        from ascout_policy.info_match import match_verification_calls, normalize_verification_command

        wrapped = 'cd /tmp/work && TMPDIR=/tmp PYTHONDONTWRITEBYTECODE=1 python3 -m pytest --basetemp=/tmp/x -q tests/a.py -k smoke'
        plain = 'pytest tests/a.py -k smoke'
        self.assertEqual(normalize_verification_command(wrapped), normalize_verification_command(plain))
        def matches(candidate, required):
            return match_verification_calls([bash(candidate)], [bash(required)])['matched']
        self.assertTrue(matches(wrapped, plain))
        self.assertTrue(matches('pytest tests/a.py tests/b.py -k smoke', plain))
        self.assertFalse(matches('pytest --basetemp /tmp/x tests/b.py -k smoke', plain))
        self.assertFalse(matches('pytest tests/a.py', plain))
        self.assertFalse(matches('mkdir -p temp && ' + plain, plain))
        self.assertFalse(match_verification_calls([bash(plain)], [bash(plain), bash('git diff --check')])['matched'])

    def test_stage_3b_wrapped_verification_and_capability_checks(self):
        from ascout_policy.info_match import is_verification_command, match_verification_calls

        plain = 'HOME="$PWD" TMPDIR="$PWD" python tests/runtests.py suite'
        wrapped = 'cd /workspace/project/tests && python runtests.py suite 2>&1 | tail -15'
        for command in (plain, wrapped):
            self.assertTrue(is_verification_command(command))
        for candidate, authority in ((plain, wrapped), (wrapped, plain)):
            self.assertTrue(match_verification_calls([bash(candidate)], [bash(authority)])['matched'])
        for command in ('pytest >log | tail', 'pytest | tail -15 log', 'pytest | head | tail',
                        'HOME="$OTHER" pytest', 'HOME="$(pwd)" pytest'):
            self.assertFalse(is_verification_command(command), command)
        checks = "sed -n '1,3p' src.py; type apply_patch; command -v python; which pip"
        self.assertTrue(is_readonly_action('bash', {'command': checks}))
        self.assertTrue(is_readonly_action('bash', {'command': 'type apply_patch; command -v python'}))
        self.assertGreater(directional_overlap([bash("sed -n '1,3p' src.py")], [bash(checks)]), 0)

    def test_yield_deduplicates_and_uses_neutral_headers(self):
        policy = runpy.run_path(str(CODING_POLICY / 'decisions.py'))
        outcome = {'tool_calls': [read('base')]}
        entries = [dict(action_id='read', tool_name='read', arguments={'path': 'novel'},
                        status='succeeded', result={'content': 'workspace report'}),
                   dict(action_id='test', tool_name='bash', arguments={'command': 'pytest'},
                        status='failed', result={'content': 'verification report'})]
        candidate = {'speculation_kind': 'model_inference', 'yield_only': True,
                     'active_tools': [entries[1]], 'yielded_action_ids': []}
        proposal = {'provenance': {'kind': 'active_post_edit_test'}}
        data = {'proposal': proposal, 'outcome': outcome, 'match_context': {
            'candidate': candidate, 'candidate_results': entries}}
        for intent, resolution in (('Salvage', 'preempted'), ('Yield', 'rejected')):
            result = policy['match_proposal'](data, policy_config={}, capabilities={})
            context = {'match': result, 'authority_outcome': outcome, 'candidate': candidate,
                       'candidate_results': entries}
            plan = policy['dispose']({'resolution': resolution, 'context': context},
                                     policy_config={}, capabilities={})
            self.assertEqual(plan['actions'][0]['intent'], intent)
            self.assertEqual({entry['id'] for entry in plan['actions'][0]['results']}, {'read', 'test'})
            for entry in plan['actions'][0]['results']:
                self.assertNotIn('read-only', entry['notification'].splitlines()[0])
        candidate['yielded_action_ids'] = ['read']
        result = policy['match_proposal'](data, policy_config={}, capabilities={})
        self.assertEqual([entry['id'] for entry in result['details']['safe_results']], ['test'])

    def test_observable_degradation(self):
        from ascout_policy.degradation import should_stop_degraded

        def completed(action, **kwargs):
            return {**action, 'status': 'succeeded', **kwargs}
        whole = completed(read('src/a.py', limit=10))
        first = completed(read('src/a.py', limit=3))
        second = completed(read('src/a.py', offset=4, limit=3))
        self.assertEqual(should_stop_degraded([whole, first, second]), (True, 'stagnant_progress'))
        self.assertEqual(should_stop_degraded([first, second]), (False, None))
        self.assertEqual(should_stop_degraded([first, second], read('./src/a.py', limit=3)), (True, 'repeated_action'))
        self.assertEqual(should_stop_degraded([whole, whole]), (True, 'repeated_action'))
        self.assertEqual(should_stop_degraded([completed(read('a'), status='failed'),
                                               completed(read('b'), is_error=True)]), (True, 'consecutive_errors'))

    def test_filter_drops_observations_and_keeps_novel_reads_and_searches(self):
        observations = [{'tool_name': 'bash', 'arguments': {'command': command}}
                        for command in ('git status', 'ls', 'git log', 'python --version')]
        novel = [{'tool_name': 'read', 'arguments': {'path': 'extra'}},
                 {'tool_name': 'bash', 'arguments': {'command': 'rg token src'}}]
        for authority in ([read('same')], [bash('cargo test')]):
            with self.subTest(authority=authority):
                self.assertEqual(filter_novel_readonly(observations + novel, authority), novel)

    def test_unsupported_commands_invalidate_entire_batch(self):
        commands = [
            'cat f > x', 'cat f >> x', 'cat < f', 'cat f 2>&1', 'cat f &', 'cat f || ls',
            'cat f | tee x', 'cat f; touch x', 'cat f\npytest', 'find . -delete', 'find . -exec touch x ;',
            'find . -fprint x', 'sed -ni 2p f', "sed -n '1p' f -i --", "sed -n '1w x' f", "sed -n '1e touch x' f",
            'git diff --output=x', 'git diff --ext-diff', 'git log --output=x', 'git push',
            'rg --pre=script one .', 'grep -f f .', 'cat $(touch x)', 'cat "$VAR"', 'cat "`touch x`"',
            'cat *.py', 'bash -c ls', 'PATH=/tmp cat f', './cat f', 'cat -', 'python -m unittest',
            'npm run build', "'LC_ALL=C' cat f", "rg 'one' f |", 'cat f &&',
        ]
        for command in commands:
            with self.subTest(command=command):
                batch = [read('f'), bash(command)]
                self.assertFalse(extract_info_set(batch).pure_readonly)
                self.compare(batch, [read('./f')], 'mismatch', 0)
        for name in ('write', 'edit', 'unknown'):
            self.assertFalse(extract_info_set([read('f'), {'name': name, 'arguments': {'path': 'f'}}]).pure_readonly)


class CodingPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.decisions = runpy.run_path(str(CODING_POLICY / 'decisions.py'))

    @staticmethod
    def action(id, name, args, output='', status='succeeded'):
        return {'action_id': id, 'tool_name': name, 'arguments': args, 'status': status,
                'result': {'content': output}}

    def test_scratch_writes_do_not_hide_later_reads(self):
        action = self.action
        after = action('read', 'read', {'path': 'sphinx/ext/autodoc.py'}, 'novel source')
        for command in ('ls sphinx/ext/autodoc/ 2>/dev/null && python -V', 'cd var 2>/dev/null',
                        'ls >/dev/null', 'ls 1> /dev/null', 'ls &>/dev/null', 'ls >> /dev/null',
                        'mkdir -p .test-tmp /tmp/sphinx; rm -rf .baseline/old /tmp/sphinx/old',
                        "cat > '.scratch/notes' <<'EOF'\n> this is data\nEOF",
                        'cat <<EOF >/tmp/sphinx-notes\ntext\nEOF',
                        'python -c \'from pathlib import Path; Path(".pytest-work/notes").write_text("ok")\''):
            with self.subTest(command=command):
                scratch = action('scratch', 'bash', {'command': command})
                self.assertIsNone(self.decisions['_mutation'](scratch, None))
                self.assertEqual(self.decisions['_readonly_results']([scratch, after], [], None), [after])
        for path in ('./.test-tmp/file', '/tmp/sphinx/file'):
            for tool_name in ('edit', 'write'):
                self.assertIsNone(self.decisions['_mutation'](action('scratch', tool_name, {'path': path}), None))
        for command in ('ls > output 2>/dev/null', 'mkdir -p .scratch src',
                        'cat > .scratch/../src.py', 'rm /tmp/a src.py', 'cat > .scratch.txt'):
            self.assertEqual(self.decisions['_mutation'](action('write', 'bash', {'command': command}), None), 'source')

    def test_large_readonly_outputs_are_delivered_whole(self):
        entries = [self.action(str(n), 'read', {'path': f'src/{n}.py'}, 'x' * n) for n in (9500, 15000)]
        result = self.decisions['_safe_results']
        for config in ({}, {'safe_result_chars': None, 'safe_result_count': None}):
            self.assertEqual([item['notification'].partition('\n')[2] for item in result(entries, config, set())],
                             [entry['result']['content'] for entry in entries])
        self.assertEqual(result(entries, {'safe_result_chars': 8000}, set()), [])
        self.assertEqual(len(result(entries, {'safe_result_count': 1}, set())), 1)

    def test_failed_or_delivered_verification_covers_unchanged_files(self):
        action = self.action
        baseline = action('baseline', 'bash', {'command': 'pytest tests'}, '1 passed')
        edit = {**action('edit', 'edit', {'path': 'src.py'}), 'timestamp_ms': 50}
        failed = action('failed', 'bash', {'command': 'pytest tests'}, 'AssertionError: expected value\n1 failed', 'failed')
        notice = {'role': 'custom', 'custom_type': 'notification', 'content':
                  'Accepted result: [Speculative verification output collected on current files]: bash pytest tests\n1 failed'}
        debug = action('rg', 'bash', {'command': 'rg broken src.py'})

        def predict(history, chance=None, process=None):
            return self.decisions['predict']({'history': history, 'chance_context': chance or {'kind': 'model_inference'},
                                              'process_context': process or {}}, policy_config={}, capabilities={})

        for verification in (failed, notice):
            history = [baseline, edit, verification, debug]
            self.assertEqual(predict(history).get('predict'), 'branch_model')
            self.assertEqual(predict(history + [edit])['proposals'][0]['provenance']['kind'], 'active_post_edit_test')
            self.assertEqual(predict(history + [edit], {'kind': 'bash', 'arguments': failed['arguments']})
                             ['proposals'][0]['provenance']['phase'], 'post_source_edit')
            context = {'candidate': {'speculation_kind': 'model_inference'}, 'candidate_results': [failed],
                       'authority_history': history}
            data = {'proposal': {'provenance': {'kind': 'active_post_edit_test'}},
                    'outcome': {'tool_calls': [read('src.py')]}, 'match_context': context}
            self.assertEqual(self.decisions['match_proposal'](data, policy_config={}, capabilities={})
                             ['details']['safe_results'], [])
            context['authority_history'] = history + [edit]
            self.assertEqual(len(self.decisions['match_proposal'](data, policy_config={}, capabilities={})
                                 ['details']['safe_results']), 1)
        for invalid in ({**failed, 'result': {'content': 'pytest: command not found'}},
                        {**failed, 'started_at_ms': 40, 'timestamp_ms': 60},
                        {**failed, 'arguments': {'command': 'pytest -k smoke'}},
                        {**notice, 'role': 'assistant'}):
            self.assertEqual(predict([baseline, edit, invalid, debug])['proposals'][0]['provenance']['kind'],
                             'active_post_edit_test')
        live = {'proposal_provenance': {'kind': 'active_post_edit_test'}}
        self.assertTrue(predict([baseline, edit], process={'active_branches': [live]})['abstain'])

    def test_verify_extent_allows_only_missing_post_edit_verification(self):
        state = {'steps_completed': 1, 'phase': 'tool_execution', 'semantic_facts': {
            'speculation_kind': 'tool_execution', 'next_action': bash('pytest')}}
        mutation = {'tool_name': 'edit', 'arguments': {'path': 'src.py'}, 'status': 'succeeded'}
        passed = {'tool_name': 'bash', 'arguments': {'command': 'pytest'}, 'status': 'succeeded',
                  'result': {'content': '1 passed'}}
        for entries, expected in (([], 'stop'), ([mutation], 'continue'),
                                  ([mutation, passed], 'stop'), ([passed, mutation], 'continue')):
            state['result_facts'] = {'completed_tools': entries}
            self.assertEqual(self.decisions['extent']({'process_state': state}, policy_config={}, capabilities={}), expected)
        state['semantic_facts']['yield_only'] = True
        self.assertEqual(self.decisions['extent']({'process_state': state}, policy_config={}, capabilities={}), 'continue')
        state['phase'] = 'model_inference'
        self.assertEqual(self.decisions['extent']({'process_state': state}, policy_config={}, capabilities={}), 'stop')

    def test_fast_write_salvage_requires_verification_after_last_edit(self):
        def action(id, name, args, output='', status='succeeded'):
            return {'action_id': id, 'tool_name': name, 'arguments': args, 'status': status,
                    'result': {'content': output}}

        read_action = action('read', 'read', {'path': 'other.py'}, 'useful diagnostic')
        first = {**action('write', 'write', {'path': 'src.py', 'content': 'before = 1'}), 'timestamp_ms': 10}
        latest = action('edit', 'edit', {'path': './src.py', 'edits': [{'oldText': 'before', 'newText': 'after'}]})
        passed = action('pass', 'bash', {'command': 'pytest'}, '3 passed')
        bad_invocation = action('bad', 'bash', {'command': 'pytest'}, 'ImportError')
        failed_edit = action('failed-edit', 'edit', {'path': 'src.py'}, status='failed')
        outside_test = action('outside-test', 'bash', {'command': 'cd /tmp && pytest'}, '3 passed')
        outside_edit = action('outside-edit', 'write', {'path': '/tmp/app.py', 'content': 'x'})
        base = {'workspace': '/workspace', 'candidate_proposal': {'model_options': {'model': 'local/fast'}},
                'candidate': {'speculation_kind': 'model_inference'}}

        def dispose(resolution, entries, candidate=None):
            context = {**base, 'candidate_results': [read_action, *entries]}
            if candidate:
                context['candidate'] = {**base['candidate'], **candidate}
            return self.decisions['dispose']({'resolution': resolution, 'context': context}, policy_config={}, capabilities={})['actions']

        for entries in ([first], [first, passed, latest], [first, bad_invocation],
                        [first, failed_edit, passed], [first, outside_test], [outside_edit, passed],
                        [first, {**passed, 'started_at_ms': 9, 'timestamp_ms': 11}]):
            with self.subTest(entries=entries):
                self.assertEqual([r['id'] for r in dispose('accepted', entries)[0]['results']], ['read'])
        for resolution in ('accepted', 'extent_reached'):
            actions = dispose(resolution, [first, latest, passed], {'yield_only': resolution != 'accepted'})
            self.assertEqual([item['intent'] for item in actions], ['Salvage'] + (['Discard'] if resolution != 'accepted' else []))
            file, verification, readonly = actions[0]['results']
            self.assertEqual(file['id'], 'edit')
            self.assertNotIn('files', file)
            self.assertIn("'before' → 'after'", file['notification'])
            self.assertEqual(verification['files'], [])
            self.assertIn('3 passed', verification['notification'])
            self.assertEqual(readonly['files'], [])
        self.assertEqual(dispose('rejected', [first, latest, passed])[0]['results'][0]['id'], 'read')

    def assert_thinking_off(self, config):
        result = self.decisions['predict']({'chance_context': {'kind': 'model_inference'}},
                                           policy_config=config, capabilities={})
        self.assertEqual(result['thinking'], 'off')
        self.assertIsInstance(result['thinking'], str)

    def test_yaml_candidate_thinking_is_a_string(self):
        try:
            import yaml
        except ImportError:
            self.skipTest('PyYAML is unavailable')
        policy = yaml.safe_load((CODING_POLICY / 'policy.yaml').read_text(encoding='utf-8'))
        config = policy['decisions']['predict']['policy_config']
        self.assertEqual(config['candidate_thinking'], 'off')
        self.assertIs(config['share_inflight_commands'], False)
        guidance = config['fail_candidate_guidance']
        self.assertIn('failure output has not arrived yet', guidance)
        self.assertRegex(guidance, r'Do not re-run .*right away; first inspect .*call sites.*tests')
        self.assertIn('Only run the workspace test command after editing files', guidance)
        for decision in ('match', 'dispose'):
            self.assertNotIn('safe_result_chars', policy['decisions'][decision]['policy_config'])
        self.assert_thinking_off(config)

    def test_predict_normalizes_yaml_boolean_false(self):
        self.assert_thinking_off({'candidate_model': 'fixture/fake', 'candidate_thinking': False})

    def extent(self, speculation_kind, next_action, *, prediction_kind=None, model_inferences=0,
               is_error=False, phase=None):
        state = {
            'steps_completed': 1,
            'phase': phase or next_action.get('kind'),
            'semantic_facts': {'speculation_kind': speculation_kind, 'prediction_kind': prediction_kind,
                               'next_action': next_action},
            'resource_counters': {'model_inferences': model_inferences},
            'result_facts': {'last_action': {'is_error': is_error}},
        }
        return self.decisions['extent']({'process_state': state}, policy_config={}, capabilities={})

    def test_tool_execution_extent_stops_at_the_next_recognized_verification(self):
        for name, args, expected in (
            ('bash', {'command': 'mkdir -p .pytest-work'}, 'continue'),
            ('bash', {'command': 'pytest tests'}, 'stop'),
            ('write', {'path': 'result.txt', 'content': 'result'}, 'continue'),
            ('bash', {'command': 'git diff'}, 'continue'),
            ('read', {'path': 'result.txt'}, 'continue'),
        ):
            with self.subTest(name=name, args=args):
                action = {'kind': 'tool_execution', 'name': name, 'arguments': args}
                self.assertEqual(self.extent('tool_execution', action, model_inferences=1), expected)
        inference = {'kind': 'model_inference'}
        self.assertEqual(self.extent('tool_execution', inference), 'continue')
        self.assertEqual(self.extent('tool_execution', inference, model_inferences=1), 'continue')

    def test_predicted_tool_calls_extent_runs_verification_then_parks(self):
        verification = {'kind': 'tool_execution', 'name': 'bash', 'arguments': {'command': 'pytest tests'}}
        for action, phase, expected in (
            (verification, None, 'continue'),
            ({'kind': 'model_inference'}, None, 'stop'),
            ({}, 'model_inference', 'stop'),
        ):
            with self.subTest(action=action, phase=phase):
                self.assertEqual(self.extent('model_inference', action, prediction_kind='predicted_tool_calls',
                                             phase=phase), expected)
        self.assertEqual(self.extent('model_inference', {'kind': 'model_inference'},
                                     prediction_kind='branch_model'), 'continue')

    def match_verification(self, kind, output, *, is_error):
        data = {
            'evidence': {'outcome': {'is_error': is_error, 'content': [{'type': 'text', 'text': output}]}},
            'proposal': {'provenance': {'kind': kind}},
            'match_context': {'candidate': {'speculation_kind': 'tool_execution'},
                              'authority': {'status': 'failed' if is_error else 'succeeded'}},
        }
        return self.decisions['match_proposal'](data, policy_config={}, capabilities={})['verdict']

    def test_launcher_and_collection_errors_do_not_match_verification(self):
        diagnostics = (
            "python: can't open file 'runtests.py': [Errno 2] No such file or directory",
            'OSError: No such file or directory',
            'bash: pytest: command not found',
            'bash: ./runtests.py: Permission denied',
            'ERROR collecting tests/test_cookie.py',
            'pytest: error: unrecognized arguments: --fake',
            "ModuleNotFoundError: No module named 'tests.responses'",
            'SyntaxError: invalid syntax',
            'ImportError: cannot import name test_cookie',
            'Usage: runtests.py [options]',
        )
        for output in diagnostics:
            for kind, is_error in (('expected_test_failure', True), ('verification_pass', False)):
                with self.subTest(output=output, kind=kind):
                    self.assertEqual(self.match_verification(kind, output, is_error=is_error), 'incompatible')

    def test_assertion_failure_and_pass_still_match(self):
        output = 'AssertionError: cookie was not deleted\nFAILED tests/test_cookie.py\n1 failed'
        self.assertEqual(self.match_verification('expected_test_failure', output, is_error=True), 'compatible')
        self.assertEqual(self.match_verification('verification_pass', '1 passed', is_error=False), 'compatible')


if __name__ == '__main__':
    unittest.main()
