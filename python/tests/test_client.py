"""ACC-CLIENT-183 and ACC-CLIENT-184, against the installed wheel: a real btop-web on a free
loopback port (standalone and agent) and a local fake stream for what a real server cannot show."""

import hashlib
import http.server
import importlib.metadata
import json
import os
import re
import socket
import ssl
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from btop_web_client import ClientError, frames

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "crates/btop-web/tests/fixtures"
BINARY = Path(os.environ.get("BTOP_WEB_BIN", ROOT / "target/release/btop-web"))
CLI = str(Path(sys.executable).with_name("btop-web-client"))
SKILL = "share/jupyter/agents/skills/btop-web-client/SKILL.md"
UPDATE_MS = 100
KEYS = {
    "events",
    "update_ms",
    "window_ms",
    "slow_ms",
    "slow_seq",
    "container",
    "cpu",
    "mem",
    "net",
    "proc",
    "gpu",
    "plots",
}
# crates/btop-web/tests/fixtures/proc/meminfo: MemTotal in kB.
FIXTURE_RAM_TOTAL = 527649328 * 1024


def cli(*args):
    return subprocess.run([CLI, *args], capture_output=True, text=True, timeout=30)


def start_server(xdg, *args):
    """Run the binary on the port the kernel gives it; returns the process and the port."""
    log = xdg / "server.log"
    env = os.environ | {
        "BTOP_WEB_LISTEN": "127.0.0.1:0",
        "BTOP_WEB_PROC_ROOT": str(FIXTURES / "proc"),
        "BTOP_WEB_UPDATE_MS": str(UPDATE_MS),
        "XDG_CONFIG_HOME": str(xdg),
        "RUST_LOG": "info",
    }
    with log.open("w") as output:
        server = subprocess.Popen(
            [BINARY, *args], env=env, stdout=output, stderr=subprocess.STDOUT
        )
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline and server.poll() is None:
        for line in log.read_text().splitlines():
            if "btop-web listening" in line:
                return server, re.search(r"127\.0\.0\.1:(\d+)", line)[1]
        time.sleep(0.01)
    server.kill()
    raise AssertionError(f"the server did not listen: {log.read_text()}")


@pytest.fixture
def standalone(tmp_path):
    server, port = start_server(tmp_path)
    yield f"http://127.0.0.1:{port}"
    server.terminate()
    server.wait(10)


@pytest.fixture
def agent(tmp_path):
    """The agent's URL, key file and certificate fingerprint."""
    server, port = start_server(tmp_path, "--agent")
    pem = (tmp_path / "btop-web/agent.crt").read_text()
    digest = hashlib.sha256(ssl.PEM_cert_to_DER_cert(pem)).hexdigest()
    yield f"https://127.0.0.1:{port}", str(tmp_path / "btop-web/access-key"), f"sha256:{digest}"
    server.terminate()
    server.wait(10)


def assert_is_fixture_frame(frame):
    assert set(frame) == KEYS
    assert frame["events"] == ["cpu", "mem", "net", "proc", "gpu"]
    assert frame["update_ms"] == UPDATE_MS
    assert frame["mem"]["ram_total"] == FIXTURE_RAM_TOTAL
    assert "Threadripper" in frame["cpu"]["name"]
    plots = frame["plots"]
    assert [len(plots[name]) for name in ("cpu", "mem", "net", "proc")] == [1, 1, 2, 1]
    assert len(plots["gpu"]) == len(frame["gpu"]["devices"])


def test_a_real_stream_yields_one_parsed_frame_per_line(standalone):
    stream = frames(standalone)
    for _ in range(3):
        assert_is_fixture_frame(next(stream))
    stream.close()


def test_the_cli_prints_one_frame(standalone):
    result = cli(standalone)
    assert result.returncode == 0, result.stderr
    lines = result.stdout.splitlines()
    assert len(lines) == 1
    assert_is_fixture_frame(json.loads(lines[0]))


