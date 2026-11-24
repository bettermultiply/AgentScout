"""Match requested tool information without executing commands or inspecting the filesystem.

Shell recognition assumes standard utilities, trusted configuration and a shared
working directory. Only a small shell grammar and explicit option allowlists
contribute information. Paths are lexical, and an open-ended range represents
a request through EOF.
"""

import json
import posixpath
import re
import shlex
from dataclasses import dataclass, field
from fractions import Fraction


@dataclass(frozen=True)
class Search:
    terms: frozenset
    scopes: frozenset
    mode: tuple
    recursive: bool


@dataclass
class InfoSet:
    pure_readonly: bool = True
    file_ranges: dict = field(default_factory=dict)
    tail_ranges: dict = field(default_factory=dict)
    searches: set = field(default_factory=set)
    observations: set = field(default_factory=set)
    limited: set = field(default_factory=set)
    absolute_cd: set = field(default_factory=set)
    reason: str | None = None


def _path(path):
    for prefix in ('$PWD/', '${PWD}/'):
        if path.startswith(prefix):
            path = './' + path[len(prefix):]
            break
    if path == '~' or path.startswith('~/'):
        # Keep home-relative paths distinct from workspace-relative paths.
        return '~' + posixpath.normpath('/' + path[1:].lstrip('/'))
    if path.startswith('/'):
        path = '/' + path.lstrip('/')
    return posixpath.normpath(path)


def _call(call):
    function = call.get('function', call) if isinstance(call, dict) else None
    if not isinstance(function, dict):
        raise ValueError('invalid tool call')
    name, args = function.get('name'), function.get('arguments')
    if isinstance(args, str):
        args = json.loads(args)
    if not isinstance(name, str) or not name or not isinstance(args, dict):
        raise ValueError('invalid tool call')
    return name, args


# Recognize words without discarding the quote boundaries that distinguish
# shell syntax from literals. shlex decodes each word after the syntax check.
_TOKEN = re.compile(r'''[ \t]+|&&|[;|\n]|(?:(?:\\[^\n]|'[^']*'|"(?:\\.|[^"\\])*"|[^\s;&|<>()`'"\\])+ )''', re.X)
_UNQUOTED = re.compile(r'''\\.|'[^']*'|"(?:\\.|[^"\\])*"''')
_ENV = re.compile(r'(?:(?:PYTHONDONTWRITEBYTECODE=1|LC_ALL=[A-Za-z0-9_.@-]+)[ \t]+)+')
_LINE_TOOLS = {'cat', 'nl', 'head', 'tail', 'sed'}
_NOOP = {'echo', 'printf', 'pwd', 'true'}
_SIMPLE_GLOB = re.compile(r'[a-zA-Z0-9_./*+-]+')
_PATH_ALTERNATIVES = re.compile(r'([a-zA-Z0-9_./+-]*/)(?:\{([a-zA-Z0-9_.+-]+(?:,[a-zA-Z0-9_.+-]+)+)\})([a-zA-Z0-9_.+-]*)')


class _Glob(str):
    """An unquoted path pattern whose expansion is unknown."""


class _PathAlternatives(_Glob):
    """A finite, literal list of filenames within a fixed directory."""


def _command_prefix(command):
    prefix = _ENV.match(command)
    if prefix:
        body = command[prefix.end():]
        assignments = prefix.group().split()
        if all(value == 'PYTHONDONTWRITEBYTECODE=1' for value in assignments):
            return body, ()
        locale = tuple(value for value in assignments if value.startswith('LC_ALL='))[-1:]
        return body, locale
    return command, ()


def _shell_commands(command):
    parts, words, position = [], [], 0
    while position < len(command):
        token = _TOKEN.match(command, position)
        if not token:
            raise ValueError('unsupported shell syntax')
        position = token.end()
        raw = token.group()
        if raw in ('&&', ';', '|', '\n'):
            if not words:
                if raw == '\n' and parts:
                    continue
                raise ValueError('empty shell command')
            parts.append((words, raw))
            words = []
        elif not raw.isspace():
            # Keep an unquoted leading PWD or home reference symbolic.
            pwd = next((p for p in ('$PWD/', '${PWD}/') if raw.startswith(p)), '')
            remainder = raw[len(pwd):]
            unquoted = _UNQUOTED.sub('', remainder)
            if not raw.startswith('-') and _PATH_ALTERNATIVES.fullmatch(raw):
                words.append(_PathAlternatives(raw))
                continue
            glob = '*' in unquoted
            if glob:
                pattern = raw.removeprefix('--include=').removeprefix('--exclude=').removeprefix('--exclude-dir=')
                if (not _SIMPLE_GLOB.fullmatch(pattern) or pattern.startswith('-')
                        or pattern.startswith('*') and pattern == raw):
                    raise ValueError('unsupported shell glob')
            if (re.search(r'[$?\[\]{}]', unquoted) or unquoted.startswith('#')
                    or unquoted.startswith('~') and not raw.startswith('~/')
                    or any(re.search(r'[`$]', re.sub(r'\\.', '', span.group()[1:-1]))
                           for span in _UNQUOTED.finditer(remainder) if span.group().startswith('"'))):
                raise ValueError('unsupported shell expansion or comment')
            word, = shlex.split(raw)
            if (not pwd and word.startswith(('$PWD/', '${PWD}/'))
                    or word.startswith('~/') and not raw.startswith('~/')
                    or raw != word and re.match(r'\w+=', word)):
                raise ValueError('ambiguous quoted shell word')
            words.append(_Glob(word) if glob else word)
    if words:
        parts.append((words, None))
    elif not parts or parts[-1][1] in ('&&', '|'):
        raise ValueError('incomplete shell command')
    return parts


