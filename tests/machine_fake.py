"""Public snapshot/focus responses shared by the launcher PTY boundary doubles."""
API_ROUTES = r'''
import json, sys, os
if sys.argv[1:] == ['plugin', 'config-dir', 'herdrpp']:
    print(os.path.join(os.path.dirname(os.environ['HERDR_CONFIG_PATH']), 'plugins/config/herdrpp'))
    sys.exit(0)

def machine_api(arguments):
    name = 'Local'
    args = list(arguments)
    if args[:1] == ['--machine']:
        name = 'Alpha'
        args = args[2:]
    elif args[:1] == ['--session']:
        args = args[2:]
    if args == ['api', 'snapshot']:
        return {'result': {'snapshot': {
            'workspaces': [{'workspace_id': 'w1', 'label': name, 'number': 1, 'active_tab_id': 'w1:t1', 'worktree': None}],
            'tabs': [{'workspace_id': 'w1', 'tab_id': 'w1:t1', 'label': 'Terminal', 'number': 1}],
            'focused_workspace_id': 'w1', 'focused_tab_id': 'w1:t1'}}}
    if len(args) == 3 and args[0] in ['workspace', 'tab'] and args[1] == 'focus':
        kind = args[0]
        return {'result': {'type': kind + '_info', kind: {kind + '_id': args[2]}}}
    return None
response = machine_api(sys.argv[1:])
if response is not None:
    print(json.dumps(response))
    sys.exit(0)
'''


_screens = {}

def wait_workspace_picker(fd, label, reset=True):
    import os
    import select
    import time
    import pyte
    if reset or fd not in _screens:
        screen = pyte.Screen(100, 24)
        _screens[fd] = (screen, pyte.Stream(screen))
    screen, stream = _screens[fd]
    deadline = time.monotonic() + 8
    while True:
        text = '\n'.join(screen.display)
        if f'Machine: {label}' in text and '1 results' in text and 'Loading' not in text:
            return text
        assert time.monotonic() < deadline, text
        if select.select([fd], [], [], .05)[0]:
            stream.feed(os.read(fd, 65536).decode('utf-8', errors='replace'))
