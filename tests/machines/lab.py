"""Independent Herdr sessions behind distinct loopback SSH endpoints in one container."""
import json
import os
from pathlib import Path
import socket
import subprocess
import time

CONFIG = '''onboarding = false
[theme]
name = "dracula"
[terminal]
default_shell = "/bin/sh"
shell_mode = "non_login"
[update]
version_check = false
manifest_check = false
[ui]
sidebar_width = 30
[ui.toast]
delivery = "off"
[ui.sound]
enabled = false
[remote]
manage_ssh_config = false
'''


def clean_environment():
    env = {k: v for k, v in os.environ.items() if not k.startswith(('HERDR_', 'XDG_'))}
    env.update(TERM='xterm-256color', LANG='C.UTF-8', SHELL='/bin/sh', PS1='$ ')
    return env


class Lab:
    def __init__(self, root):
        self.root = Path(root)
        self.root.mkdir(parents=True)
        self.env = clean_environment()
        for key, directory in [('XDG_CONFIG_HOME', 'config'), ('XDG_STATE_HOME', 'state'),
                               ('XDG_DATA_HOME', 'data'), ('XDG_RUNTIME_DIR', 'runtime')]:
            self.env[key] = str(self.root / directory)
            (self.root / directory).mkdir()
        self.env['HERDR_CONFIG_PATH'] = str(self.root / 'config/herdr/config.toml')
        config = Path(self.env['HERDR_CONFIG_PATH'])
        config.parent.mkdir(parents=True)
        config.write_text(CONFIG)
        self.remote = dict(self.env)
        self.remote.update(XDG_CONFIG_HOME=str(self.root / 'remote-config'),
                           XDG_STATE_HOME=str(self.root / 'remote-state'),
                           HERDR_CONFIG_PATH=str(self.root / 'remote-config/herdr/config.toml'))
        remote_config = Path(self.remote['HERDR_CONFIG_PATH'])
        remote_config.parent.mkdir(parents=True)
        remote_config.write_text(CONFIG)
        self.sshds = {}
        self.targets = {}
        self.clients = []
        self.sessions = []

    def run(self, *args, env=None, check=True):
        return subprocess.run(args, env=env or self.env, cwd=self.root, text=True,
                              capture_output=True, check=check, timeout=30)

    def api(self, *args, env=None):
        result = self.run('herdr', *args, env=env)
        return json.loads(result.stdout)['result']

    def start(self):
        self.run('ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(self.root / 'key'))
        self.run('ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(self.root / 'host'))
        host_key = (self.root / 'host.pub').read_text().split()
        known_hosts = []
        allocated_ports = set()
        for endpoint in ['alpha', 'beta']:
            while True:
                with socket.socket() as sock:
                    sock.bind(('127.0.0.1', 0))
                    port = sock.getsockname()[1]
                if port not in allocated_ports:
                    allocated_ports.add(port)
                    break
            config = self.root / f'{endpoint}-sshd_config'
            config.write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {self.root}/host
PidFile {self.root}/{endpoint}-sshd.pid
AuthorizedKeysFile {self.root}/key.pub
PermitRootLogin yes
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
AcceptEnv XDG_CONFIG_HOME XDG_STATE_HOME HERDR_CONFIG_PATH
''')
            log = open(self.root / f'{endpoint}-sshd.log', 'w')
            self.sshds[endpoint] = subprocess.Popen(
                ['/usr/sbin/sshd', '-D', '-e', '-f', str(config)], stdout=log, stderr=log)
            log.close()
            self.targets[endpoint] = f'ssh://root@127.0.0.1:{port}'
            known_hosts.append(f'[127.0.0.1]:{port} {host_key[0]} {host_key[1]}\n')
        (self.root / 'known_hosts').write_text(''.join(known_hosts))
        ssh_config = self.root / 'ssh_config'
        ssh_config.write_text(f'''Host *
  IdentityFile {self.root}/key
  UserKnownHostsFile {self.root}/known_hosts
  StrictHostKeyChecking yes
  BatchMode yes
  SetEnv XDG_CONFIG_HOME={self.remote['XDG_CONFIG_HOME']} XDG_STATE_HOME={self.remote['XDG_STATE_HOME']} HERDR_CONFIG_PATH={self.remote['HERDR_CONFIG_PATH']}
''')
        (self.root / 'bin').mkdir()
        shim = self.root / 'bin/ssh'
        shim.write_text(f'#!/bin/sh\nexec /usr/bin/ssh -F {ssh_config} "$@"\n')
        shim.chmod(0o700)
        self.env['PATH'] = str(self.root / 'bin') + ':' + self.env['PATH']
        self.target = self.targets['alpha']
        for endpoint, target in self.targets.items():
            deadline = time.monotonic() + 10
            while True:
                result = self.run('ssh', target, 'true', check=False)
                if result.returncode == 0:
                    break
                if time.monotonic() > deadline:
                    raise RuntimeError(result.stderr + (self.root / f'{endpoint}-sshd.log').read_text())
                time.sleep(.05)
        for session, label, env in [('default', 'LOCAL_ONLY', self.env),
                                    ('alpha', 'ALPHA_ONLY', self.remote),
                                    ('beta', 'BETA_ONLY', self.remote)]:
            log = open(self.root / f'{session}.log', 'w')
            process = subprocess.Popen(['herdr', '--session', session, 'server'], env=env,
                                       stdout=log, stderr=log, start_new_session=True)
            log.close()
            self.sessions.append((session, env, process))
            deadline = time.monotonic() + 15
            while True:
                result = self.run('herdr', '--session', session, 'api', 'snapshot', env=env, check=False)
                if result.returncode == 0:
                    break
                if time.monotonic() > deadline:
                    raise RuntimeError(result.stderr + (self.root / f'{session}.log').read_text())
                time.sleep(.05)
            self.api('--session', session, 'workspace', 'create', '--label', label, env=env)
        for session, label in [('alpha', 'Alpha'), ('beta', 'Beta')]:
            self.run('herdr', 'machine', 'add', self.targets[session], '--remote-session', session, '--label', label)
        return self

    def remote_endpoint_running(self, session):
        server = next(process for name, _, process in self.sessions if name == session)
        return self.sshds[session].poll() is None and server.poll() is None

    def snapshot(self, session, remote=True):
        return self.api('--session', session, 'api', 'snapshot',
                        env=self.remote if remote else self.env)['snapshot']

    def pane_text(self, session, remote=True, pane_id=None):
        env = self.remote if remote else self.env
        pane = pane_id
        if pane is None:
            pane = self.snapshot(session, remote=remote)['panes'][0]['pane_id']
        return self.run('herdr', '--session', session, 'pane', 'read', pane,
                        '--source', 'recent', '--lines', '100', env=env).stdout

    def close(self):
        for client in self.clients:
            client.close()
        for session, env, process in reversed(self.sessions):
            self.run('herdr', '--session', session, 'server', 'stop', env=env, check=False)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)
        for daemon in self.sshds.values():
            daemon.terminate()
            daemon.wait(timeout=5)