def _options(args, flags=(), values=(), *, numeric=False, intermixed=False):
    """Parse only listed options, including GNU short clusters and --."""
    options, operands, index = [], [], 0
    while index < len(args):
        arg = args[index]
        index += 1
        if arg == '--':
            operands.extend(args[index:])
            break
        if not arg.startswith('-') or arg == '-':
            operands.append(arg)
            if not intermixed:
                operands.extend(args[index:])
                break
            continue
        if numeric and re.fullmatch(r'-[0-9]+', arg):
            options.append(('-n', arg[1:]))
            continue
        if arg.startswith('--'):
            flag, separator, value = arg.partition('=')
            if isinstance(arg, _Glob) and flag in ('--include', '--exclude', '--exclude-dir') and separator:
                value = _Glob(value)
            pending = [(flag, value if separator else None)]
        else:
            pending = []
            for i, char in enumerate(arg[1:], 2):
                flag = '-' + char
                pending.append((flag, (arg[i:] or None) if flag in values else None))
                if flag in values:
                    break
        for flag, value in pending:
            if flag in values:
                if value is None:
                    value = args[index]
                    index += 1
            elif flag not in flags or value is not None:
                raise ValueError('unsupported option: ' + flag)
            options.append((flag, value))
    return options, operands


def _scope(path, cwd):
    if not path or path == '-' or any(ord(c) < 32 for c in path):
        raise ValueError('invalid or streaming path')
    for prefix in ('$PWD/', '${PWD}/'):
        if path.startswith(prefix):
            path = './' + path[len(prefix):]
            break
    if path.startswith(('/', '~/')) or path == '~':
        return _path(path)
    return _path(posixpath.join(cwd, path))


def _path_operands(operands):
    for operand in operands:
        if isinstance(operand, _PathAlternatives):
            directory, names, suffix = _PATH_ALTERNATIVES.fullmatch(operand).groups()
            for name in names.split(','):
                yield directory + name + suffix
        else:
            yield operand


def _line_ranges(name, args, cwd, *, stream=False):
    if name in ('head', 'tail'):
        options, paths = _options(args, values=('-n', '--lines'), numeric=name == 'head' or stream, intermixed=True)
        count = options[-1][1] if options else '10'
        if not re.fullmatch(r'[+]?[0-9]+', count) or not 0 < int(count) <= 2**63 - 1:
            raise ValueError('unsupported line count')
        number = int(count)
        if stream:
            if paths:
                raise ValueError('unsupported pipeline operands')
            return (name, count.startswith('+'), number)
        if name == 'tail' and not count.startswith('+'):
            if not paths:
                raise ValueError('missing file operands')
            # EOF-relative intervals cannot be compared to absolute line numbers.
            return [(_scope(path, cwd), -number, -1) for path in paths]
        if name == 'head' and count.startswith('+'):
            raise ValueError('unsupported relative line count')
        ranges = [(1, number)] if name == 'head' else [(number, None)]
    elif name == 'sed':
        options, paths = _options(args, ('-n', '-s', '--quiet', '--silent', '--separate'),
                                  ('-e', '--expression'), intermixed=True)
        if all(flag not in ('-n', '--quiet', '--silent') for flag, _ in options):
            raise ValueError('unsupported sed mode')
        scripts = [value for flag, value in options if flag in ('-e', '--expression')]
        if not scripts and paths:
            scripts, paths = paths[:1], paths[1:]
        ranges = []
        for script in scripts:
            for part in script.split(';'):
                match = re.fullmatch(r'\s*([1-9][0-9]*)(?:\s*,\s*([1-9][0-9]*|\$))?\s*p\s*', part)
                if not match:
                    raise ValueError('unsupported sed script')
                start, end = match.groups()
                ranges.append((int(start), None if end == '$' else max(int(start), int(end or start))))
        separate = any(flag in ('-s', '--separate') for flag, _ in options)
        if not ranges or len(paths) > 1 and not separate:
            raise ValueError('sed requires per-file line numbering')
    else:
        if name == 'nl' and args[:1] == ['-ba']:
            args = args[1:]
        _, paths = _options(args, ('-n', '-b') if name == 'cat' else (), intermixed=True)
        ranges = [(1, None)]
    if not paths:
        raise ValueError('missing file operands')
    return [(_scope(path, cwd), start, end) for path in paths for start, end in ranges]


def _terms(pattern, dialect):
    if dialect not in ('extended', 'rg'):
        return {pattern}
    start = depth = 0
    bracket = escaped = False
    terms = set()
    for index, char in enumerate(pattern):
        if escaped:
            escaped = False
        elif char == '\\':
            escaped = True
        elif bracket:
            if char == ']':
                bracket = False
        elif char == '[':
            # POSIX character classes and nested classes are kept opaque.
            if '[' in pattern[index + 1:pattern.find(']', index + 1)]:
                return {pattern}
            bracket = True
        elif char == '(':
            depth += 1
        elif char == ')':
            depth -= 1
        elif char == '|' and depth == 0:
            terms.add(pattern[start:index])
            start = index + 1
    return terms | {pattern[start:]} if not (bracket or escaped or depth) else {pattern}


