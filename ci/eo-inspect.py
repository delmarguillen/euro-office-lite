#!/usr/bin/env python3
"""
eo-inspect: remote JS console against the Euro-Office Lite webview on Linux.

Speaks the WebKit Remote Inspector Protocol over WebSocket with no third party
dependencies (stdlib only). It is used to check UI state as TEXT instead of
screenshots, which is what makes DOM assertions cheap enough for CI.

The app has to be launched with both variables:
    WEBKIT_INSPECTOR_SERVER=127.0.0.1:2999 \
    WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:3000 euro-office-lite

The first one opens the socket but does not speak HTTP (port 2999 accepts the
connection and answers nothing, which is normal); the second one serves the
target list on port 3000. Both listen on loopback only.

Usage:
    eo-inspect.py 'document.title'
    eo-inspect.py -f query.js
    echo 'document.title' | eo-inspect.py

Host and port default to EO_INSPECT_HOST / EO_INSPECT_PORT when those are set,
so a caller can move the ports without rewriting every invocation.

Protocol notes (found by hand against the real app):
  - Commands cannot be sent on their own: the backend answers
    "'Runtime' domain was not found". They have to be wrapped in
    Target.sendMessageToTarget with the targetId that arrives in the
    Target.targetCreated event on connect.
  - Answers come back inside Target.dispatchMessageFromTarget, with the real
    JSON as a STRING in params.message.

Output: the serialized value on stdout. Exit 0 = OK, 1 = error (JS exception,
no target, app not listening).
"""
import argparse
import base64
import json
import os
import socket
import struct
import sys

DEFAULT_HOST = os.environ.get("EO_INSPECT_HOST", "127.0.0.1")
DEFAULT_PORT = int(os.environ.get("EO_INSPECT_PORT", "3000"))
DEFAULT_PATH = "/socket/1/1/WebPage"


class Inspector:
    def __init__(self, host, port, path, timeout):
        self.timeout = timeout
        self.sock = socket.create_connection((host, port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            (
                "GET %s HTTP/1.1\r\n"
                "Host: %s:%d\r\n"
                "Upgrade: websocket\r\n"
                "Connection: Upgrade\r\n"
                "Sec-WebSocket-Key: %s\r\n"
                "Sec-WebSocket-Version: 13\r\n\r\n" % (path, host, port, key)
            ).encode()
        )
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("the server closed during the handshake")
            buf += chunk
        status = buf.split(b"\r\n")[0].decode(errors="replace")
        if "101" not in status:
            raise RuntimeError("handshake rejected: %s" % status)
        self.sock.settimeout(timeout)
        self.next_id = 1

    def _send(self, obj):
        data = json.dumps(obj).encode()
        header = bytearray([0x81])
        n = len(data)
        mask = os.urandom(4)
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header += struct.pack(">H", n)
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", n)
        header += mask
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        self.sock.sendall(bytes(header) + masked)

    def _read_exact(self, n):
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                return None
            buf += chunk
        return buf

    def _recv(self):
        while True:
            head = self._read_exact(2)
            if head is None:
                return None
            opcode = head[0] & 0x0F
            length = head[1] & 0x7F
            if length == 126:
                length = struct.unpack(">H", self._read_exact(2))[0]
            elif length == 127:
                length = struct.unpack(">Q", self._read_exact(8))[0]
            payload = self._read_exact(length) if length else b""
            if opcode == 0x8:  # close
                return None
            if opcode == 0x1:
                return json.loads(payload.decode())
            # ping/pong/binary: keep reading

    def wait_target(self, tries=10):
        for _ in range(tries):
            msg = self._recv()
            if msg is None:
                break
            if msg.get("method") == "Target.targetCreated":
                return msg["params"]["targetInfo"]["targetId"]
        raise RuntimeError(
            "no Target.targetCreated arrived; the app exposes no inspectable target"
        )

    def evaluate(self, target_id, expression, tries=40):
        self.next_id += 1
        inner_id = self.next_id
        inner = {
            "id": inner_id,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expression,
                "returnByValue": True,
                "includeCommandLineAPI": True,
            },
        }
        self.next_id += 1
        self._send(
            {
                "id": self.next_id,
                "method": "Target.sendMessageToTarget",
                "params": {"targetId": target_id, "message": json.dumps(inner)},
            }
        )
        for _ in range(tries):
            msg = self._recv()
            if msg is None:
                raise RuntimeError("connection closed while waiting for the answer")
            if msg.get("method") != "Target.dispatchMessageFromTarget":
                continue
            inner_msg = json.loads(msg["params"]["message"])
            if inner_msg.get("id") != inner_id:
                continue
            if "error" in inner_msg:
                raise RuntimeError(
                    "inspector error: %s" % json.dumps(inner_msg["error"])
                )
            result = inner_msg.get("result", {})
            if result.get("wasThrown"):
                raise RuntimeError(
                    "JS exception: %s"
                    % result.get("result", {}).get("description", "unknown")
                )
            return result.get("result", {})
        raise RuntimeError("no answer after %d messages" % tries)

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


def render(value):
    """Returns plain text; objects come out as compact JSON."""
    if "value" not in value:
        # undefined, functions, non serializable nodes
        return value.get("description", value.get("type", "undefined"))
    val = value["value"]
    if isinstance(val, str):
        return val
    return json.dumps(val, ensure_ascii=False, indent=2)


def main():
    ap = argparse.ArgumentParser(
        description="Evaluates JS in the Euro-Office Lite webview via the remote inspector."
    )
    ap.add_argument("expression", nargs="?", help="JS expression to evaluate")
    ap.add_argument("-f", "--file", help="read the expression from a file")
    ap.add_argument("--host", default=DEFAULT_HOST)
    ap.add_argument("--port", type=int, default=DEFAULT_PORT)
    ap.add_argument("--path", default=DEFAULT_PATH)
    ap.add_argument("--timeout", type=float, default=10.0)
    args = ap.parse_args()

    if args.file:
        with open(args.file, encoding="utf-8") as fh:
            expression = fh.read()
    elif args.expression:
        expression = args.expression
    elif not sys.stdin.isatty():
        expression = sys.stdin.read()
    else:
        ap.error("missing expression (argument, -f or stdin)")

    try:
        insp = Inspector(args.host, args.port, args.path, args.timeout)
    except OSError as exc:
        print(
            "could not connect to %s:%d - is the app running with "
            "WEBKIT_INSPECTOR_HTTP_SERVER? (%s)" % (args.host, args.port, exc),
            file=sys.stderr,
        )
        return 1
    except RuntimeError as exc:
        print("connection error: %s" % exc, file=sys.stderr)
        return 1

    try:
        target = insp.wait_target()
        print(render(insp.evaluate(target, expression)))
        return 0
    except (RuntimeError, socket.timeout) as exc:
        print("failed: %s" % exc, file=sys.stderr)
        return 1
    finally:
        insp.close()


if __name__ == "__main__":
    sys.exit(main())
