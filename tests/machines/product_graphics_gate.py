"""Render the actual machine launcher/picker in isolated reference Kitty.

Run through xvfb-run + kitty. Required env: MACHINE_GRAPHICS_BINARY. Evidence
is written under MACHINE_GRAPHICS_OUTPUT (default /output/graphics). Geometry
changes cover both a manual proxy PTY resize and the actual Kitty X11 window.
"""
import base64
import fcntl
import json
import hashlib
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import traceback
import tty

from PIL import Image

OUTPUT = Path(os.environ.get('MACHINE_GRAPHICS_OUTPUT', '/output/graphics'))
BINARY = os.environ['MACHINE_GRAPHICS_BINARY']
OUTPUT.mkdir(parents=True, exist_ok=True)
(OUTPUT / 'outcome.txt').write_text('FAIL\n')

CONFIG = '''onboarding = false
[terminal]
default_shell = "/bin/sh"
shell_mode = "non_login"
[update]
version_check = false
manifest_check = false
[ui.toast]
delivery = "off"
[ui.sound]
enabled = false
[remote]
manage_ssh_config = false
'''


import codecs
import pyte
from terminal import Screen


class Client:
    def __init__(self, environment, arguments=()):
        self.master, slave = pty.openpty()
        self.size = os.get_terminal_size(0)
        self.screen = Screen(self.size.columns, self.size.lines)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')
        geometry = fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
        fcntl.ioctl(slave, termios.TIOCSWINSZ, geometry)
        self.process = subprocess.Popen([BINARY, 'launch', *arguments], env=environment,
                                        stdin=slave, stdout=slave, stderr=slave,
                                        start_new_session=True)
        os.close(slave)
        self.wire = bytearray()
        self.host_resized = False
        self.original_winch = signal.signal(signal.SIGWINCH, self.host_resize)

    @property
    def text(self):
        return '\n'.join(self.screen.display)

    def read(self, timeout=.05):
        self.pump(timeout)

    def wait(self, *needles, timeout=20):
        deadline = time.monotonic() + timeout
        while not all(needle in self.text for needle in needles):
            assert self.process.poll() is None, self.text
            assert time.monotonic() < deadline, self.text
            self.pump(.05)

    def host_resize(self, *_):
        self.host_resized = True

    def forward_host_geometry(self):
        geometry = fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, geometry)
        os.kill(self.process.pid, signal.SIGWINCH)
        self.host_resized = False

    def send(self, data):
        offset = 0
        while offset < len(data):
            offset += os.write(self.master, data[offset:])

    def pump(self, duration):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            if self.host_resized:
                self.forward_host_geometry()
            ready = select.select([0, self.master], [], [], .02)[0]
            if 0 in ready:
                self.send(os.read(0, 65536))
            if self.master in ready:
                data = os.read(self.master, 1048576)
                self.wire.extend(data)
                self.stream.feed(self.decoder.decode(data))
                os.write(1, data)

    def wait_output(self, needle, start=0):
        deadline = time.monotonic() + 15
        while needle not in self.wire[start:]:
            assert self.process.poll() is None, self.wire.decode('utf-8', 'replace')
            assert time.monotonic() < deadline, self.wire.decode('utf-8', 'replace')
            self.pump(.05)
        self.pump(.2)

    def resize(self, columns, lines):
        fcntl.ioctl(self.master, termios.TIOCSWINSZ,
                    struct.pack('HHHH', lines, columns, 0, 0))
        os.kill(self.process.pid, signal.SIGWINCH)
        self.pump(.5)

    def close(self):
        signal.signal(signal.SIGWINCH, self.original_winch)
        os.killpg(self.process.pid, signal.SIGTERM)
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=5)
        os.close(self.master)


def window_geometry():
    windows = subprocess.run(['xdotool', 'search', '--onlyvisible', '--class', 'kitty'],
                             check=True, capture_output=True, text=True).stdout.splitlines()
    assert len(windows) == 1, windows
    output = subprocess.run(['xdotool', 'getwindowgeometry', '--shell', windows[0]],
                            check=True, capture_output=True, text=True).stdout
    geometry = dict(line.split('=', 1) for line in output.splitlines())
    return dict(window_id=windows[0], width=int(geometry['WIDTH']), height=int(geometry['HEIGHT']))