def _search(name, args, cwd, locale):
    flags = ('-n', '-i', '-E', '-F', '-G', '-H', '-h', '--line-number', '--ignore-case',
             '--extended-regexp', '--fixed-strings', '--basic-regexp', '--with-filename', '--no-filename',
             '-l', '--files-with-matches', '-w', '--word-regexp')
    numeric_options = ('-A', '--after-context', '-B', '--before-context', '-C', '--context', '-m', '--max-count')
    values = ('-e', '--regexp') + numeric_options
    if name == 'rg':
        flags += ('--no-heading', '-S', '--smart-case')
        values += ('-g', '--glob', '-t', '--type', '--color')
    elif name != 'git grep':
        flags += ('-r', '-R', '--recursive', '--dereference-recursive')
        values += ('--include', '--exclude', '--exclude-dir', '--color')
    aliases = {'--glob': '-g', '--type': '-t', '--max-count': '-m', '--word-regexp': '-w',
               '--files-with-matches': '-l', '--smart-case': '-S'}
    git_paths = []
    if name == 'git grep':
        args, _, git_paths = _partition(args, '--')
        if any(path.startswith(':') or any(c in path for c in '*?[') for path in git_paths):
            raise ValueError('unsupported git pathspec')
    options, operands = _options(args, flags, values, intermixed=name != 'git grep')
    if name == 'rg' and any(flag in ('-h', '--basic-regexp', '--extended-regexp') for flag, _ in options):
        raise ValueError('unsupported rg option')
    if name == 'git grep' and any(flag in ('--basic-regexp',) for flag, _ in options):
        raise ValueError('unsupported git grep option')
    for flag, value in options:
        if (flag in numeric_options and not re.fullmatch(r'[0-9]+', value)
                or flag == '--color' and value != 'never'
                or isinstance(value, _PathAlternatives)
                or isinstance(value, _Glob) and flag not in ('--include', '--exclude', '--exclude-dir')):
            raise ValueError('unsupported search option value')
    patterns = [value for flag, value in options if flag in ('-e', '--regexp')]
    if not patterns and operands:
        patterns, operands = operands[:1], operands[1:]
    if name == 'git grep':
        if operands:
            raise ValueError('git grep paths must follow --')
        operands = git_paths
    if not patterns or any(isinstance(p, _Glob) or '\n' in p for p in patterns):
        raise ValueError('missing or unsupported search pattern')
    dialect = {'rg': 'rg', 'egrep': 'extended', 'fgrep': 'literal'}.get(name, 'basic')
    modes = {'-E': 'extended', '-F': 'literal', '-G': 'basic', '--extended-regexp': 'extended',
             '--fixed-strings': 'literal', '--basic-regexp': 'basic'}
    if name == 'rg' and any(flag in modes and modes[flag] != 'literal' for flag, _ in options):
        raise ValueError('unsupported rg regex mode')
    for flag, _ in options:
        dialect = modes.get(flag, dialect)
    terms = frozenset(term for pattern in patterns for term in _terms(pattern, dialect))
    scopes = frozenset(_scope(path, cwd) for path in _path_operands(operands or ['.']))
    dereference = any(flag in ('-R', '--dereference-recursive') for flag, _ in options)
    recursive = name in ('rg', 'git grep') or dereference or any(flag in ('-r', '--recursive') for flag, _ in options)
    filters = tuple((aliases.get(flag, flag), value) for flag, value in options
                    if aliases.get(flag, flag) in ('-g', '--include', '--exclude', '--exclude-dir', '-t', '-m'))
    corpus = name if name in ('rg', 'git grep') else 'grep'
    case_flags = {'-i': '-i', '--ignore-case': '-i', '-S': '-S', '--smart-case': '-S'}
    case = tuple(case_flags[flag] for flag, _ in options if flag in case_flags)[-1:]
    word = any(aliases.get(flag, flag) == '-w' for flag, _ in options)
    files_only = any(aliases.get(flag, flag) == '-l' for flag, _ in options)
    return Search(terms, scopes, (dialect, case, filters, corpus, dereference, locale, word, files_only), recursive)


_GIT_FLAGS = {
    'status': ('-s', '--short', '--porcelain', '--porcelain=v1', '--porcelain=v2', '-b', '--branch',
               '--untracked-files=no', '--untracked-files=normal', '--untracked-files=all'),
    'diff': ('--stat', '--name-only', '--name-status', '--check', '--cached', '--staged',
             '--no-ext-diff', '--no-textconv', '--no-color', '--quiet', '--exit-code', '--numstat'),
    'log': ('--oneline', '--stat', '--no-color', '--all', '--decorate'),
}


