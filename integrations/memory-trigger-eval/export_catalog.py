#!/usr/bin/env python3
"""Export an MCP stdio server's real initialize/tools-list metadata without tool calls."""
import argparse
import json
from pathlib import Path
import selectors
import subprocess
import tempfile
import time


def export_catalog(command, timeout=20):
    errors = tempfile.TemporaryFile()
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=errors, text=False, bufsize=0)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    pending = b""

    def send(message):
        process.stdin.write((json.dumps(message) + "\n").encode())
        process.stdin.flush()

    def response(request_id):
        nonlocal pending
        deadline = time.monotonic() + timeout
        while True:
            while b"\n" in pending:
                line, pending = pending.split(b"\n", 1)
                if not line.strip():
                    continue
                message = json.loads(line)
                if message.get("id") != request_id:
                    continue
                if "error" in message:
                    raise ValueError(f"MCP request {request_id} failed: {message['error']}")
                return message["result"]
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not selector.select(remaining):
                raise ValueError(f"MCP request {request_id} timed out")
            chunk = process.stdout.read(65536)
            if not chunk:
                errors.seek(0)
                detail = errors.read(2000).decode(errors="replace").strip()
                raise ValueError(f"MCP server exited before request {request_id} completed: {detail}")
            pending += chunk

    try:
        send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "pcp-catalog-evaluation", "version": "1"},
        }})
        initialized = response(1)
        send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        tools, cursor, request_id = [], None, 2
        while True:
            send({"jsonrpc": "2.0", "id": request_id, "method": "tools/list",
                  "params": {"cursor": cursor} if cursor else {}})
            result = response(request_id)
            tools.extend(result["tools"])
            cursor = result.get("nextCursor")
            if not cursor:
                break
            request_id += 1
        return {"instructions": initialized.get("instructions", ""), "tools": tools,
                "serverInfo": initialized.get("serverInfo"),
                "protocolVersion": initialized.get("protocolVersion")}
    finally:
        selector.close()
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        process.stdout.close()
        errors.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER,
                        help="MCP server command after --; inherits the selected connection environment")
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("An MCP stdio server command is required after --")
    try:
        catalog = export_catalog(command)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(catalog, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"tools": len(catalog["tools"]), "output": str(args.output)}))


if __name__ == "__main__":
    main()
