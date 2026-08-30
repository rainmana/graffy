#!/usr/bin/env python3
"""Minimal MCP stdio server — a committed test fixture for graffy-mcp.

Speaks genuine MCP JSON-RPC over stdio (newline-delimited), implementing the
smallest honest surface: initialize, tools/list (one read-only `echo` tool
with annotations), tools/call, ping. Exists so the full rmcp client
round-trip runs hermetically in CI — no network, no npm.
"""
import json
import sys


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = msg.get("method")
        mid = msg.get("id")

        if method == "initialize":
            requested = (msg.get("params") or {}).get("protocolVersion", "2025-06-18")
            send({
                "jsonrpc": "2.0",
                "id": mid,
                "result": {
                    "protocolVersion": requested,
                    "capabilities": {"tools": {}, "prompts": {}},
                    "serverInfo": {"name": "graffy-fixture", "version": "0.1.0"},
                },
            })
        elif method == "notifications/initialized":
            pass  # notification: no response
        elif method == "tools/list":
            send({
                "jsonrpc": "2.0",
                "id": mid,
                "result": {
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Echo a message back (fixture).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"message": {"type": "string"}},
                            },
                            "annotations": {
                                "readOnlyHint": True,
                                "destructiveHint": False,
                            },
                        }
                    ]
                },
            })
        elif method == "prompts/list":
            send({
                "jsonrpc": "2.0",
                "id": mid,
                "result": {
                    "prompts": [
                        {
                            "name": "usage",
                            "description": "How to use this server well.",
                        }
                    ]
                },
            })
        elif method == "prompts/get":
            send({
                "jsonrpc": "2.0",
                "id": mid,
                "result": {
                    "description": "Fixture usage knowledge.",
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": "Fixture usage: the echo tool expects "
                                        "{\"message\": string}; keep messages short.",
                            },
                        }
                    ],
                },
            })
        elif method == "tools/call":
            params = msg.get("params") or {}
            args = params.get("arguments") or {}
            text = "fixture-echo: " + str(args.get("message", "(no message)"))
            send({
                "jsonrpc": "2.0",
                "id": mid,
                "result": {
                    "content": [{"type": "text", "text": text}],
                    "isError": False,
                },
            })
        elif method == "ping":
            send({"jsonrpc": "2.0", "id": mid, "result": {}})
        elif mid is not None:
            send({
                "jsonrpc": "2.0",
                "id": mid,
                "error": {"code": -32601, "message": f"method not found: {method}"},
            })


if __name__ == "__main__":
    main()