def _observation(name, args, cwd, locale):
    if name in ('type', 'which', 'command'):
        operands = args[1:] if name == 'command' and args[:1] == ['-v'] else args if name != 'command' else []
        if len(operands) == 1 and re.fullmatch(r'[A-Za-z0-9_./][A-Za-z0-9_./+-]*', operands[0]):
            return [('command-lookup', cwd, name, operands[0])]
        raise ValueError('unsupported command lookup')
    if re.fullmatch(r'python(?:[23](?:\.\d+)?)?', name) and args == ['--version']:
        return [('python_version', cwd, locale, name)]
    if name == 'git':
        subcommand, *args = args
        if subcommand == 'remote' and args == ['-v']:
            return [('git_remote', cwd, locale, ('-v',))]
        if subcommand in ('branch', 'tag'):
            flags = ('-a', '--all', '--list', '--show-current', '--no-color') if subcommand == 'branch' else ('-l', '--list')
            options, operands = _options(args, flags, ('--contains',) if subcommand == 'tag' else (), intermixed=True)
            if operands or any(value is not None and value.startswith('-') for _, value in options):
                raise ValueError('unsupported git listing arguments')
            aliases = {'-a': '--all', '-l': '--list'}
            return [('git_' + subcommand, cwd, locale, tuple((aliases.get(flag, flag), value) for flag, value in options))]
        if subcommand not in _GIT_FLAGS:
            raise ValueError('unsupported git command')
        before, separator, after = _partition(args, '--')
        split = len(before)
        for index, arg in enumerate(before):
            if (arg in _GIT_FLAGS[subcommand] or subcommand == 'log' and re.fullmatch(r'-[0-9]+', arg)):
                continue
            if arg.startswith('-'):
                raise ValueError('unsupported git option: ' + arg)
            split = index
            break
        if any(arg.startswith('-') for arg in before[split:]):
            raise ValueError('unsupported git option order')
        flags = tuple({'-s': '--short', '--staged': '--cached'}.get(arg, arg) for arg in before[:split])
        if any(p.startswith(':') or any(c in p for c in '*?[') for p in after):
            raise ValueError('unsupported git pathspec')
        if not separator and subcommand == 'diff' and before[split:]:
            # Without -- Git can interpret operands as either revisions or paths.
            return [('git_diff', cwd, locale, flags, tuple(before[split:]), ())]
        return [('git_' + subcommand, cwd, locale, flags, tuple(before[split:]),
                 tuple(_scope(path, cwd) for path in after))]
    if name == 'ls':
        options, paths = _options(args, ('-a', '-A', '-l', '-h', '-1', '-d', '-F', '-t', '-r', '-R',
                                          '--all', '--almost-all', '--recursive'), intermixed=True)
        aliases = {'--all': '-a', '--almost-all': '-A', '--recursive': '-R'}
        flags = tuple(sorted({aliases.get(flag, flag) for flag, _ in options}))
        return [('ls', _scope(path, cwd), locale, flags) for path in _path_operands(paths or ['.'])]
    if name == 'rg' and args[:1] == ['--files']:
        options, paths = _options(args[1:], ('--hidden',), ('-g', '--glob'), intermixed=True)
        if any(isinstance(value, _Glob) for _, value in options):
            raise ValueError('unsupported shell expansion in rg filter')
        flags = tuple(('-g' if flag == '--glob' else flag, value) for flag, value in options)
        return [('rg_files', _scope(path, cwd), locale, flags) for path in _path_operands(paths or ['.'])]
    if name == 'find':
        index = next((i for i, value in enumerate(args) if value.startswith('-') or value in ('!', '(', ')')), len(args))
        paths, predicates = args[:index], args[index:]
        normalized = []
        while predicates:
            flag, *predicates = predicates
            if flag in ('-name', '-iname', '-path', '-ipath', '-type', '-maxdepth', '-mindepth'):
                value, *predicates = predicates
                normalized.extend((flag, value))
            elif flag == '-print' and not predicates:
                break
            else:
                raise ValueError('unsupported find predicate: ' + flag)
        return [('find', _scope(path, cwd), locale, tuple(normalized)) for path in paths or ['.']]
    raise ValueError('unsupported command: ' + name)


def _partition(args, delimiter):
    if delimiter not in args:
        return args, False, []
    index = args.index(delimiter)
    return args[:index], True, args[index + 1:]


def _shell_info(command):
    cwd = '.'
    facts, pipeline = [], None
    for words, separator in _shell_commands(command):
        name, *args = words
        assignments = []
        while re.fullmatch(r'PYTHONDONTWRITEBYTECODE=1|LC_ALL=[A-Za-z0-9_.@-]+', name) and args:
            assignments.append(name)
            name, *args = args
        locale = tuple(word for word in assignments if word.startswith('LC_ALL='))[-1:]
        if pipeline is not None:
            if name not in ('head', 'tail'):
                raise ValueError('unsupported pipeline stage: ' + name)
            raw_pipeline, upstream, limits = pipeline
            limits += (_line_ranges(name, args, cwd, stream=True),)
            if separator != '|':
                if upstream:
                    facts.extend(('limited', (kind, item, limits)) for kind, item in upstream)
                else:
                    facts.append(('observation', ('pipeline', raw_pipeline + limits)))
                pipeline = None
            else:
                pipeline = raw_pipeline, upstream, limits
            continue
        if any(isinstance(word, _Glob) for word in words) and (name not in ('rg', 'grep', 'fgrep', 'egrep', 'ls')
                                                               or isinstance(name, _Glob)):
            raise ValueError('unsupported glob command')
        if name == 'cd':
            if len(args) != 1 or args[0].startswith('-') or separator != '&&':
                raise ValueError('unsupported directory change')
            cwd = _scope(args[0], cwd)
            if cwd.startswith('/'):
                facts.append(('absolute_cd', cwd))
            continue
        if name in _NOOP and separator != '|':
            if name == 'printf' and args and args[0].startswith('-v'):
                raise ValueError('unsupported variable assignment')
            continue
        if name in _LINE_TOOLS:
            current = [('tail' if item[1] < 0 else 'file', item) for item in _line_ranges(name, args, cwd)]
        elif (name == 'rg' and args[:1] != ['--files'] or name in ('grep', 'egrep', 'fgrep')
              or name == 'git' and args[:1] == ['grep']):
            current = [('search', _search('git grep' if name == 'git' else name,
                                          args[1:] if name == 'git' else args, cwd, locale))]
        else:
            current = [('observation', item) for item in _observation(name, args, cwd, locale)]
        if separator == '|':
            supported = (name in ('ls', 'rg', 'grep', 'egrep', 'fgrep')
                         or name == 'git' and args[:1] in (['log'], ['tag'], ['branch']))
            pipeline = (((cwd, tuple(words)),), current if supported else (), ())
        else:
            facts.extend(current)
    return facts


def _merge(ranges):
    merged = []
    for start, end in sorted(ranges, key=lambda interval: interval[0]):
        if merged and merged[-1][1] is None:
            break
        if merged and start <= merged[-1][1] + 1:
            first, last = merged[-1]
            merged[-1] = (first, None if end is None else max(end, last))
        else:
            merged.append((start, end))
    return tuple(merged)


