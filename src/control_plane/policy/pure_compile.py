class Unsupported(Exception):
    pass


class PureCompiler:
    def __init__(self, defer_helper_calls=False):
        self.globals = {}
        self.helpers = set()
        self.defer_helper_calls = defer_helper_calls
        self.partial = False

    def signature(self, node):
        args = node.args
        if (node.decorator_list or node.returns or node.type_comment
                or getattr(node, 'type_params', []) or args.vararg or args.kwarg
                or len(args.posonlyargs + args.args) != 1
                or {a.arg for a in args.kwonlyargs} != {'policy_config', 'capabilities'}
                or args.defaults or any(args.kw_defaults)
                or any(a.annotation for a in args.posonlyargs + args.args + args.kwonlyargs)):
            raise Unsupported()
        return (args.posonlyargs + args.args)[0].arg

    def expr(self, node, names):
        lower = lambda n: self.expr(n, names)
        if isinstance(node, ast.Constant):
            value = node.value
            if (type(value) not in (type(None), bool, str, int, float)
                    or type(value) is int and abs(value) > 2**53 - 1
                    or type(value) is float and not math.isfinite(value)):
                raise Unsupported()
            if type(value) is str:
                value.encode('utf-8')
            return ['float' if type(value) is float else 'const', value]
        if isinstance(node, ast.Name) and node.id in names:
            return ['var', node.id]
        if isinstance(node, ast.Name) and node.id in self.globals:
            return self.globals[node.id]
        if isinstance(node, (ast.List, ast.Tuple)):
            return ['tuple' if isinstance(node, ast.Tuple) else 'list', *map(lower, node.elts)]
        if isinstance(node, ast.Dict) and all(isinstance(k, ast.Constant) and type(k.value) is str for k in node.keys):
            return ['dict', *[[*lower(k)[1:], lower(v)] for k, v in zip(node.keys, node.values)]]
        if isinstance(node, ast.Subscript):
            key = node.slice
            if isinstance(key, ast.Slice) and key.step is None:
                return ['slice', lower(node.value), *[lower(x) if x else ['const', None] for x in (key.lower, key.upper)]]
            return ['index', lower(node.value), lower(key)]
        if isinstance(node, ast.IfExp):
            return ['if', lower(node.test), lower(node.body), lower(node.orelse)]
        if isinstance(node, ast.BoolOp):
            return ['and' if isinstance(node.op, ast.And) else 'or', *map(lower, node.values)]
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.Not, ast.USub, ast.UAdd)):
            return [type(node.op).__name__, lower(node.operand)]
        if isinstance(node, ast.BinOp) and isinstance(node.op, (ast.Add, ast.Sub, ast.Mult)):
            return [type(node.op).__name__, lower(node.left), lower(node.right)]
        if isinstance(node, ast.Compare):
            return ['compare', lower(node.left), *[[type(op).__name__, lower(x)] for op, x in zip(node.ops, node.comparators)]]
        if isinstance(node, ast.Call) and not node.keywords:
            fn = node.func
            if (self.defer_helper_calls and isinstance(fn, ast.Name)
                    and fn.id in self.helpers and fn.id not in names):
                # Reached only if Python would evaluate this operand. Restart
                # the entire Decision in the sandbox, including argument evaluation.
                self.partial = True
                return ['sandbox']
            if (isinstance(fn, ast.Name) and fn.id in {'len', 'bool', 'any', 'all'}
                    and fn.id not in names and fn.id not in self.globals and len(node.args) == 1):
                arg = node.args[0]
                if fn.id in {'any', 'all'} and isinstance(arg, ast.GeneratorExp):
                    return self.comprehension(arg, names, fn.id)
                return [fn.id, lower(arg)]
            if isinstance(fn, ast.Attribute) and fn.attr in {'get', 'startswith', 'endswith', 'strip', 'lower', 'split'}:
                return ['method', fn.attr, lower(fn.value), *map(lower, node.args)]
        if isinstance(node, ast.ListComp):
            return self.comprehension(node, names, 'list')
        raise Unsupported()

    def comprehension(self, node, names, mode):
        if len(node.generators) != 1:
            raise Unsupported()
        generator = node.generators[0]
        def target(node):
            if isinstance(node, ast.Name):
                return node.id
            if isinstance(node, (ast.Tuple, ast.List)) and all(isinstance(x, ast.Name) for x in node.elts):
                return [x.id for x in node.elts]
            raise Unsupported()
        if generator.is_async:
            raise Unsupported()
        variable = target(generator.target)
        inner = names | ({variable} if isinstance(variable, str) else set(variable))
        iterable = generator.iter
        if (isinstance(iterable, ast.Call) and isinstance(iterable.func, ast.Attribute)
                and iterable.func.attr == 'items' and not iterable.args and not iterable.keywords):
            compiled_iter = ['method', 'items', self.expr(iterable.func.value, names)]
        else:
            compiled_iter = self.expr(iterable, names)
        return ['comprehension', mode, variable, compiled_iter,
                self.expr(node.elt, inner), *[self.expr(test, inner) for test in generator.ifs]]

    def block(self, statements, names):
        result = []
        for node in statements:
            if isinstance(node, ast.Return):
                result.append(['return', self.expr(node.value, names) if node.value else ['const', None]])
            elif isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
                result.append(['assign', node.targets[0].id, self.expr(node.value, names)])
            elif isinstance(node, ast.If):
                result.append(['branch', self.expr(node.test, names), self.block(node.body, names), self.block(node.orelse, names)])
            else:
                raise Unsupported()
        return result

    def module(self, tree, entry, filename):
        # Package initializers execute before imports. Leave packages to Python.
        if '/' in filename:
            raise Unsupported()
        def body(node):
            statements = node.body
            return statements[1:] if ast.get_docstring(node, clean=False) is not None else statements
        seen = set()
        for node in body(tree):
            if isinstance(node, ast.FunctionDef):
                defaults = node.args.defaults + node.args.kw_defaults
                if (node.decorator_list or node.returns
                        or any(value is not None and not isinstance(value, ast.Constant) for value in defaults)
                        or getattr(node, 'type_params', [])
                        or any(x.annotation for x in ast.walk(node.args) if isinstance(x, ast.arg))):
                    raise Unsupported()
                name = node.name
                if name in {'len', 'bool', 'any', 'all'}:
                    raise Unsupported()
                self.helpers.add(name)
            elif (isinstance(node, ast.Assign) and len(node.targets) == 1
                  and isinstance(node.targets[0], ast.Name) and isinstance(node.value, ast.Constant)):
                name = node.targets[0].id
                self.globals[name] = self.expr(node.value, set())
            else:
                raise Unsupported()
            if name in seen or name.startswith('__'):
                raise Unsupported()
            seen.add(name)
        argument = self.signature(entry)
        if any(isinstance(n, ast.Name) and n.id == 'capabilities' for n in ast.walk(entry)):
            raise Unsupported()
        names = {argument, 'policy_config'} | {
            n.targets[0].id for n in ast.walk(entry) if isinstance(n, ast.Assign)
            and len(n.targets) == 1 and isinstance(n.targets[0], ast.Name)
        }
        return {'argument': argument, 'body': self.block(body(entry), names)}


def emit_pure_check(tree, entry, binding):
    result = check_pure(tree, entry, binding)
    encoded = json.dumps(result, allow_nan=False, separators=(',', ':')).encode()
    message = len(encoded).to_bytes(4, 'big') + encoded
    while message:
        message = message[os.write(3, message):]


def check_pure(tree, entry, binding):
    if MANIFEST['decision'] == 'admit.opportunity_pass' and any(
        isinstance(n, ast.Attribute) and n.attr == 'model_complete'
        for n in ast.walk(entry)
    ):
        return {'error': 'admit.opportunity_pass cannot reference capabilities.model_complete'}
    try:
        compiler = PureCompiler(defer_helper_calls=MANIFEST['decision'] == 'extent')
        program = compiler.module(tree, entry, binding['file'])
        compile(tree, '/policy/' + binding['file'], 'exec')
        json_value({'pure': program})
        if len(json.dumps(program).encode()) > 512 * 1024:
            return {'pure': None}
        return {'pure': program, 'partial': compiler.partial}
    except (Unsupported, UnicodeError, RecursionError, ValueError, TypeError, SyntaxError):
        return {'pure': None}