def resize_host_window(client, width, height):
    before = fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
    window = window_geometry()
    subprocess.run(['xdotool', 'windowsize', window['window_id'], str(width), str(height)],
                   check=True, capture_output=True)
    deadline = time.monotonic() + 10
    while fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)) == before:
        assert time.monotonic() < deadline, 'Kitty did not update its actual host PTY geometry'
        client.pump(.05)
    client.pump(.3)
    host = fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
    proxy = fcntl.ioctl(client.master, termios.TIOCGWINSZ, bytes(8))
    actual = window_geometry()
    assert actual['window_id'] == window['window_id']
    assert actual['width'] == width and actual['height'] == height, actual
    assert host != before
    assert proxy == host, (struct.unpack('HHHH', host), struct.unpack('HHHH', proxy))
    return dict(window=actual, host_winsize=struct.unpack('HHHH', host),
                proxy_winsize=struct.unpack('HHHH', proxy))


def capture(name):
    subprocess.run(['import', '-window', 'root', str(OUTPUT / (name + '.png'))],
                   check=True, capture_output=True)
    pixels = list(Image.open(OUTPUT / (name + '.png')).convert('RGB').getdata())
    return dict(red=sum(1 for r, g, b in pixels if r > 220 and g < 30 and b < 30),
                blue=sum(1 for r, g, b in pixels if b > 220 and g < 30 and r < 30))


def wait_for_fixture_marker(client, path, expected):
    deadline = time.monotonic() + 10
    while not path.exists():
        assert time.monotonic() < deadline, f'busy fixture did not emit {expected!r}'
        assert client.process.poll() is None
        client.pump(.05)
    assert path.read_text() == expected


def start_server(root, environment):
    log = open(root / 'server.log', 'w')
    server = subprocess.Popen(['herdr', 'server'], env=environment, stdout=log,
                              stderr=log, start_new_session=True)
    log.close()
    deadline = time.monotonic() + 15
    while subprocess.run(['herdr', 'api', 'snapshot'], env=environment,
                          capture_output=True).returncode:
        assert server.poll() is None, (root / 'server.log').read_text()
        assert time.monotonic() < deadline
        time.sleep(.05)
    subprocess.run(['herdr', 'workspace', 'create', '--label', 'GRAPHICS_BUSY', '--focus'],
                   env=environment, check=True, capture_output=True)
    return server