def extract_info_set(calls):
    """Return merged file ranges, search intents and observations for a batch.

    A rejected call invalidates the entire information set. Empty batches have
    no information and cannot match.
    """
    info = InfoSet()
    try:
        if not isinstance(calls, (list, tuple)):
            raise ValueError('invalid or missing tool batch')
        for call in calls:
            name, args = _call(call)
            if name == 'read' and args.keys() <= {'path', 'offset', 'limit'}:
                path = args.get('path')
                offset, limit = args.get('offset'), args.get('limit')
                if not isinstance(path, str) or any(value is not None and (type(value) is not int or value < 0)
                                                    for value in (offset, limit)):
                    raise ValueError('invalid read arguments')
                start = offset or 1
                facts = [('file', (_scope(path, '.'), start, start + limit - 1 if limit else None))]
            elif name == 'bash' and args.keys() <= {'command', 'timeout'} and isinstance(args.get('command'), str):
                facts = _shell_info(args['command'].strip())
            else:
                raise ValueError('unsupported tool or arguments: ' + name)
            for kind, item in facts:
                if kind in ('file', 'tail'):
                    path, start, end = item
                    ranges = info.file_ranges if kind == 'file' else info.tail_ranges
                    ranges.setdefault(path, []).append((start, end))
                elif kind == 'search':
                    info.searches.add(item)
                elif kind == 'limited':
                    info.limited.add(item)
                elif kind == 'absolute_cd':
                    info.absolute_cd.add(item)
                else:
                    info.observations.add(item)
    except (ValueError, IndexError) as error:
        return InfoSet(pure_readonly=False, reason=str(error))
    info.file_ranges = {path: _merge(ranges) for path, ranges in info.file_ranges.items()}
    info.tail_ranges = {path: _merge(ranges) for path, ranges in info.tail_ranges.items()}
    return info


def _has_facts(info):
    return bool(info.file_ranges or info.tail_ranges or info.searches or info.observations or info.limited)


def _uncovered(ranges, start, end):
    for first, last in ranges:
        if end is not None and first > end:
            break
        if last is not None and last < start:
            continue
        if first > start:
            yield start, first - 1
        if last is None:
            return
        start = last + 1
    if end is None or start <= end:
        yield start, end


def _entry_output(entry):
    content = (entry.get('result') or {}).get('content')
    if isinstance(content, str):
        return content
    return '\n'.join(part.get('text', '') for part in (content or []) if part.get('type') == 'text')


def _readonly_subset(calls):
    return extract_info_set([call for call in (calls or []) if extract_info_set([call]).pure_readonly])


def _search_hit(line):
    match = re.fullmatch(r'(.+?):([1-9]\d*):.*', line)
    return (_path(match[1]), int(match[2])) if match else None


def filter_novel_readonly(candidate_entries, authority_calls, *, authority_entries=()):
    """Subtract requested reads and known search hits from completed output."""
    authority_info = _readonly_subset(authority_calls)
    authority_hits = {hit for entry in authority_entries for line in _entry_output(entry).splitlines()
                      if (hit := _search_hit(line))}
    novel = []
    for entry in candidate_entries:
        name, args = entry.get('tool_name'), entry.get('arguments') or {}
        info, authority = _aligned_info(extract_info_set([{'name': name, 'arguments': args}]), authority_info)
        searches = info.searches | {item for kind, item, _ in info.limited if kind == 'search'}
        new_search = any(not _search_covered(authority, search) for search in searches)
        output = _entry_output(entry)
        lines = output.splitlines()
        if new_search and not info.file_ranges and lines and all(map(_search_hit, lines)):
            kept = []
            for line in lines:
                path, number = _search_hit(line)
                if (path, number) not in authority_hits and not any(
                        start <= number and (end is None or number <= end)
                        for start, end in authority.file_ranges.get(path, ())):
                    kept.append(line)
            if kept:
                novel.append(dict(entry, cropped_output='\n'.join(kept)))
            continue
        slices = [(path, start, end) for path, ranges in info.file_ranges.items() for start, end in ranges]
        missing = [(path, first, last) for path, start, end in slices
                   for first, last in _uncovered(authority.file_ranges.get(path, ()), start, end)]
        novel_tail = any((1, None) not in authority.file_ranges.get(path, ())
                         and any(_uncovered(authority.tail_ranges.get(path, ()), start, end))
                         for path, ranges in info.tail_ranges.items() for start, end in ranges)
        if not (missing or new_search or novel_tail):
            continue
        if missing != slices and len(slices) == 1 and not (searches or info.observations or info.tail_ranges):
            path, start, end = slices[0]
            # Only crop a single direct slice with a verifiable output-to-line mapping.
            shell = _shell_commands(args['command'].strip()) if name == 'bash' else []
            direct = name == 'read'
            if len(shell) == 1:
                (utility, *operands), _ = shell[0]
                direct = utility in ('sed', 'head') and len(_line_ranges(utility, operands, '.')) == 1
            if direct and end is not None and len(lines) == end - start + 1:
                chunks = [(first, last, '\n'.join(lines[first - start:last - start + 1]))
                          for _, first, last in missing]
                chunks = [chunk for chunk in chunks if chunk[2].strip()]
                if not chunks:
                    continue
                bounds = ', '.join(f'{first}..{last}' for first, last, _ in chunks)
                entry = dict(entry, cropped_output='\n'.join(text for _, _, text in chunks),
                             cropped_label=f'{path} (lines {bounds})',
                             cropped_ranges=[(first, last) for first, last, _ in chunks])
        novel.append(entry)
    return novel


