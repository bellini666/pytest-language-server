#!/usr/bin/env python3
"""
Manual test script to verify the LSP server works correctly.
This simulates what an LSP client would do.
"""

import json
import subprocess
import sys


def send_message(proc, message):
    """Send a JSON-RPC message to the LSP server."""
    content = json.dumps(message)
    header = f"Content-Length: {len(content)}\r\n\r\n"
    proc.stdin.write(header.encode() + content.encode())
    proc.stdin.flush()


def read_message(proc):
    """Read a JSON-RPC message from the LSP server."""
    # Read headers
    headers = {}
    while True:
        line = proc.stdout.readline().decode().strip()
        if not line:
            break
        key, value = line.split(": ", 1)
        headers[key] = value

    # Read content
    content_length = int(headers["Content-Length"])
    content = proc.stdout.read(content_length).decode()
    return json.loads(content)


def test_lsp():
    # Start the LSP server
    proc = subprocess.Popen(
        ["./target/release/pytest-lsp"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    # Initialize
    init_msg = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "rootUri": "file:///Users/bellini/dev/pytest-lsp/test_project",
            "capabilities": {},
        },
    }

    send_message(proc, init_msg)
    response = read_message(proc)
    print(f"Initialize response: {json.dumps(response, indent=2)}")

    # Send initialized notification
    send_message(proc, {"jsonrpc": "2.0", "method": "initialized", "params": {}})

    # Open test file
    with open("test_project/test_example.py") as f:
        test_content = f.read()

    send_message(
        proc,
        {
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///Users/bellini/dev/pytest-lsp/test_project/test_example.py",
                    "languageId": "python",
                    "version": 1,
                    "text": test_content,
                }
            },
        },
    )

    # Request go-to-definition for sample_fixture on line 1 (0-indexed)
    send_message(
        proc,
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/definition",
            "params": {
                "textDocument": {
                    "uri": "file:///Users/bellini/dev/pytest-lsp/test_project/test_example.py"
                },
                "position": {"line": 0, "character": 20},
            },
        },
    )

    response = read_message(proc)
    print(f"\nGo-to-definition response: {json.dumps(response, indent=2)}")

    # Shutdown
    send_message(
        proc, {"jsonrpc": "2.0", "id": 3, "method": "shutdown", "params": None}
    )

    response = read_message(proc)
    print(f"\nShutdown response: {json.dumps(response, indent=2)}")

    proc.terminate()
    proc.wait()


if __name__ == "__main__":
    test_lsp()
