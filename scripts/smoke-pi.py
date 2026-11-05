#!/usr/bin/env python3
"""Installed Pi + TGV launcher: Anthropic, Codex, and public OpenAI OAuth-shaped mock round trips.

Build TGV first, then run: python3 scripts/smoke-pi.py
Uses the installed Pi CLI, temporary configuration/storage, and loopback servers.
"""
import base64
import http.server
import json
import os
import pathlib
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
import urllib.parse
import uuid


def events(index, result, model, tool_id, tool_name):
    message = {"type": "message", "id": f"msg_fixture_{index}", "role": "assistant", "model": model, "content": [], "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 1, "output_tokens": 0}}
    block = {"type": "text", "text": ""} if result else {"type": "tool_use", "id": tool_id, "name": tool_name, "input": {}}
    delta = {"type": "text_delta", "text": "TGV_OK"} if result else {"type": "input_json_delta", "partial_json": json.dumps({"path": "probe.txt"})}
    payloads = [
        {"type": "message_start", "message": message},
        {"type": "content_block_start", "index": 0, "content_block": block},
        {"type": "content_block_delta", "index": 0, "delta": delta},
        {"type": "content_block_stop", "index": 0},
        {"type": "message_delta", "delta": {"stop_reason": "end_turn" if result else "tool_use", "stop_sequence": None}, "usage": {"output_tokens": 1}},
        {"type": "message_stop"},
    ]
    return "".join(f"event: {p['type']}\ndata: {json.dumps(p)}\n\n" for p in payloads).encode()


def codex_events(index, result, model, tool_id, tool_name):
    response_id = f"resp_fixture_{index}"
    item = ({"type": "message", "id": f"msg_{index}", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": "TGV_OK", "annotations": []}]}
            if result else {"type": "function_call", "id": f"fc_{index}", "call_id": tool_id, "name": tool_name, "arguments": json.dumps({"path": "probe.txt"}), "status": "completed"})
    response = {"id": response_id, "status": "completed", "model": model, "output": [item], "usage": {"input_tokens": 12, "output_tokens": 5, "input_tokens_details": {"cached_tokens": 0}}}
    payloads = [
        {"type": "response.created", "response": {"id": response_id, "status": "in_progress"}},
        {"type": "response.output_item.added", "output_index": 0, "item": item},
        {"type": "response.output_item.done", "output_index": 0, "item": item},
        {"type": "response.completed", "response": response},
    ]
    return "".join(f"data: {json.dumps(event)}\n\n" for event in payloads).encode()



def scenario(provider, model):
    binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/tgv"
    expected = "TGV_" + uuid.uuid4().hex
    session_id = str(uuid.uuid4())
    observed = []
    failures = []
    credential = "sk-ant-oat01-LOCAL_FIXTURE_ONLY" if provider == "anthropic" else "fixture." + base64.urlsafe_b64encode(json.dumps({"https://api.openai.com/auth": {"chatgpt_account_id": "fixture-account"}}).encode()).decode().rstrip("=") + ".fixture"
    if provider == "openai":
        credential = "LOCAL_CHATGPT_DIRECT_ACCESS_FIXTURE"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    class Upstream(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                raw = self.rfile.read(int(self.headers["content-length"]))
                if self.headers.get("content-encoding") == "zstd":
                    raw = subprocess.check_output(["node", "-e", "let b=[];process.stdin.on('data',x=>b.push(x));process.stdin.on('end',()=>process.stdout.write(require('node:zlib').zstdDecompressSync(Buffer.concat(b))))"], input=raw)
                body = json.loads(raw)
                assert self.headers.get("authorization") == "Bearer " + credential, "bearer auth mismatch"
                assert not any(name.lower().startswith("x-tgv-pi-") for name in self.headers), "internal headers leaked"
                index = len(observed)
                tool_id = f"tool_fixture_{index // 2}"
                result = index % 2 == 1
                if provider == "anthropic":
                    assert urllib.parse.urlsplit(self.path).path == "/v1/messages", "incorrect Anthropic path"
                    assert "oauth" in self.headers.get("anthropic-beta", ""), "missing OAuth capability"
                    tool_name = next(t["name"] for t in body["tools"] if t["name"].lower() == "read")
                    results = [block for message in body["messages"] for block in message.get("content", []) if isinstance(block, dict) and block.get("type") == "tool_result" and block.get("tool_use_id") == tool_id]
                    payload = events(index, result, model, tool_id, tool_name)
                else:
                    expected_path = "/v1/responses" if provider == "openai" else "/responses"
                    assert urllib.parse.urlsplit(self.path).path == expected_path, "incorrect Responses path"
                    if provider == "openai":
                        assert body["store"] is False and body["stream"] is True
                        assert not any(key in body for key in ("max_output_tokens", "temperature", "prompt_cache_retention", "prompt_cache_options")), "ChatGPT-incompatible fields sent"
                        assert self.headers.get("session_id") == session_id
                        assert self.headers.get("user-agent", "").startswith("pi ("), "incorrect client identity"
                    else:
                        assert self.headers.get("originator") == "pi", "incorrect client identity"
                    tool_name = next(t["name"] for t in body["tools"] if t["name"] == "read")
                    results = [item for item in body["input"] if item.get("type") == "function_call_output" and item.get("call_id") == tool_id]
                    payload = codex_events(index, result, model, tool_id, tool_name)
                if result:
                    assert expected in json.dumps(results), "Pi did not send the read tool result"
                observed.append(result)
                self.send_response(200)
                self.send_header("content-type", "text/event-stream")
                self.send_header("content-length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
            except Exception as error:
                failures.append(type(error).__name__ + ": " + str(error))
                self.send_error(500)

    upstream = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    thread = threading.Thread(target=upstream.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="tgv-pi-") as directory:
            root = pathlib.Path(directory)
            (root / "probe.txt").write_text(expected)
            agent = root / "agent"
            agent.mkdir()
            auth = agent / "auth.json"
            auth.write_text(json.dumps({provider: {"type": "oauth", "access": credential, "refresh": "LOCAL_REFRESH_NEVER_USED", "clientId": "fixture-issued-client", "scopes": ["chatgpt.tokens.use.direct"], "expires": int(time.time() * 1000) + 3600000}}))
            initial_auth = auth.read_bytes()
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            config = root / "tgv.toml"
            config.write_text(f'listen = "127.0.0.1:{port}"\nanthropic_base_url = "http://127.0.0.1:{upstream.server_port}"\ncodex_base_url = "http://127.0.0.1:{upstream.server_port}"\nopenai_base_url = "http://127.0.0.1:{upstream.server_port}"\n')
            state = root / "state"
            daemon = subprocess.Popen([str(binary), "--config", str(config), "--data-dir", str(state), "serve"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                for _ in range(100):
                    try:
                        with opener.open(f"http://127.0.0.1:{port}/health", timeout=1):
                            break
                    except OSError:
                        if daemon.poll() is not None:
                            raise RuntimeError("TGV failed to start")
                        time.sleep(0.05)
                else:
                    raise RuntimeError("TGV did not become healthy")
                env = os.environ.copy()
                # All credentials and configuration are disposable. Startup
                # networking, discovered extensions, and provider proxies are off.
                for key in list(env):
                    if key.upper().endswith(("_API_KEY", "_AUTH_TOKEN", "_OAUTH_TOKEN", "_PROXY")):
                        env.pop(key)
                env.update(PI_CODING_AGENT_DIR=str(agent), XDG_CACHE_HOME=str(root / "cache"), PI_OFFLINE="1", PI_TELEMETRY="0", PI_CACHE_RETENTION="long")
                for _ in range(2):
                    result = subprocess.run([str(binary), "run", "pi", "--gateway", f"http://127.0.0.1:{port}", "--", "--offline", "--print", "--session-id", session_id, "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-themes", "--no-context-files", "--no-approve", "--tools", "read", "--model", f"{provider}/{model}", "--thinking", "off", "--mode", "json", "Read probe.txt using a tool, then reply TGV_OK."], cwd=root, env=env, capture_output=True, text=True, timeout=60)
                    if result.returncode != 0 or 'TGV_OK' not in result.stdout:
                        print(result.stderr.replace(credential, "[fixture credential]"))
                        print(result.stdout[-4000:].replace(credential, "[fixture credential]"))
                        raise RuntimeError(f"Pi failed for {provider}: {failures}")
                assert observed == [False, True, False, True], (observed, failures)
                assert auth.read_bytes() == initial_auth
            finally:
                daemon.send_signal(signal.SIGTERM)
                try:
                    daemon.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()
            records = json.loads(subprocess.check_output([str(binary), "--data-dir", str(state), "history"], text=True))
            assert len(records) == 4
            assert all(r["state"] == "finished" and r["outcome"] == "completed" for r in records)
            assert all(r["attribution"]["hint"]["source"] == "pi_extension_headers" for r in records)
            assert all(r["attribution"]["hint"]["session"] == session_id for r in records)
            assert len({r["session"] for r in records}) == 1
            assert len({r["turn"] for r in records}) == 2 and all(r["turn"] for r in records)
            assert sum(len(r["tool_names"]) for r in records) == 2
            for database in ("tgv.sqlite3", "blobs.sqlite3"):
                assert credential.encode() not in (state / database).read_bytes()
            print(f"PASS: {provider}: stored OAuth-shaped auth, SSE tool round trips, resumed session, 2 turns / 4 calls, local hints stripped.")
    finally:
        upstream.shutdown()
        upstream.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    scenario("anthropic", "claude-sonnet-4-5")
    scenario("openai-codex", "gpt-5.3-codex")
    scenario("openai", "gpt-5.4")