def is_readonly_action(tool_name, arguments):
    """Whether a single action is recognized as read-only and yields information."""
    info = extract_info_set([{'name': tool_name, 'arguments': arguments}])
    return info.pure_readonly and _has_facts(info)


def _cumulative_sets(candidate_actions, authority_calls):
    readonly = []
    for action in candidate_actions:
        try:
            name, arguments = _call(action)
        except ValueError:
            continue
        if is_readonly_action(name, arguments):
            readonly.append(action)
    return extract_info_set(readonly), extract_info_set(authority_calls)


def match_cumulative_info(candidate_actions, authority_calls):
    """Compare accumulated read-only calls, excluding mutations and unknown actions."""
    return _match_info(*_cumulative_sets(candidate_actions, authority_calls))


def directional_overlap(candidate_actions, authority_calls):
    """Score alignment for Salvage, not information equivalence for Promote."""
    candidate, authority = _cumulative_sets(candidate_actions, authority_calls)
    if not candidate.pure_readonly or not authority.pure_readonly:
        return 0.0
    strict = _coverage(*_aligned_info(candidate, authority))
    aligned = _coverage(*_aligned_info(candidate, authority, include_upstream=True), directional=True)
    return float(max(_coverage_ratio(strict), _coverage_ratio(aligned)))


def _normalized_calls(calls, info):
    try:
        if not isinstance(calls, (list, tuple)):
            return []
        normalized = []
        for call in calls:
            name, raw_args = _call(call)
            args = raw_args.copy()
            if name == 'bash':
                args.pop('timeout', None)
                if isinstance(args.get('command'), str):
                    command = args['command'].strip()
                    body, locale = _command_prefix(command)
                    if info.pure_readonly and (not locale or all(
                            words[0] in _LINE_TOOLS for words, _ in _shell_commands(body))):
                        command = body
                    args['command'] = command
            elif name in ('read', 'edit', 'write') and isinstance(args.get('path'), str) and args['path']:
                args['path'] = _path(args['path'])
            normalized.append((name, json.dumps(args, sort_keys=True, allow_nan=False)))
        return sorted(normalized) if info.pure_readonly else normalized
    except (TypeError, ValueError):
        return []


def _under(path, parent):
    if parent == '.':
        return not path.startswith(('/', '~/', '../')) and path not in ('..', '~')
    return path == parent or path.startswith(parent.rstrip('/') + '/')


def _range_coverage(ranges, start, end):
    if end is None:
        return Fraction(any(a <= start and b is None for a, b in ranges))
    covered = sum(max(0, min(b if b is not None else end, end) - max(a, start) + 1) for a, b in ranges)
    return Fraction(covered, end - start + 1)


def _search_covered(info, required):
    for scope in required.scopes:
        if (1, None) in info.file_ranges.get(scope, ()):
            continue
        if not any(search.mode == required.mode and search.terms >= required.terms
                   and (scope in search.scopes and (search.recursive or not required.recursive)
                        or search.recursive and any(_under(scope, parent) for parent in search.scopes))
                   for search in info.searches):
            return False
    return True


def _search_tokens(search):
    return {token for term in search.terms for token in re.findall(r'[a-z0-9_]{4,}', term.lower())}


def _search_aligned(info, required):
    tokens = _search_tokens(required)
    return any(tokens & _search_tokens(search) and any(
        _under(left, right) or _under(right, left)
        for left in search.scopes for right in required.scopes
    ) for search in info.searches)


def _observation_covered(info, required):
    if required in info.observations:
        return True
    if required[0] == 'git_status':
        kind, cwd, locale, flags, operands, paths = required
        if '--short' in flags and not any(flag.startswith('--porcelain') for flag in flags):
            full = tuple(flag for flag in flags if flag != '--short')
            return (kind, cwd, locale, full, operands, paths) in info.observations
    if required[0] == 'ls':
        _, path, locale, flags = required
        return '-d' not in flags and any(
            item[0] == 'ls' and item[2:] == (locale, tuple(sorted(set(flags) | {'-R'})))
            and _under(path, item[1]) for item in info.observations)
    return False


def match_tool_calls(candidate_calls, authority_calls):
    """Compare a candidate batch against the authority's required information."""
    candidate, authority = map(extract_info_set, (candidate_calls, authority_calls))
    exact = _normalized_calls(candidate_calls, candidate)
    if exact and exact == _normalized_calls(authority_calls, authority):
        return dict(matched=True, kind='exact', score=1.0, coverage=1.0, details={})
    return _match_info(candidate, authority)


def _map_observation(item, path):
    kind, scope, *rest = item
    if kind == 'pipeline':
        return item
    if kind in ('git_status', 'git_diff', 'git_log'):
        locale, flags, revisions, paths = rest
        rest = [locale, flags, revisions, tuple(map(path, paths))]
    return (kind, path(scope), *rest)


def _map_info(info, path, *, include_upstream=False):
    mapped = InfoSet()
    for source, target in ((info.file_ranges, mapped.file_ranges), (info.tail_ranges, mapped.tail_ranges)):
        for filename, ranges in source.items():
            target.setdefault(path(filename), []).extend(ranges)
        target.update({filename: _merge(ranges) for filename, ranges in target.items()})

    def map_search(search):
        return Search(search.terms, frozenset(map(path, search.scopes)), search.mode, search.recursive)

    mapped.searches = {map_search(item) for item in info.searches}
    mapped.observations = {_map_observation(item, path) for item in info.observations}
    for kind, item, limits in info.limited:
        if kind == 'search':
            item = map_search(item)
            target = mapped.searches
        else:
            item = _map_observation(item, path)
            target = mapped.observations
        if include_upstream:
            target.add(item)
        else:
            mapped.limited.add((kind, item, limits))
    return mapped


