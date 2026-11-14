"""Internal selected-path export/publication helper. No whole-mount walk.

Host writes use pinned directory descriptors and O_NOFOLLOW at every component.
Receipts are written after each successful operation; retry never re-deletes an
already published opaque directory. No host/base conflict comparison occurs.
"""
import json
import os
import shutil
import stat
import sys

operation, root, package = sys.argv[1:]
NOFOLLOW = os.O_NOFOLLOW | os.O_DIRECTORY


def save(name, value):
    temporary = os.path.join(package, name + '.tmp')
    with open(temporary, 'w') as output:
        json.dump(value, output)
    os.replace(temporary, os.path.join(package, name))


def components(path):
    parts = path.split('/')
    if any(p in ('', '.', '..') for p in parts):
        raise ValueError('invalid relative publication path: ' + path)
    return parts


if operation == 'export':
    with open(os.path.join(package, 'selection.json')) as source:
        selection = json.load(source)
    os.mkdir(os.path.join(package, 'data'))
    nodes = {}
    data_index = 0

    def export(path, recursive=False):
        global data_index
        parts = components(path)
        # Selection can contain a stale descendant from an older delta layer.
        # Never follow its final-view ancestor symlink out of the mount.
        ancestor = root
        for part in parts[:-1]:
            ancestor = os.path.join(ancestor, part)
            try:
                ancestor_info = os.lstat(ancestor)
            except (FileNotFoundError, NotADirectoryError):
                return
            if not stat.S_ISDIR(ancestor_info.st_mode):
                return
        if path in nodes:
            if recursive and nodes[path]['kind'] == 'directory':
                nodes[path]['subtree'] = True
                for child in sorted(os.listdir(os.path.join(root, path))):
                    export(path + '/' + child, True)
            return
        source = os.path.join(root, path)
        try:
            info = os.lstat(source)
        except (FileNotFoundError, NotADirectoryError):
            nodes[path] = {'kind': 'absent'}
            return
        node = {'mode': stat.S_IMODE(info.st_mode)}
        if stat.S_ISDIR(info.st_mode):
            node['kind'] = 'directory'
            node['subtree'] = recursive
        elif stat.S_ISREG(info.st_mode):
            if info.st_nlink > 1:
                raise RuntimeError('materialization capability limitation: hard-linked source ' + path)
            node['kind'] = 'file'
            node['data'] = str(data_index)
            data_index += 1
            shutil.copyfile(source, os.path.join(package, 'data', node['data']))
        elif stat.S_ISLNK(info.st_mode):
            node['kind'] = 'symlink'
            node['link'] = os.readlink(source)
        else:
            raise RuntimeError('materialization capability limitation: special file ' + path)
        nodes[path] = node
        if recursive and node['kind'] == 'directory':
            for child in sorted(os.listdir(source)):
                export(path + '/' + child, True)

    for item in selection:
        export(item['path'], item['subtree'])
    # Copy-up directories are selected individually, not recursively. Only
    # opaque/redirect subtrees require enumerating their final descendants.
    save('nodes.json', nodes)
    save('root.json', {'mode': stat.S_IMODE(os.stat(root).st_mode)})
    save('ready', True)
    sys.exit(0)

try:
    with open(os.path.join(package, 'receipts.json')) as source:
        receipts = json.load(source)
except FileNotFoundError:
    receipts = {'paths': [], 'modes': [], 'published': False}
if receipts['published']:
    sys.exit(0)
with open(os.path.join(package, 'nodes.json')) as source:
    nodes = json.load(source)
with open(os.path.join(package, 'root.json')) as source:
    root_mode = json.load(source)['mode']

# Traversal and chmod do not require directory read permission. O_PATH pins the
# inode without temporarily widening its permissions. Only enumeration opens a
# readable descriptor (and still fails honestly when that is not permitted).
root_fd = os.open(root, os.O_PATH | NOFOLLOW)
directories = {}


def remember(fd):
    info = os.fstat(fd)
    key = str(info.st_dev) + ':' + str(info.st_ino)
    if key not in directories:
        directories[key] = (os.dup(fd), info)


def remove(fd, name):
    try:
        info = os.stat(name, dir_fd=fd, follow_symlinks=False)
    except FileNotFoundError:
        return
    remember(fd)
    if stat.S_ISDIR(info.st_mode):
        child = os.open(name, os.O_RDONLY | NOFOLLOW, dir_fd=fd)
        try:
            for entry in os.listdir(child):
                remove(child, entry)
        finally:
            os.close(child)
        os.rmdir(name, dir_fd=fd)
    else:
        os.unlink(name, dir_fd=fd)


def parent(path):
    parts = components(path)
    fd = os.dup(root_fd)
    try:
        for part in parts[:-1]:
            try:
                child = os.open(part, os.O_PATH | NOFOLLOW, dir_fd=fd)
            except (FileNotFoundError, NotADirectoryError):
                remove(fd, part)
                remember(fd)
                os.mkdir(part, dir_fd=fd)
                child = os.open(part, os.O_PATH | NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd, parts[-1]
    except BaseException:
        os.close(fd)
        raise


def equal_file(fd, name, source):
    try:
        target = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NOATIME, dir_fd=fd)
    except (FileNotFoundError, OSError):
        return False
    with os.fdopen(target, 'rb') as left, open(source, 'rb') as right:
        if not stat.S_ISREG(os.fstat(left.fileno()).st_mode):
            return False
        while True:
            a, b = left.read(1024 * 1024), right.read(1024 * 1024)
            if a != b:
                return False
            if not a:
                return True


