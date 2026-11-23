"""Tests of the CLI with a fake daemon that speaks the control protocol. They also test the parts around it: config, state, and task XML."""

import json
import socketserver
import sys
import threading

import pytest
from typer.testing import CliRunner

from seltun import autostart
from seltun.cli import app
from seltun.config import Config
from seltun.control import request
from seltun.state import State
from seltun.support.errors import DaemonNotRunning, SeltunError

runner = CliRunner()


@pytest.fixture(autouse=True)
def _no_autostart_task(monkeypatch):
    """Do not query the real Task Scheduler in tests."""
    monkeypatch.setattr(autostart, "registered", lambda: None)


class FakeDaemon:
    """A fake daemon. It answers the control protocol from the state file, as the real daemon does at `reload`."""

    def __init__(self, state_path, error=None):
        self.state_path = state_path
        self.error = error
        self.commands: list[str] = []
        outer = self

        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                for line in self.rfile:
                    cmd = json.loads(line)["cmd"]
                    outer.commands.append(cmd)
                    enabled = State.load(outer.state_path).enabled
                    status = {
                        "pid": 1,
                        "enabled": enabled,
                        "active": enabled and outer.error is None,
                        "error": outer.error if enabled else None,
                        "processes": ["a.exe"],
                        "upstream": "127.0.0.1:1080",
                        "elevated": True,
                    }
                    self.wfile.write(
                        json.dumps({"ok": True, "status": status}).encode() + b"\n"
                    )

        self.server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    @property
    def port(self) -> int:
        return self.server.server_address[1]

    def close(self):
        self.server.shutdown()
        self.server.server_close()


@pytest.fixture
def env(tmp_path):
    state = tmp_path / "state.json"
    config = tmp_path / "config.toml"

    def write_config(port: int, names=("a.exe",)):
        listed = ", ".join(f'"{n}"' for n in names)
        config.write_text(
            f'[processes]\nnames = [{listed}]\n[daemon]\ncontrol = "127.0.0.1:{port}"\n',
            encoding="utf-8",
        )

    def invoke(*args):
        return runner.invoke(
            app, ["--config", str(config), "--state", str(state), *args]
        )

    return state, write_config, invoke


def test_on_persists_and_reloads_the_daemon(env):
    state, write_config, invoke = env
    daemon = FakeDaemon(state)
    try:
        write_config(daemon.port)
        result = invoke("switch", "on")
        assert result.exit_code == 0, result.output
        assert State.load(state).enabled is True
        assert daemon.commands == ["reload"]
        result = invoke("switch", "off")
        assert result.exit_code == 0 and State.load(state).enabled is False
    finally:
        daemon.close()


def test_status_shows_the_autostart_task_even_without_a_daemon(env, monkeypatch):
    from seltun import autostart

    _, write_config, invoke = env
    write_config(1)
    monkeypatch.setattr(autostart, "registered", lambda: None)
    assert "autostart disabled" in invoke("status").output
    monkeypatch.setattr(autostart, "registered", lambda: "seltund.exe --config x")
    assert "autostart enabled: seltund.exe --config x" in invoke("status").output
    out = json.loads(invoke("status", "--json").output)
    assert out["daemon"] is None and out["autostart"] == "seltund.exe --config x"


def test_switch_survives_without_a_daemon(env):
    state, write_config, invoke = env
    write_config(1)  # No program listens on port 1
    result = invoke("switch", "on")
    assert result.exit_code == 0, result.output
    assert "not running" in result.output
    assert State.load(state).enabled is True
    assert "switch    on" in invoke("status").output


def test_on_reports_a_daemon_side_problem_but_keeps_the_switch(env):
    state, write_config, invoke = env
    daemon = FakeDaemon(state, error="needs an elevated process")
    try:
        write_config(daemon.port)
        result = invoke("switch", "on")
        assert result.exit_code == 1
        assert "elevated" in result.output
        assert State.load(state).enabled is True
    finally:
        daemon.close()


def test_status_json(env):
    state, write_config, invoke = env
    daemon = FakeDaemon(state)
    try:
        write_config(daemon.port)
        State(enabled=True).save(state)
        out = json.loads(invoke("status", "--json").output)
        assert out["state"] == {"enabled": True}
        assert out["daemon"]["active"] is True
    finally:
        daemon.close()


def test_request_without_daemon_says_so():
    with pytest.raises(DaemonNotRunning):
        request(("127.0.0.1", 1), "ping", timeout=1)