def test_the_cli_follows_the_stream_until_the_reader_leaves(standalone):
    result = subprocess.run(
        ["bash", "-c", f"set -o pipefail; '{CLI}' '{standalone}' --follow | head -n 3"],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, result.stderr
    assert result.stderr == ""
    lines = result.stdout.splitlines()
    assert len(lines) == 3
    for line in lines:
        assert_is_fixture_frame(json.loads(line))


def test_an_agent_answers_the_key_and_the_pin(agent):
    url, key_file, pin = agent
    assert_is_fixture_frame(next(frames(url, key_file, pin)))
    result = cli(url, "--key-file", key_file, "--pin", pin)
    assert result.returncode == 0, result.stderr
    assert_is_fixture_frame(json.loads(result.stdout))


def test_an_agent_refuses_a_wrong_or_missing_key(agent, tmp_path):
    url, _, pin = agent
    wrong = tmp_path / "wrong-key"
    wrong.write_text("not-the-key\n")
    for args in (["--key-file", str(wrong)], []):
        result = cli(url, "--pin", pin, *args)
        assert result.returncode == 1
        assert "answered 401" in result.stderr
        assert "not-the-key" not in result.stderr
        assert result.stdout == ""


def test_an_agent_is_refused_on_a_wrong_or_missing_pin(agent):
    url, key_file, pin = agent
    other = "sha256:" + "0" * 64
    result = cli(url, "--key-file", key_file, "--pin", other)
    assert result.returncode == 1
    assert pin in result.stderr and other in result.stderr
    result = cli(url, "--key-file", key_file)
    assert result.returncode == 1
    assert pin in result.stderr
    assert result.stdout == ""


def test_plain_http_to_an_agent_names_https(agent):
    url, _, _ = agent
    result = cli(url.replace("https://", "http://"))
    assert result.returncode == 1
    assert "an agent takes https://" in result.stderr


class FakeStream:
    """A local stream that records each request's Authorization header and writes `chunks`."""

    def __init__(self, chunks, tls=False):
        fake = self
        self.authorizations = []

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_GET(self):
                fake.authorizations.append(self.headers.get("Authorization"))
                self.send_response(200)
                self.send_header("Transfer-Encoding", "chunked")
                self.send_header("Connection", "close")
                self.end_headers()
                for chunk in chunks:
                    self.wfile.write(b"%x\r\n%s\r\n" % (len(chunk), chunk))
                self.wfile.write(b"0\r\n\r\n")

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        if tls:
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(FIXTURES / "tls/agent.crt", FIXTURES / "tls/agent.key")
            self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        port = self.server.server_address[1]
        self.url = f"{'https' if tls else 'http'}://127.0.0.1:{port}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def test_the_cli_prints_one_json_line_per_fake_frame_across_chunks():
    documents = [{"seq": 1}, {"seq": 2, "name": "é"}, {"seq": 3}]
    second = json.dumps(documents[1]).encode() + b"\n"
    fake = FakeStream(
        [json.dumps(documents[0]).encode() + b"\n", second[:4], second[4:], b'{"seq": 3}\n']
    )
    result = cli(fake.url, "--follow")
    fake.close()
    assert [json.loads(line) for line in result.stdout.splitlines()] == documents
    assert result.returncode == 1
    assert "the stream closed" in result.stderr


def test_the_pin_is_checked_before_the_key_is_sent(tmp_path):
    key_file = tmp_path / "key"
    key_file.write_text("fake-secret\n")
    pem = (FIXTURES / "tls/agent.crt").read_text()
    pin = f"sha256:{hashlib.sha256(ssl.PEM_cert_to_DER_cert(pem)).hexdigest()}"
    fake = FakeStream([b'{"seq": 1}\n'], tls=True)
    with pytest.raises(ClientError, match="refused"):
        next(frames(fake.url, key_file, "sha256:" + "0" * 64))
    assert fake.authorizations == []
    assert next(frames(fake.url, key_file, pin)) == {"seq": 1}
    assert fake.authorizations == ["Bearer fake-secret"]
    fake.close()


def test_a_key_or_a_pin_never_goes_over_plain_http(tmp_path):
    key_file = tmp_path / "key"
    key_file.write_text("fake-secret\n")
    fake = FakeStream([b'{"seq": 1}\n'])
    for args in ((key_file, None), (None, "sha256:" + "0" * 64)):
        with pytest.raises(ClientError, match="https://"):
            next(frames(fake.url, *args))
    assert fake.authorizations == []
    fake.close()


def test_a_two_line_key_file_is_refused_without_printing_the_key(tmp_path):
    key_file = tmp_path / "key"
    key_file.write_text("fake\nsecret\n")
    result = cli("https://127.0.0.1:9", "--key-file", str(key_file), "--pin", "sha256:0")
    assert result.returncode == 1
    assert "one line" in result.stderr
    assert "secret" not in result.stderr


def test_a_silent_stream_ends_at_the_timeout():
    with socket.create_server(("127.0.0.1", 0)) as silent:
        started = time.monotonic()
        result = cli(f"http://127.0.0.1:{silent.getsockname()[1]}", "--timeout", "0.2")
    assert result.returncode == 1
    assert "timed out" in result.stderr
    assert time.monotonic() - started < 10


def test_the_help_carries_what_an_agent_needs():
    result = cli("--help")
    assert result.returncode == 0
    for text in ("--follow", "--key-file", "--pin", "--timeout", "exit codes:", "examples:"):
        assert text in result.stdout


def test_the_wheel_ships_the_skill_outside_the_package():
    repository = (ROOT / ".agents/skills/btop-web-client/SKILL.md").read_text()
    assert (Path(sys.prefix) / SKILL).read_text() == repository
    assert len(repository.splitlines()) < 30
    assert "name: btop-web-client\n" in repository
    assert "`btop-web-client --help`" in repository
    files = [path.as_posix() for path in importlib.metadata.distribution("btop-web-client").files]
    assert any(path.endswith(SKILL) for path in files)
    assert not any(path.startswith("btop_web_client/") and path.endswith(".md") for path in files)