try:
    for path, node in sorted(nodes.items(), key=lambda item: (item[0].count('/'), item[0])):
        if path in receipts['paths']:
            continue
        if not path:
            continue
        fd, name = parent(path)
        try:
            try:
                old = os.stat(name, dir_fd=fd, follow_symlinks=False)
            except FileNotFoundError:
                old = None
            kind = node['kind']
            if kind == 'absent':
                remove(fd, name)
            elif kind == 'directory':
                if old is not None and not stat.S_ISDIR(old.st_mode):
                    remove(fd, name)
                    old = None
                if old is None:
                    remember(fd)
                    os.mkdir(name, 0o700, dir_fd=fd)
                if node.get('subtree'):
                    child = os.open(name, os.O_RDONLY | NOFOLLOW, dir_fd=fd)
                    try:
                        for entry in os.listdir(child):
                            if path + '/' + entry not in nodes:
                                remove(child, entry)
                    finally:
                        os.close(child)
            elif kind == 'symlink':
                if old is not None and stat.S_ISLNK(old.st_mode) and os.readlink(name, dir_fd=fd) == node['link']:
                    pass
                else:
                    if old is not None and stat.S_ISLNK(old.st_mode):
                        raise RuntimeError('materialization capability limitation: replacing existing symlink attributes ' + path)
                    remove(fd, name)
                    remember(fd)
                    os.symlink(node['link'], name, dir_fd=fd)
            elif kind == 'file':
                source = os.path.join(package, 'data', node['data'])
                if old is not None and stat.S_ISREG(old.st_mode) and old.st_nlink > 1:
                    raise RuntimeError('materialization capability limitation: hard-linked destination ' + path)
                if old is not None and stat.S_ISREG(old.st_mode) and equal_file(fd, name, source):
                    target = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=fd)
                    try:
                        os.fchmod(target, node['mode'])
                    finally:
                        os.close(target)
                else:
                    if old is not None and not stat.S_ISREG(old.st_mode):
                        remove(fd, name)
                        old = None
                    remember(fd)
                    temporary = '.ascout-publish-' + str(os.getpid())
                    original = None
                    try:
                        if old is not None:
                            # Pin the original inode before collecting attributes;
                            # pathname-based cp could race a substituted symlink.
                            original = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NOATIME, dir_fd=fd)
                            old = os.fstat(original)
                            attributes = {key: os.getxattr(original, key) for key in os.listxattr(original)}
                        output = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
                        with os.fdopen(output, 'wb') as destination, open(source, 'rb') as input:
                            shutil.copyfileobj(input, destination)
                            destination.flush()
                            target = destination.fileno()
                            if old is not None:
                                current = os.fstat(target)
                                if (current.st_uid, current.st_gid) != (old.st_uid, old.st_gid):
                                    os.fchown(target, old.st_uid, old.st_gid)
                                for key in os.listxattr(target):
                                    if key not in attributes:
                                        os.removexattr(target, key)
                                for key, value in attributes.items():
                                    os.setxattr(target, key, value)
                                os.utime(target, ns=(old.st_atime_ns, old.st_mtime_ns))
                            os.fchmod(target, node['mode'])
                        os.rename(temporary, name, src_dir_fd=fd, dst_dir_fd=fd)
                    finally:
                        if original is not None:
                            os.close(original)
                        try:
                            os.unlink(temporary, dir_fd=fd)
                        except FileNotFoundError:
                            pass
            receipts['paths'].append(path)
            save('receipts.json', receipts)
        finally:
            os.close(fd)
finally:
    # Child creation/deletion must not reset untouched parent timestamps.
    for fd, info in directories.values():
        try:
            os.utime('/proc/self/fd/' + str(fd), ns=(info.st_atime_ns, info.st_mtime_ns))
        finally:
            os.close(fd)

# Directory permissions last, including the mount root itself. Directory
# receipts are written only after this phase, so a failure remains retryable.
for path, node in sorted(nodes.items(), key=lambda item: (-item[0].count('/'), item[0])):
    if node['kind'] == 'directory' and path not in receipts['modes']:
        if path:
            fd, name = parent(path)
            target = os.open(name, os.O_PATH | NOFOLLOW, dir_fd=fd)
        else:
            fd, target = -1, os.dup(root_fd)
        try:
            # Linux fchmod rejects O_PATH descriptors. The proc fd link refers
            # to this pinned inode, never a re-resolved user-controlled path.
            os.chmod('/proc/self/fd/' + str(target), node['mode'])
        finally:
            os.close(target)
            if fd >= 0:
                os.close(fd)
        receipts['modes'].append(path)
        save('receipts.json', receipts)
if '' not in receipts['modes']:
    os.chmod('/proc/self/fd/' + str(root_fd), root_mode)
    receipts['modes'].append('')
    save('receipts.json', receipts)
receipts['published'] = True
save('receipts.json', receipts)
os.close(root_fd)
