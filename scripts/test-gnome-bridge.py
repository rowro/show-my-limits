"""Run under xvfb-run + dbus-run-session. Uses an isolated, empty app profile."""
import ast
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='sml-gnome-smoke-') as temporary:
    env = os.environ.copy()
    for name, folder in [('XDG_DATA_HOME', 'data'), ('XDG_CONFIG_HOME', 'config'), ('XDG_CACHE_HOME', 'cache')]:
        env[name] = str(Path(temporary) / folder)
        Path(env[name]).mkdir()
    env['WEBKIT_DISABLE_DMABUF_RENDERER'] = '1'
    # Capture is never configured, CA is not created and no client is launched.
    with open(Path(temporary) / 'app.log', 'w+') as log:
        app = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
        def call(method, *args, check=True):
            return subprocess.run(['gdbus', 'call', '--session', '--dest', 'io.github.rowro.ShowMyLimits',
                '--object-path', '/io/github/rowro/ShowMyLimits', '--method',
                f'io.github.rowro.ShowMyLimits1.{method}', *args],
                env=env, capture_output=True, text=True, timeout=5, check=check)
        try:
            deadline = time.monotonic() + 40
            while True:
                result = call('GetState', check=False)
                if result.returncode == 0 and result.stdout.strip() != "('null',)":
                    break
                if app.poll() is not None or time.monotonic() >= deadline:
                    raise AssertionError(f'GNOME bridge did not start (app exit={app.poll()})')
                time.sleep(0.25)
            snapshot = json.loads(ast.literal_eval(result.stdout)[0])
            assert set(snapshot) == {'protocol', 'now', 'phase', 'setupComplete', 'connectionIssue', 'profile'}, snapshot
            assert snapshot['protocol'] == 1 and snapshot['profile'] is None
            # The main window has never been shown. State must advance anyway.
            time.sleep(1.25)
            hidden = json.loads(ast.literal_eval(call('GetState').stdout)[0])
            assert hidden['now'] > snapshot['now'] + 1.0, 'state froze while the window was hidden'
            assert call('ToggleWindow', 'invalid-view', check=False).returncode != 0
            call('ToggleWindow', 'overview')
            call('ToggleWindow', 'overview')
            assert 'false' in call('Heartbeat', '999').stdout
            assert 'true' in call('Heartbeat', '1').stdout
            assert call('OpenWindow', 'invalid-view', check=False).returncode != 0
            call('OpenWindow', 'settings')
            call('Detach')
            call('Quit')
            assert app.wait(timeout=10) == 0
            print('GNOME session bridge smoke test passed: state, protocol, actions, detach, shutdown')
        finally:
            if app.poll() is None:
                app.terminate()
                try:
                    app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait()