def test_files_with_a_byte_order_mark_are_accepted(tmp_path):
    (tmp_path / "s.json").write_text('{"enabled": true}', encoding="utf-8-sig")
    assert State.load(tmp_path / "s.json").enabled is True
    (tmp_path / "c.toml").write_text(
        '[processes]\nnames = ["a.exe"]\n', encoding="utf-8-sig"
    )
    assert Config.load(tmp_path / "c.toml").processes.names == ["a.exe"]


def test_state_defaults_to_off_and_round_trips(tmp_path):
    assert State.load(tmp_path / "none.json").enabled is False
    State(enabled=True).save(tmp_path / "s.json")
    assert State.load(tmp_path / "s.json").enabled is True


def test_config_defaults_and_kebab_keys(tmp_path):
    assert Config.load(tmp_path / "none.toml").upstream.address == "127.0.0.1:1080"
    p = tmp_path / "c.toml"
    p.write_text(
        '[upstream]\nprotocol = "http"\n[processes]\nnames = ["x.exe"]\n',
        encoding="utf-8",
    )
    cfg = Config.load(p)
    assert cfg.upstream.protocol == "http" and cfg.processes.names == ["x.exe"]


@pytest.mark.parametrize(
    "text",
    [
        "bare = 1\n",
        '[upstream]\nadress = "x"\n',
        '[upstream]\nprotocol = "http"\nudp = true\n',
        '[daemon]\ncontrol = "localhost"\n',
        '[tun]\naddress = "127.0.0.1"\n',
    ],
)
def test_config_rejects_mistakes(tmp_path, text):
    p = tmp_path / "c.toml"
    p.write_text(text, encoding="utf-8")
    with pytest.raises(SeltunError):
        Config.load(p)


def test_packaged_example_config_loads():
    from seltun.config import EXAMPLE_CONFIG

    assert Config.load(EXAMPLE_CONFIG).processes.names == []


def test_task_xml_is_elevated_logon_task_with_quoted_arguments():
    xml = autostart.task_xml(
        "PC\\me", [r"C:\Program Files\seltun\seltund.exe", "--config", r"C:\a b\c.toml"]
    )
    assert "<RunLevel>HighestAvailable</RunLevel>" in xml
    assert "<LogonTrigger>" in xml
    assert r"<Command>C:\Program Files\seltun\seltund.exe</Command>" in xml
    assert r'--config "C:\a b\c.toml"' in xml


def test_launchd_plist_runs_the_daemon_at_load_and_after_a_failure():
    import plistlib

    argv = ["/Users/me/seltund", "--config", "/Users/me/a b/c.toml"]
    definition = plistlib.loads(autostart.launchd_plist(argv))
    assert definition["Label"] == autostart.LAUNCHD_LABEL
    assert definition["ProgramArguments"] == argv
    assert definition["RunAtLoad"] is True
    assert definition["KeepAlive"] == {"SuccessfulExit": False}


def test_sudoers_rule_permits_only_the_start_of_the_daemon():
    rule = autostart.sudoers_rule("me")
    assert rule.startswith("me ALL=(root) NOPASSWD: /bin/launchctl kickstart system/")
    assert rule.count("/bin/launchctl") == 2 and rule.endswith("\n")
    assert "ALL=(ALL)" not in rule and rule.count(":") == 1


def test_init_creates_config_and_wintun_only_when_missing(env, tmp_path, monkeypatch):
    from seltun import daemon as daemon_
    from seltun.cli import ensure_wintun as _  # noqa: F401  (the name the command uses)

    _, write_config, invoke = env
    binary = tmp_path / "bin" / "seltund.exe"
    binary.parent.mkdir()
    binary.write_bytes(b"")
    monkeypatch.setattr(daemon_, "find_binary", lambda: binary)
    fetched = []

    def fake_fetch(directory, force=False):
        fetched.append(directory)
        (directory / "wintun.dll").write_bytes(b"dll")
        return directory / "wintun.dll"

    monkeypatch.setattr("seltun.cli.ensure_wintun", fake_fetch)
    monkeypatch.setattr("seltun.cli.sys.platform", "win32")
    config = tmp_path / "config.toml"
    assert not config.exists()
    result = invoke("init")
    assert result.exit_code == 0, result.output
    # Only Windows needs wintun.dll.
    expected = [binary.parent] if sys.platform == "win32" else []
    assert config.exists() and fetched == expected

    config.write_text("# mine", encoding="utf-8")
    result = invoke("init")
    assert result.exit_code == 0 and "is already there" in result.output
    assert config.read_text(encoding="utf-8") == "# mine"
    assert fetched == expected


def test_version_option(env):
    from seltun import __version__

    _, _, invoke = env
    result = invoke("--version")
    assert result.exit_code == 0
    assert result.output.strip() == f"seltun {__version__}"
    assert __version__ != "0.0.0+unknown"
