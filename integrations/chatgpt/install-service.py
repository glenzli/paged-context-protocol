#!/usr/bin/env python3
"""Install the existing ChatGPT tunnel profile as a per-user macOS service."""

import argparse
import getpass
import json
import os
from pathlib import Path
import plistlib
import re
import socket
import stat
import subprocess
import sys
import tempfile
import time

LABEL = "com.glenzli.pcp-chatgpt-tunnel"


def private_write(path, data):
    """Replace one private regular file without exposing partial contents."""
    if path.is_symlink():
        raise ValueError(f"Refusing symlink: {path}")
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as output:
            os.fchmod(output.fileno(), 0o600)
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def check_private_key(path):
    info = path.lstat()
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
            or stat.S_IMODE(info.st_mode) & 0o077 or info.st_size == 0):
        raise ValueError("The runtime key must be a nonempty, owner-only regular file.")


def service_profile(source, key_file, port):
    """Patch only known scalars in an init-generated YAML block profile.

    Preserve MCP commands and custom transport settings; reject unsupported or
    ambiguous layouts instead of interpreting arbitrary YAML without a parser.
    Config secrets are resolved before CLI overrides, so the file must be usable
    without the foreground terminal's environment.
    """
    lines = source.splitlines(keepends=True)
    for section, field, value in (
        ("control_plane", "api_key", f"file:{key_file}"),
        ("health", "listen_addr", f"127.0.0.1:{port}"),
        ("admin_ui", "open_browser", False),
    ):
        headers = [i for i, line in enumerate(lines)
                   if re.fullmatch(rf"{section}:[ \t]*(?:#.*)?", line.rstrip("\r\n"))]
        if len(headers) != 1:
            raise ValueError(f"Expected one init-generated {section} block.")
        start = headers[0] + 1
        end = next((i for i in range(start, len(lines))
                    if lines[i].strip() and not lines[i][0].isspace()
                    and not lines[i].startswith("#")), len(lines))
        fields = [i for i in range(start, end)
                  if re.match(rf"^[ \t]+{field}:", lines[i])]
        if len(fields) != 1 or not re.fullmatch(
            rf"  {field}:[ \t]*(?:\"(?:[^\"\\]|\\.)*\"|'[^']*'|[^\s#|>&*{{\[]+)[ \t]*(?:#.*)?",
            lines[fields[0]].rstrip("\r\n"),
        ):
            raise ValueError(f"Expected one scalar {section}.{field} in an init-generated profile.")
        lines[fields[0]] = f"  {field}: {json.dumps(value)}\n"
    return "".join(lines)


def service_plist(binary, profile, key_file, log_dir, port):
    return {
        "Label": LABEL,
        "ProgramArguments": [str(binary), "run", "--profile-file", str(profile),
                             "--control-plane.api-key", f"file:{key_file}",
                             "--health.listen-addr", f"127.0.0.1:{port}"],
        "RunAtLoad": True,
        "KeepAlive": True,
        "ThrottleInterval": 15,
        "ExitTimeOut": 15,
        "Umask": 0o077,
        "EnvironmentVariables": {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"},
        "StandardOutPath": str(log_dir / "stdout.log"),
        "StandardErrorPath": str(log_dir / "stderr.log"),
    }


def run_quiet(*args, check=True):
    return subprocess.run(args, check=check, stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL, timeout=15)


def wait_for_ready(binary, port, timeout=90):
    # A successful idle long-poll may not be recorded for more than 30 seconds.
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = run_quiet(str(binary), "health", "--port", str(port),
                           "--require-control-plane-poll", check=False)
        if result.returncode == 0:
            return True
        time.sleep(1)
    return False


def main():
    home = Path.home()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=4319)
    parser.add_argument("--profile", type=Path,
                        default=home / ".config/tunnel-client/pcp-chatgpt.yaml")
    parser.add_argument("--binary", type=Path,
                        default=home / "Applications/tunnel-client/tunnel-client")
    parser.add_argument("--replace-key", action="store_true",
                        help="Read a replacement key from the environment or a hidden prompt")
    args = parser.parse_args()
    if sys.platform != "darwin" or os.getuid() == 0:
        parser.error("Run as your normal macOS login user, without sudo.")
    if not 1024 <= args.port <= 65535:
        parser.error("Choose a port between 1024 and 65535.")
    binary, profile = args.binary.expanduser().resolve(), args.profile.expanduser().resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK) or not profile.is_file():
        parser.error("Install tunnel-client and initialize the pcp-chatgpt profile first.")

    domain = f"gui/{os.getuid()}"
    target = f"{domain}/{LABEL}"
    loaded = run_quiet("launchctl", "print", target, check=False).returncode == 0
    if not loaded:
        with socket.socket() as probe:
            try:
                probe.bind(("127.0.0.1", args.port))
            except OSError:
                parser.error(f"Port {args.port} is occupied; choose another --port.")

    service_dir = home / "Library/Application Support/PCP/chatgpt-tunnel"
    log_dir = service_dir / "logs"
    for directory in (service_dir, log_dir):
        if directory.is_symlink():
            parser.error("Service directories must not be symlinks.")
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        directory.chmod(0o700)
    key_file = service_dir / "runtime-api-key"
    if args.replace_key or not key_file.exists():
        key = os.environ.get("CONTROL_PLANE_API_KEY", "").strip()
        if not key:
            if not sys.stdin.isatty():
                parser.error("Run in a terminal to enter the runtime key; never paste it into chat.")
            key = getpass.getpass("OpenAI tunnel runtime API key (hidden): ").strip()
        if not key or any(char.isspace() for char in key):
            parser.error("The runtime API key must be nonempty and contain no whitespace.")
        private_write(key_file, key.encode())
        del key
    check_private_key(key_file)

    service_config = service_dir / "service.yaml"
    config_bytes = service_profile(profile.read_text(), key_file, args.port).encode()

    plist_dir = home / "Library/LaunchAgents"
    plist_dir.mkdir(parents=True, exist_ok=True)
    plist_path = plist_dir / f"{LABEL}.plist"
    payload = plistlib.dumps(service_plist(binary, service_config, key_file, log_dir, args.port))
    # Lint before disrupting a currently loaded service.
    with tempfile.NamedTemporaryFile(suffix=".plist") as candidate:
        candidate.write(payload)
        candidate.flush()
        run_quiet("plutil", "-lint", candidate.name)
    if loaded:
        run_quiet("launchctl", "bootout", target)
    private_write(service_config, config_bytes)
    private_write(plist_path, payload)
    run_quiet("launchctl", "enable", target)
    run_quiet("launchctl", "bootstrap", domain, str(plist_path))
    print(f"Installed {LABEL}; checking readiness and control-plane polling...", flush=True)
    if wait_for_ready(binary, args.port):
        print(f"Ready: http://127.0.0.1:{args.port}/ui")
        print("Login startup and process recovery are enabled.")
        print("You may now stop the old foreground tunnel with Ctrl-C.")
        return
    print(f"Service installed but not ready yet. Inspect {log_dir}", file=sys.stderr)
    raise SystemExit(1)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Service setup failed ({type(error).__name__}); check paths, permissions, and launchctl.",
              file=sys.stderr)
        raise SystemExit(1)
