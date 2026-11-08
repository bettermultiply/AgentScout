# Embedded in each compiled Python script. Only trusted bootstrap runs before
# seccomp is loaded; user imports and module-level code run after restrictions.
import ast
import ctypes
import importlib
import importlib.abc
import importlib.util
import inspect
import math
import os
import resource
import sys


def restrict():
    resource.setrlimit(resource.RLIMIT_AS, (256 * 1024 * 1024,) * 2)
    resource.setrlimit(resource.RLIMIT_CPU, (3, 3))
    resource.setrlimit(resource.RLIMIT_FSIZE, (1024 * 1024,) * 2)
    resource.setrlimit(resource.RLIMIT_NOFILE, (32, 32))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    library = ctypes.CDLL("libseccomp.so.2", use_errno=True)
    library.seccomp_init.argtypes = [ctypes.c_uint32]
    library.seccomp_init.restype = ctypes.c_void_p
    library.seccomp_syscall_resolve_name.argtypes = [ctypes.c_char_p]
    library.seccomp_rule_add.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int, ctypes.c_uint]
    library.seccomp_load.argtypes = [ctypes.c_void_p]
    library.seccomp_release.argtypes = [ctypes.c_void_p]
    context = library.seccomp_init(0x7FFF0000)  # SCMP_ACT_ALLOW
    if not context:
        raise RuntimeError("cannot create seccomp filter")
    try:
        for name in (
            "clone", "clone3", "fork", "vfork", "execve", "execveat",
            "socket", "socketpair", "connect", "bind", "listen", "accept", "accept4",
            "ptrace", "process_vm_readv", "process_vm_writev", "mount", "umount2",
            "unshare", "setns", "bpf", "perf_event_open", "keyctl", "add_key",
            "request_key", "userfaultfd", "io_uring_setup", "open_by_handle_at",
        ):
            number = library.seccomp_syscall_resolve_name(name.encode())
            if number < 0 or library.seccomp_rule_add(context, 0x00050001, number, 0) != 0:
                raise RuntimeError("cannot restrict syscall: " + name)
        if library.seccomp_load(context) != 0:
            raise RuntimeError("cannot install seccomp filter")
    finally:
        library.seccomp_release(context)


class Sources(importlib.abc.MetaPathFinder, importlib.abc.SourceLoader):
    def __init__(self, sources):
        self.sources = sources

    def find_spec(self, fullname, path=None, target=None):
        stem = fullname.replace(".", "/")
        package = stem + "/__init__.py"
        filename = package if package in self.sources else stem + ".py"
        if filename not in self.sources:
            return None
        if fullname.split(".")[0] in sys.stdlib_module_names:
            raise ImportError("Policy module shadows Python standard library: " + fullname)
        return importlib.util.spec_from_loader(fullname, self, is_package=filename == package)

    def get_filename(self, fullname):
        stem = fullname.replace(".", "/")
        package = stem + "/__init__.py"
        return "/policy/" + (package if package in self.sources else stem + ".py")

    def get_data(self, path):
        if not path.startswith("/policy/") or path[8:] not in self.sources:
            raise OSError("undeclared Policy data: " + path)
        return self.sources[path[8:]].encode("utf-8")

    def get_resource_reader(self, fullname):
        # pkgutil.get_data uses get_data; importlib.resources is not yet supported.
        return None


def json_value(value, depth=0):
    if depth > 32:
        raise ValueError("result nesting exceeds 32")
    if value is None or type(value) in (str, bool):
        return
    if type(value) is int and -(2**63) <= value < 2**63:
        return
    if type(value) is float and math.isfinite(value):
        return
    if type(value) is list:
        for item in value:
            json_value(item, depth + 1)
        return
    if type(value) is dict and all(type(key) is str for key in value):
        for item in value.values():
            json_value(item, depth + 1)
        return
    raise TypeError("result must contain only JSON values with bounded integers")


def main():
    if sys.version != MANIFEST["python"]:
        raise RuntimeError("compiled Policy Python version mismatch")
    with open("/request.json", "rb") as stream:
        request = json.load(stream)
    restrict()
    sources = MANIFEST["files"]
    for filename in sources:
        if filename.endswith(".py"):
            root_name = filename.split("/")[0].removesuffix(".py")
            if root_name in sys.stdlib_module_names:
                raise ValueError("Policy module shadows Python standard library: " + root_name)
    binding = MANIFEST["binding"]
    if request.get("check") is True:
        # Compilation inspects syntax and binding. Evaluation loads fixed source,
        # but does not reinterpret the YAML or repeat this static check.
        for filename, source in sources.items():
            if filename.endswith(".py"):
                ast.parse(source, filename="/policy/" + filename)
        tree = ast.parse(sources[binding["file"]])
        functions = [node for node in tree.body if isinstance(node, ast.FunctionDef)
                     and node.name == binding["function"] and not node.decorator_list]
        if len(functions) != 1:
            raise ValueError("entry must be one top-level undecorated synchronous function")
        # No namespace packages and no implicit package files: all imports must
        # resolve through the declared files or the deployment standard library.
        for filename in sources:
            if filename.endswith(".py"):
                parts = filename.split("/")
                for index in range(1, len(parts)):
                    if "/".join(parts[:index]) + "/__init__.py" not in sources:
                        raise ValueError("missing declared __init__.py for " + filename)
    sys.meta_path.insert(0, Sources(sources))
    module_name = binding["file"][:-3].replace("/", ".")
    if module_name.endswith(".__init__"):
        module_name = module_name[:-9]
    module = importlib.import_module(module_name)
    function = getattr(module, binding["function"])
    if not inspect.isfunction(function) or function.__module__ != module_name:
        raise TypeError("entry is not a function defined in the declared module")
    signature = inspect.signature(function)
    signature.bind({}, policy_config={}, capabilities=None)
    if inspect.iscoroutinefunction(function) or inspect.isgeneratorfunction(function):
        raise TypeError("async and generator entries are not supported")
    if request.get("check") is True:
        result = True
    else:
        # No capability has been authorized in this prototype. This is deliberately
        # not an object exposing host functions or state lookups.
        result = function(request["data"], policy_config=binding["policy_config"], capabilities=None)
    json_value(result)
    encoded = json.dumps(result, allow_nan=False, separators=(",", ":")).encode("utf-8")
    if len(encoded) > 1024 * 1024:
        raise ValueError("result exceeds size limit")
    message = len(encoded).to_bytes(4, "big") + encoded
    while message:
        count = os.write(3, message)
        message = message[count:]


if __name__ == "__main__":
    main()
