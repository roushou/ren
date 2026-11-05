#!/usr/bin/env python3
"""Linux user-systemd lifecycle check with temporary state and mock providers.

Build TGV, then run this as a regular user with a running systemd user manager.
Creates only a uniquely named runtime unit; never enables it or changes lingering,
an existing TGV service or provider credentials. Optional --https-port briefly
adds a Tailscale Serve route on an unused port and removes only that route.
"""
import argparse
import http.server
import json
import os
import pathlib
import platform
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
import uuid


def run(*args, **kwargs):
    return subprocess.run(args, check=True, timeout=45, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[1] / 'target/debug/tgv')
    parser.add_argument('--https-port', type=int, help='also test through Tailscale Serve on an unused HTTPS port')
    parser.add_argument('--sudo-tailscale', action='store_true', help='use sudo -n only for Serve configuration')
    args = parser.parse_args()
    if args.sudo_tailscale and not args.https_port:
        parser.error('--sudo-tailscale requires --https-port')
    serve_command = ['sudo', '-n', 'tailscale'] if args.sudo_tailscale else ['tailscale']
    if args.https_port is not None and not 1 <= args.https_port <= 65535:
        parser.error('--https-port must be between 1 and 65535')
    if platform.system() != 'Linux' or os.getuid() == 0:
        parser.error('requires Linux and a non-root systemd user manager')
    binary = args.binary.resolve(strict=True)
    run('systemctl', '--user', 'show-environment', stdout=subprocess.DEVNULL)
    unit = 'tgv-smoke-' + uuid.uuid4().hex + '.service'
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    observed = []
    if args.https_port:
        serve = json.loads(subprocess.check_output(['tailscale', 'serve', 'status', '--json'], text=True, timeout=10) or '{}')
        if str(args.https_port) in (serve or {}).get('TCP', {}):
            parser.error('the requested Tailscale Serve port is already in use')
        status = json.loads(subprocess.check_output(['tailscale', 'status', '--json'], text=True, timeout=10))
        if status.get('BackendState') != 'Running':
            parser.error('Tailscale must already be connected')
        dns_name = status['Self']['DNSName'].rstrip('.')
        if not dns_name:
            parser.error('Tailscale node has no DNS name')

    class Upstream(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = self.rfile.read(int(self.headers['content-length']))
            proxy_headers = [name.lower() for name in self.headers if name.lower().startswith(('tailscale-', 'x-forwarded-')) or name.lower() == 'forwarded']
            observed.append((self.path, body, self.headers.get('authorization'), proxy_headers))
            payload = (b'data: {"type":"message_start","message":{"model":"fixture"}}\n\n'
                       b'data: {"type":"message_stop"}\n\n' if self.path == '/v1/messages' else
                       b'data: {"type":"response.completed","response":{"status":"completed","model":"fixture","usage":{"input_tokens":1,"output_tokens":1}}}\n\n')
            self.send_response(200)
            self.send_header('content-type', 'text/event-stream')
            self.send_header('content-length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    upstream = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
    thread = threading.Thread(target=upstream.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='tgv-systemd-') as directory:
            root = pathlib.Path(directory)
            # Check literal dollars/specifiers and cwd-independent configuration.
            state = root / 'state $USER %n'
            config = root / 'config & settings.toml'
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            base = f'http://127.0.0.1:{port}'
            config.write_text(f'listen = "127.0.0.1:{port}"\n' + ''.join(
                f'{provider}_base_url = "http://127.0.0.1:{upstream.server_port}"\n'
                for provider in ('anthropic', 'codex', 'openai')))
            definition = root / unit
            run(str(binary), '--config', str(config), '--data-dir', str(state), 'service', 'render', '--manager', 'systemd', '--binary', str(binary), '--output', str(definition))
            run('systemd-analyze', '--user', 'verify', str(definition))

            def pid():
                result = run('systemctl', '--user', 'show', unit, '--property=MainPID', '--value', capture_output=True, text=True)
                return int(result.stdout.strip() or '0')

            def healthy(old_pid=0):
                deadline = time.monotonic() + 40
                while time.monotonic() < deadline:
                    current = pid()
                    if current and current != old_pid:
                        try:
                            with opener.open(base + '/health', timeout=1) as response:
                                status = json.load(response)
                                assert status['storage_errors'] == 0
                                return current
                        except OSError:
                            pass
                    time.sleep(0.2)
                raise RuntimeError('temporary systemd service did not become healthy')

            linked = False
            serving = False
            try:
                run('systemctl', '--user', 'link', '--runtime', str(definition))
                linked = True
                run('systemctl', '--user', 'daemon-reload')
                run('systemctl', '--user', 'start', unit)
                first_pid = healthy()
                run('systemctl', '--user', 'kill', '--signal=SIGKILL', '--kill-whom=main', unit)
                healthy(first_pid)
                request_base = base
                if args.https_port:
                    run(*serve_command, 'serve', '--bg', f'--https={args.https_port}', base)
                    serving = True
                    request_base = f'https://{dns_name}:{args.https_port}'
                original = b'{ "model":"fixture", "unknown_future_field":true, "stream":true }'
                for route in ('/anthropic/v1/messages', '/codex/responses', '/openai/v1/responses'):
                    request = urllib.request.Request(request_base + route, data=original, headers={'authorization': 'Bearer LOCAL_FIXTURE_ONLY'})
                    with opener.open(request, timeout=5) as response:
                        assert b'fixture' in response.read()
                run('systemctl', '--user', 'stop', unit)
                assert pid() == 0
                records = json.loads(subprocess.check_output([str(binary), '--data-dir', str(state), 'history'], text=True, timeout=10))
                assert len(records) == 3
                assert {r['provider'] for r in records} == {'anthropic', 'codex', 'openai'}
                assert all(r['outcome'] == 'completed' and r['state'] == 'finished' and r['capture_complete'] for r in records)
                assert [path for path, _, _, _ in observed] == ['/v1/messages', '/responses', '/v1/responses']
                assert all(body == original and auth == 'Bearer LOCAL_FIXTURE_ONLY' and not proxy_headers for _, body, auth, proxy_headers in observed), [(path, body == original, auth == 'Bearer LOCAL_FIXTURE_ONLY', names) for path, body, auth, names in observed]
                assert state.stat().st_mode & 0o077 == 0
                assert (state / 'tgv.sqlite3').stat().st_mode & 0o077 == 0
                for name in ('tgv.sqlite3', 'blobs.sqlite3'):
                    assert (state / name).stat().st_mode & 0o077 == 0
                    assert b'LOCAL_FIXTURE_ONLY' not in (state / name).read_bytes()
                run('systemctl', '--user', 'start', unit)
                healthy()
                with opener.open(base + '/api/calls', timeout=5) as response:
                    page = json.load(response)
                    assert page['api_version'] == 1 and len(page['calls']) == 3
                run('systemctl', '--user', 'stop', unit)
                print('PASS: systemd startup, crash restart, all provider routes, graceful capture drain, private storage, and persistent history.')
                if serving:
                    print('PASS: Tailscale HTTPS forwarding preserves credentials and strips proxy identity headers.')
            finally:
                try:
                    if serving:
                        run(*serve_command, 'serve', f'--https={args.https_port}', 'off')
                finally:
                    if linked:
                        run('systemctl', '--user', 'stop', unit)
                        run('systemctl', '--user', 'disable', '--runtime', unit)
                        run('systemctl', '--user', 'daemon-reload')
    finally:
        upstream.shutdown()
        upstream.server_close()
        thread.join(timeout=5)


if __name__ == '__main__':
    main()
