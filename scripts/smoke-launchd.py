#!/usr/bin/env python3
"""macOS-only native service lifecycle check with disposable data and mock traffic.

Build TGV, then run this manually. Loads one uniquely named temporary GUI agent,
checks failure restart and graceful shutdown, then removes the job and fixtures.
Does not install a login item or use provider credentials.
"""
import http.server
import json
import os
import pathlib
import platform
import re
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
import uuid


def main():
    if platform.system() != 'Darwin':
        raise SystemExit('This check requires macOS and a logged-in GUI user.')
    binary = pathlib.Path(__file__).resolve().parents[1] / 'target/debug/tgv'
    label = 'dev.tgv.smoke.' + uuid.uuid4().hex
    domain = f'gui/{os.getuid()}'
    target = f'{domain}/{label}'
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    class Upstream(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            self.rfile.read(int(self.headers['content-length']))
            payload = json.dumps({'id': 'msg_fixture', 'type': 'message', 'role': 'assistant', 'model': 'fixture', 'content': [], 'stop_reason': 'end_turn', 'usage': {'input_tokens': 1, 'output_tokens': 1}}).encode()
            self.send_response(200)
            self.send_header('content-type', 'application/json')
            self.send_header('content-length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    upstream = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    thread = threading.Thread(target=upstream.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='tgv-launchd-') as directory:
            root = pathlib.Path(directory)
            # Exercise XML escaping and independence from the service manager's cwd.
            state = root / 'state & $dollar %percent'
            logs = root / 'logs'
            logs.mkdir(mode=0o700)
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            config = root / 'config & settings.toml'
            config.write_text(f'listen = "127.0.0.1:{port}"\nanthropic_base_url = "http://127.0.0.1:{upstream.server_port}"\n')
            plist = root / f'{label}.plist'
            subprocess.run([str(binary), '--config', str(config), '--data-dir', str(state), 'service', 'render', '--manager', 'launchd', '--binary', str(binary), '--label', label, '--log-dir', str(logs), '--output', str(plist)], check=True)
            subprocess.run(['plutil', '-lint', str(plist)], check=True)

            def pid():
                result = subprocess.run(['launchctl', 'print', target], capture_output=True, text=True)
                match = re.search(r'^\s*pid = (\d+)\s*$', result.stdout, re.MULTILINE)
                return int(match.group(1)) if match else None

            def healthy(old_pid=None):
                deadline = time.monotonic() + 40
                while time.monotonic() < deadline:
                    current = pid()
                    if current and current != old_pid:
                        try:
                            with opener.open(f'http://127.0.0.1:{port}/health', timeout=1) as response:
                                json.load(response)
                                return current
                        except OSError:
                            pass
                    time.sleep(0.2)
                raise RuntimeError('Temporary launchd service did not become healthy')

            try:
                subprocess.run(['launchctl', 'bootstrap', domain, str(plist)], check=True)
                first_pid = healthy()
                os.kill(first_pid, signal.SIGKILL)
                restarted_pid = healthy(first_pid)
                request = urllib.request.Request(f'http://127.0.0.1:{port}/anthropic/v1/messages', data=b'{"model":"fixture","messages":[]}', headers={'content-type': 'application/json'})
                with opener.open(request, timeout=5) as response:
                    assert json.load(response)['stop_reason'] == 'end_turn'
                subprocess.run(['launchctl', 'bootout', target], check=True)
                deadline = time.monotonic() + 30
                while True:
                    try:
                        os.kill(restarted_pid, 0)
                    except ProcessLookupError:
                        break
                    if time.monotonic() >= deadline:
                        raise RuntimeError('Temporary daemon did not exit after bootout')
                    time.sleep(0.1)
                records = json.loads(subprocess.check_output([str(binary), '--data-dir', str(state), 'history'], text=True))
                assert len(records) == 1
                assert records[0]['outcome'] == 'completed'
                assert records[0]['state'] == 'finished'
                print('PASS: launchd startup, failure restart, mock forwarding, and graceful capture drain.')
            finally:
                # Bootstrap can fail after registering a job. Always remove our
                # unique label, including when an assertion fails.
                subprocess.run(['launchctl', 'bootout', target], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    finally:
        upstream.shutdown()
        upstream.server_close()
        thread.join(timeout=5)


if __name__ == '__main__':
    main()
