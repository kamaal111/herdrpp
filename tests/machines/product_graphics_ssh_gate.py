"""Actual rendered graphics across Local -> Alpha -> Local -> Alpha attachments."""
import base64
import hashlib
import json
import os
from pathlib import Path
import signal
import tempfile
import termios
import time
import traceback
import tty

from lab import Lab
from picker_driver import open_picker, browse_machine
from product_graphics_gate import BINARY, OUTPUT, Client, capture


def color_script(root, color, label):
    script = root / (label + '.sh')
    pidfile = root / (label + '.pid')
    payload = base64.b64encode(bytes(color) * 400).decode()
    script.write_text(f"printf '%s\\n' $$ > {pidfile}; " +
                      "printf '\\033[5;5H\\033_Ga=T,f=24,s=20,v=20,i=123,q=2;" + payload +
                      "\\033\\\\'; while :; do printf '\\033[12;5H" + label +
                      "'; sleep .05; done\n")
    return script, pidfile


def wait_color(client, name, color):
    deadline = time.monotonic() + 15
    pixels = capture(name)
    while pixels[color] == 0:
        assert time.monotonic() < deadline, pixels
        assert client.process.poll() is None
        client.pump(.1)
        pixels = capture(name)
    return pixels


def pick(client, label, workspace):
    start = len(client.wire)
    open_picker(client)
    browse_machine(client, label)
    client.send(b'\r')
    client.wait_output(workspace, start)
    client.pump(.5)
    return start


def run(root):
    lab = Lab(root / 'lab')
    client = None
    original = termios.tcgetattr(0)
    results = []
    passed = False
    try:
        lab.start()
        lab.env['TERM'] = 'xterm-kitty'
        profiles = json.loads(lab.run('herdr', 'machine', 'list', '--json').stdout)
        alpha_profile = next(profile for profile in profiles if profile['label'] == 'Alpha')
        tty.setraw(0)
        client = Client(lab.env, ['--machine', alpha_profile['id']])
        command = b"printf 'REAL_HOST_EARLY_INPUT_OK\\n'\r"
        client.send(command)
        expected_line = 'REAL_HOST_EARLY_INPUT_OK'
        deadline = time.monotonic() + 20
        pane_text = lab.pane_text('alpha')
        while expected_line not in pane_text.splitlines():
            assert time.monotonic() < deadline, pane_text
            assert client.process.poll() is None
            client.pump(.1)
            pane_text = lab.pane_text('alpha')
        assert expected_line in pane_text.splitlines()
        (OUTPUT / 'startup.json').write_text(json.dumps(dict(
            exact_remote_output_line=expected_line, sent_before_first_client_read=True,
            binary_sha256=hashlib.sha256(Path(BINARY).read_bytes()).hexdigest()), indent=2))
        (OUTPUT / 'startup-wire.ansi').write_bytes(client.wire)
        capture('immediate-remote-startup')
        client.close()
        client = None
        local_script, local_pidfile = color_script(root, [255, 0, 0], 'LOCAL_GRAPHICS_BUSY')
        alpha_script, alpha_pidfile = color_script(root, [0, 0, 255], 'ALPHA_GRAPHICS_BUSY')
        local = lab.snapshot('default', remote=False)['panes'][0]
        alpha = lab.snapshot('alpha')['panes'][0]
        lab.run('herdr', 'pane', 'run', local['pane_id'], f'/bin/sh {local_script}')
        lab.run('herdr', '--session', 'alpha', 'pane', 'run', alpha['pane_id'],
                f'/bin/sh {alpha_script}', env=lab.remote)
        tty.setraw(0)
        client = Client(lab.env)
        client.wait_output(b'LOCAL_ONLY')
        results.append(dict(case='local-before-switch', **wait_color(client, 'local-before-switch', 'red')))
        pick(client, 'Alpha', b'ALPHA_ONLY')
        results.append(dict(case='alpha-first-attachment', **wait_color(client, 'alpha-first-attachment', 'blue')))
        local_start = pick(client, 'Local', b'LOCAL_ONLY')
        results.append(dict(case='local-reattached', **wait_color(client, 'local-reattached', 'red')))
        local_reupload = b'\x1b_Ga=t,' in client.wire[local_start:]
        alpha_start = pick(client, 'Alpha', b'ALPHA_ONLY')
        results.append(dict(case='alpha-reattached', **wait_color(client, 'alpha-reattached', 'blue')))
        alpha_reupload = b'\x1b_Ga=t,' in client.wire[alpha_start:]
        local_pid = int(local_pidfile.read_text())
        alpha_pid = int(alpha_pidfile.read_text())
        os.kill(local_pid, 0)
        os.kill(alpha_pid, 0)
        assert lab.snapshot('default', remote=False)['panes'][0]['terminal_id'] == local['terminal_id']
        assert lab.snapshot('alpha')['panes'][0]['terminal_id'] == alpha['terminal_id']
        assert results[0]['red'] > 0 and results[0]['blue'] == 0
        assert results[1]['blue'] > 0 and results[1]['red'] == 0
        assert results[2]['red'] > 0 and results[2]['blue'] == 0
        assert results[3]['blue'] > 0 and results[3]['red'] == 0
        assert local_reupload
        assert alpha_reupload
        assert client.process.poll() is None
        (OUTPUT / 'identity.json').write_text(json.dumps(dict(local_pid=local_pid,
            alpha_pid=alpha_pid, local_terminal_id=local['terminal_id'],
            alpha_terminal_id=alpha['terminal_id'], local_image_reuploaded=local_reupload,
            alpha_image_reuploaded=alpha_reupload,
            binary_sha256=hashlib.sha256(Path(BINARY).read_bytes()).hexdigest()), indent=2))
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
        lab.close()


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='hg-ssh-') as temporary:
        run(Path(temporary))