def run(root):
    environment = {k: v for k, v in os.environ.items()
                   if not k.startswith(('HERDR_', 'XDG_'))}
    environment.update(TERM='xterm-kitty', SHELL='/bin/sh', LANG='C.UTF-8')
    for key, directory in [('XDG_CONFIG_HOME', 'config'), ('XDG_STATE_HOME', 'state'),
                           ('XDG_DATA_HOME', 'data'), ('XDG_RUNTIME_DIR', 'runtime')]:
        environment[key] = str(root / directory)
        (root / directory).mkdir()
    config = root / 'config/herdr/config.toml'
    config.parent.mkdir()
    config.write_text(CONFIG)
    environment['HERDR_CONFIG_PATH'] = str(config)
    script = root / 'busy.sh'
    red = base64.b64encode(bytes([255, 0, 0]) * 400).decode()
    blue = base64.b64encode(bytes([0, 0, 255]) * 400).decode()
    trigger = root / 'emit-blue'
    emitted = root / 'blue-emitted'
    emitted_upload = root / 'emitted-upload.ansi'
    script.write_text(
        "set -e\n"
        f"printf '%s\\n' $$ > {root / 'busy.pid'}\n"
        "printf '\\033[5;5H\\033_Ga=T,f=24,s=20,v=20,i=123,q=2;" + red + "\\033\\\\'\n"
        f"while [ ! -e {trigger} ]; do printf '\\033[12;5HBUSY_WAITING'; sleep .05; done\n"
        "printf '\\033[8;5H\\033_Ga=T,f=24,s=20,v=20,i=124,q=2;" + blue +
        "\\033\\\\' | tee " + str(emitted_upload) + "\n"
        f"printf '124\\n' > {emitted}.tmp\nmv {emitted}.tmp {emitted}\n"
        "while :; do printf '\\033[12;5HBUSY_RUNNING'; sleep .05; done\n"
    )
    (OUTPUT / 'metadata.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(Path(BINARY).read_bytes()).hexdigest(), herdr=subprocess.run(['herdr', '--version'], capture_output=True, text=True, check=True).stdout.strip(), kitty=subprocess.run(['kitty', '--version'], capture_output=True, text=True, check=True).stdout.strip(), resize_kind='actual Kitty X11 window with full winsize/SIGWINCH forwarding, plus manual proxy PTY; fixed Xvfb framebuffer'), indent=2))
    server = start_server(root, environment)
    original = termios.tcgetattr(0)
    results = []
    client = None
    passed = False
    try:
        tty.setraw(0)
        client = Client(environment)
        original_window = window_geometry()
        original_host_geometry = fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
        client.wait_output(b'GRAPHICS_BUSY')
        client.send(f'/bin/sh {script}\r'.encode())
        client.pump(.7)
        busy_pid = int((root / 'busy.pid').read_text())
        os.kill(busy_pid, 0)
        initial_snapshot = json.loads(subprocess.run(['herdr', 'api', 'snapshot'], env=environment, check=True, capture_output=True, text=True).stdout)['result']['snapshot']
        results.append(dict(case='before-picker', **capture('before-picker')))
        start = len(client.wire)
        client.send(b'\x1d')
        client.wait_output(b'Search workspaces', start)
        results.append(dict(case='picker-open', **capture('picker-open')))
        client.resize(client.size.columns - 5, client.size.lines - 2)
        results.append(dict(case='picker-pty-resized', **capture('picker-pty-resized')))
        resize_start = len(client.wire)
        grown = resize_host_window(client, original_window['width'] + 180, original_window['height'] + 120)
        client.wait_output(b'Search workspaces', resize_start)
        results.append(dict(case='picker-host-window-resized', **capture('picker-host-window-resized')))
        trigger.touch()
        wait_for_fixture_marker(client, emitted, '124\n')
        expected_upload = b'\x1b[8;5H\x1b_Ga=T,f=24,s=20,v=20,i=124,q=2;' + blue.encode() + b'\x1b\\'
        assert emitted_upload.read_bytes() == expected_upload
        (OUTPUT / 'emitted-upload.ansi').write_bytes(emitted_upload.read_bytes())
        results.append(dict(case='picker-new-image-emitted', **capture('picker-new-image-emitted')))
        (OUTPUT / 'emission.json').write_text(json.dumps(dict(image_id=124,
            busy_process_pid=busy_pid, emitted_marker=emitted.read_text(),
            phase='after picker title and physical resize redraw; before cancel'), indent=2))
        client.send(b'\x1b')
        client.pump(2)
        results.append(dict(case='after-cancel', **capture('after-cancel')))
        start = len(client.wire)
        client.send(b'\x1d')
        client.wait_output(b'Search workspaces', start)
        results.append(dict(case='picker-reopened', **capture('picker-reopened')))
        resize_start = len(client.wire)
        restored = resize_host_window(client, original_window['width'], original_window['height'])
        client.wait_output(b'Search workspaces', resize_start)
        assert fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)) == original_host_geometry
        results.append(dict(case='picker-host-window-restored', **capture('picker-host-window-restored')))
        client.send(b'\x1b')
        client.pump(2)
        results.append(dict(case='after-reopen-cancel', **capture('after-reopen-cancel')))
        (OUTPUT / 'host-resize.json').write_text(json.dumps(dict(original_window=original_window, grown=grown, restored=restored), indent=2))
        by_case = {result['case']: result for result in results}
        assert by_case['before-picker']['red'] > 0
        assert by_case['before-picker']['blue'] == 0
        for case in ['picker-open', 'picker-pty-resized', 'picker-host-window-resized',
                     'picker-new-image-emitted', 'picker-reopened', 'picker-host-window-restored']:
            assert by_case[case]['red'] == by_case[case]['blue'] == 0, by_case[case]
        for case in ['after-cancel', 'after-reopen-cancel']:
            assert by_case[case]['red'] > 0 and by_case[case]['blue'] > 0, by_case[case]
        assert client.process.poll() is None
        assert int((root / 'busy.pid').read_text()) == busy_pid
        os.kill(busy_pid, 0)
        final_snapshot = json.loads(subprocess.run(['herdr', 'api', 'snapshot'], env=environment, check=True, capture_output=True, text=True).stdout)['result']['snapshot']
        assert [pane['terminal_id'] for pane in initial_snapshot['panes']] == [pane['terminal_id'] for pane in final_snapshot['panes']]
        (OUTPUT / 'identity.json').write_text(json.dumps(dict(busy_pid=busy_pid, preserved_terminal_ids=[pane['terminal_id'] for pane in final_snapshot['panes']]), indent=2))
        passed = True
    except BaseException:
        (OUTPUT / 'failure.txt').write_text(traceback.format_exc())
        raise
    finally:
        (OUTPUT / 'outcome.txt').write_text('PASS\n' if passed else 'FAIL\n')
        (OUTPUT / 'results.json').write_text(json.dumps(results, indent=2))
        if client is not None:
            (OUTPUT / 'wire.ansi').write_bytes(client.wire)
            client.close()
        termios.tcsetattr(0, termios.TCSANOW, original)
        subprocess.run(['herdr', 'server', 'stop'], env=environment, capture_output=True)
        server.wait(timeout=5)


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='hg-') as temporary:
        run(Path(temporary))