def _workspace_root(candidate, authority):
    def paths(info):
        found = set()
        def collect(path):
            if '*' not in path:
                found.add(path)
            return path
        _map_info(info, collect)
        return found

    roots, confirmed = set(), set()
    candidate_paths, authority_paths = map(paths, (candidate, authority))
    for absolute_paths, relative_paths, directories in (
            (candidate_paths, authority_paths, candidate.absolute_cd),
            (authority_paths, candidate_paths, authority.absolute_cd)):
        for absolute in absolute_paths:
            if not absolute.startswith('/'):
                continue
            for relative in relative_paths:
                if relative == '.' or not _under(relative, '.') or not absolute.endswith('/' + relative):
                    continue
                prefix = absolute[:-len(relative) - 1]
                if prefix in directories:
                    confirmed.add(prefix)
                if prefix and '/' in relative:
                    roots.add(prefix)
    # An explicit directory change helps identify the right suffix when
    # there are multiple possible roots. Never infer from a basename alone.
    roots = confirmed or roots
    return next(iter(roots)) if len(roots) == 1 else None


def _aligned_info(candidate, authority, *, include_upstream=False):
    root = _workspace_root(candidate, authority)

    def align(path):
        if root and _under(path, root):
            return path[len(root):].lstrip('/') or '.'
        return path

    return tuple(_map_info(info, align, include_upstream=include_upstream) for info in (candidate, authority))


def _coverage(candidate, authority, *, directional=False):
    def file_coverage(path, start, end):
        ranges = candidate.file_ranges.get(path, ())
        if directional and end is None:
            # Without an end line, only presence of an intersection is knowable.
            return Fraction(any(last is None or last >= start for _, last in ranges))
        return _range_coverage(ranges, start, end)

    return [
        *(file_coverage(path, start, end)
          for path, ranges in authority.file_ranges.items() for start, end in ranges),
        *(Fraction(1) if (1, None) in candidate.file_ranges.get(path, ()) else
          _range_coverage(candidate.tail_ranges.get(path, ()), start, end)
          for path, ranges in authority.tail_ranges.items() for start, end in ranges),
        *(Fraction(_search_covered(candidate, item) or directional and _search_aligned(candidate, item))
          for item in authority.searches),
        *(Fraction(_observation_covered(candidate, item)) for item in authority.observations),
        *(Fraction(item in candidate.limited) for item in authority.limited),
    ]


def _coverage_ratio(coverage):
    return sum(coverage) / len(coverage) if coverage else 0


def _match_info(candidate, authority):
    details = {}
    for role, info in (('candidate', candidate), ('authority', authority)):
        if not info.pure_readonly:
            details[role + '_reason'] = info.reason
    if details:
        return dict(matched=False, kind='mismatch', score=0.0, coverage=0.0, details=details)
    coverage = _coverage(*_aligned_info(candidate, authority))
    details = {'required_items': len(coverage), 'covered_items': coverage.count(1)}
    ratio = _coverage_ratio(coverage)
    if ratio == 1:
        return dict(matched=True, kind='info_equivalent', score=0.95, coverage=1.0, details=details)
    if ratio:
        return dict(matched=False, kind='partial_info', score=round(float(ratio), 3), coverage=float(ratio), details=details)
    return dict(matched=False, kind='mismatch', score=0.0, coverage=0.0, details=details)


def _shell_tokens(command):
    import re

    # Keep quoted or escaped separators inside their argument.
    return re.compile(r'''\\.|'[^']*'|"(?:\\.|[^"\\])*"|(?<!\S)#.*|(?P<operator>(?<!\S)2>&1|[;&|<>()\n`$])''').finditer(command)


def _verification_words(command, *, allow_setup=True, directories=None):
    if not isinstance(command, str):
        return []
    command = command.strip()
    if '`' in command:
        return []
    parts, start, redirect = [], 0, None
    for match in _shell_tokens(command):
        operator = match['operator']
        if not operator or operator == '$' or match.start() < start:
            continue
        if operator == '2>&1':
            suffix = command[match.end():].strip()
            if suffix and not suffix.startswith('|'):
                return []
            redirect = match.start()
            continue
        if operator == '|':
            try:
                [(limiter, separator)] = _shell_commands(command[match.end():].strip())
                if separator or limiter[0] not in ('head', 'tail'):
                    return []
                _line_ranges(limiter[0], limiter[1:], '.', stream=True)
            except (ValueError, IndexError):
                return []
            redirect = match.start() if redirect is None else redirect
            break
        if operator == '&' and command.startswith('&&', match.start()) and match.start() >= start:
            end = match.end() + 1
        elif operator == '\n':
            if not command[start:match.start()].strip():
                continue
            end = match.end()
        elif match.start() < start:
            continue
        else:
            return []
        parts.append(command[start:match.start()])
        start = end
    parts.append(command[start:redirect])
    unchecked = re.sub(r'\$\{PWD\}|\$PWD(?!\w)', lambda match: ' ' * len(match[0]), command)
    if '$' in unchecked:
        # Allow a simple TMPDIR reference only in a leading temp-directory assignment.
        position = start
        while token := _TOKEN.match(command, position):
            raw = token.group()
            if re.fullmatch(r'(?:TMPDIR|SPHINX_TEST_TEMPDIR)="\$(?:\{TMPDIR\}|TMPDIR)(?:/[\w./-]+)?"', raw):
                unchecked = unchecked[:position] + ' ' * len(raw) + unchecked[token.end():]
            elif not (raw.isspace() or raw in ('env', 'command', '--') or re.match(r'[A-Za-z_]\w*=', raw)):
                break
            position = token.end()
        if '$' in unchecked:
            return []
    try:
        *preambles, words = [shlex.split(part, comments=True) for part in parts]
    except ValueError:
        return []
    for preamble in preambles:
        if not (len(preamble) == 2 and preamble[0] == 'cd' and not preamble[1].startswith('-')
                or allow_setup and len(preamble) == 3 and preamble[:2] == ['mkdir', '-p'] and not preamble[2].startswith('-')):
            return []
        if not preamble[-1]:
            return []
        if directories is not None and preamble[0] == 'cd':
            directories.append(preamble[1])
    while words:
        if words[0] in ('env', 'command', '--'):
            words.pop(0)
            continue
        if not re.match(r'^[A-Za-z_][A-Za-z0-9_]*=', words[0]):
            break
        words.pop(0)
    return words


def is_verification_command(command):
    import re

    words = _verification_words(command)
    if not words or {'--help', '-h', '--version', '--collect-only', '--fix', '--write', '-U',
                     '--update-snapshots', '--updateSnapshot', '--snapshot-update'}.intersection(words):
        return False
    runners = {'pytest', 'py.test', 'runtests', 'runtests.py', 'unittest', 'tox',
                'vitest', 'jest', 'mocha', 'ctest', 'rspec', 'mypy', 'pyright', 'eslint', 'tsc'}
    targets = {
        'cargo': {'test', 'check', 'clippy', 'build'}, 'go': {'test', 'build', 'vet'},
        'npm': {'test', 'build', 'check', 'lint'}, 'pnpm': {'test', 'build', 'check', 'lint'},
        'yarn': {'test', 'build', 'check', 'lint'}, 'make': {'test', 'check', 'build'},
        'mvn': {'test', 'verify'}, 'gradle': {'test', 'check', 'build'},
        'gradlew': {'test', 'check', 'build'}, 'node': {'--test'},
    }
    name, *args = words
    name = name.rsplit('/', 1)[-1]
    if re.fullmatch(r'python(?:[23](?:\.\d+)?)?', name):
        if args[:1] == ['-m']:
            args = args[1:]
        return bool(args and args[0].rsplit('/', 1)[-1] in runners)
    if name in ('npm', 'pnpm', 'yarn') and args[:1] == ['run']:
        args = args[1:]
    return name in runners or bool(args and args[0] in targets.get(name, ()))




def normalize_verification_command(command, *, directories=None):
    """Return a conservative runner and argument signature, or None."""
    words = _verification_words(command, allow_setup=False, directories=directories)
    if not words or not is_verification_command(command):
        return None
    name, *args = words
    if name in ('python', 'python3') and args[:2] == ['-m', 'pytest']:
        name, args = 'pytest', args[2:]
    elif name == 'python3':
        name = 'python'
    if name != 'pytest':
        return name, tuple(args), None
    targets, options = [], []
    iterator = iter(args)
    for arg in iterator:
        if arg == '--basetemp':
            if next(iterator, None) is None:
                return None
        elif arg.startswith('--basetemp=') or arg in ('-q', '-v', '--quiet', '--verbose'):
            continue
        elif arg in ('-k', '-m'):
            value = next(iterator, None)
            if value is None:
                return None
            options.extend((arg, value))
        elif arg.startswith('-'):
            # Unknown options may take a value, so they only match exact argv.
            return name, tuple(args), None
        else:
            targets.append(_path(arg))
    return name, tuple(options), frozenset(targets)


def match_verification_calls(candidate_calls, authority_calls):
    """Require coverage of every Authority call and at least one verification."""
    def signature(call):
        try:
            name, args = _call(call)
        except ValueError:
            return None
        cwd = []
        sig = normalize_verification_command(args.get('command'), directories=cwd) if name == 'bash' else None
        if sig is None:
            return None
        directory = _path(posixpath.join(*cwd)) if cwd else None
        name, args, targets = sig
        if name == 'python' and args and args[0].endswith('.py'):
            return (name, args[1:], targets), directory, _scope(args[0], directory or '.')
        return sig, directory, None

    def covers(candidate, required):
        left, right = signature(candidate), signature(required)
        if right is None:
            return left is None and is_readonly_call(candidate) and match_tool_calls([candidate], [required])['matched']
        if left is None or left[0][:2] != right[0][:2]:
            return False
        (left, candidate_dir, candidate_runner), (right, authority_dir, authority_runner) = left, right
        if candidate_runner != authority_runner:
            if not candidate_runner or not authority_runner or candidate_dir and authority_dir:
                return False
            absolute, relative = sorted((candidate_runner, authority_runner), key=lambda path: not path.startswith('/'))
            if not (absolute.startswith('/') and not relative.startswith('/') and '/' in relative
                    and absolute.endswith('/' + relative)):
                return False
        elif candidate_runner is None and candidate_dir and authority_dir and candidate_dir != authority_dir:
            return False
        a, b = left[2], right[2]
        if a is None or b is None:
            return a == b
        return not a or bool(b) and a >= b

    def is_readonly_call(call):
        info = extract_info_set([call])
        return info.pure_readonly and _has_facts(info)

    candidate_calls, authority_calls = candidate_calls or [], authority_calls or []
    return {'matched': (any(map(signature, authority_calls))
                        and all(signature(call) or is_readonly_call(call) for call in candidate_calls)
                        and all(any(covers(candidate, required) for candidate in candidate_calls)
                                for required in authority_calls))}
