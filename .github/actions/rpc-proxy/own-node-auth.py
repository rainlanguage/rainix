#!/usr/bin/env python3
"""Local auth forwarder from eRPC to our own RPC nodes.

Our nodes sit behind st0x-rpc-gate at <chain>-mainnet.arpeecee.com, which
takes a GitHub Actions OIDC token as `Authorization: Bearer`. A token lives
about five minutes and a fork suite runs longer, so eRPC cannot carry one as a
static header. This forwarder listens on 127.0.0.1, takes eRPC's JSON-RPC
POST at /<network>, adds a token minted for that gate's audience (re-minted a
minute before it expires), and sends it on over a kept-alive HTTPS
connection. No secret is stored anywhere: the token comes from the job's own
ACTIONS_ID_TOKEN_REQUEST_URL, which exists only when the caller grants
`id-token: write`.

Tokens are never printed. `--claims` prints the non-secret claims of one token
(repository, workflow ref, event, lifetime), so the gate's trust list can be
checked against what the job really sends.

Usage: own-node-auth.py serve <port> | own-node-auth.py --claims <network>
"""

import base64
import http.client
import json
import os
import sys
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Network key (rainix-static's NETWORKS) to gate host. The audience is the
# gate's public URL with no trailing slash, byte for byte what the gate expects.
GATES = {
    "ethereum": "eth-mainnet.arpeecee.com",
    "robinhood": "robinhood-mainnet.arpeecee.com",
    "base": "base-mainnet.arpeecee.com",
}

# Re-mint this long before `exp`, so a token never expires in flight.
REFRESH_MARGIN_SECS = 60
# The gate refuses bodies over 4 MiB; refuse them here first.
MAX_BODY_BYTES = 4 * 1024 * 1024
UPSTREAM_TIMEOUT_SECS = 30


def audience(host):
    return f"https://{host}"


def claims(token):
    payload = token.split(".")[1]
    payload += "=" * (-len(payload) % 4)
    return json.loads(base64.urlsafe_b64decode(payload))


def mint(aud):
    url = os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"]
    url += ("&" if "?" in url else "?") + "audience=" + urllib.parse.quote(aud, safe="")
    req = urllib.request.Request(
        url,
        headers={"Authorization": "bearer " + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]},
    )
    with urllib.request.urlopen(req, timeout=15) as resp:
        token = json.load(resp)["value"]
    return token, int(claims(token)["exp"])


class Tokens:
    def __init__(self):
        self._lock = threading.Lock()
        self._cache = {}

    def get(self, aud, fresh=False):
        with self._lock:
            hit = self._cache.get(aud)
            if fresh or hit is None or hit[1] - time.time() < REFRESH_MARGIN_SECS:
                hit = mint(aud)
                self._cache[aud] = hit
            return hit[0]


TOKENS = Tokens()
LOCAL = threading.local()


def connection(host):
    conns = getattr(LOCAL, "conns", None)
    if conns is None:
        conns = LOCAL.conns = {}
    conn = conns.get(host)
    if conn is None:
        conn = conns[host] = http.client.HTTPSConnection(host, timeout=UPSTREAM_TIMEOUT_SECS)
    return conn


def drop(host):
    conn = getattr(LOCAL, "conns", {}).pop(host, None)
    if conn is not None:
        conn.close()


def forward(host, body):
    """POST `body` to the gate; one retry on a dropped connection, and one on
    a 401 with a freshly minted token (the gate's clock may run ahead)."""
    fresh = False
    for attempt in range(3):
        token = TOKENS.get(audience(host), fresh=fresh)
        conn = connection(host)
        try:
            conn.request(
                "POST",
                "/",
                body,
                {"Content-Type": "application/json", "Authorization": "Bearer " + token},
            )
            resp = conn.getresponse()
            data = resp.read()
        except (http.client.HTTPException, OSError):
            drop(host)
            if attempt == 2:
                raise
            continue
        if resp.status == 401 and not fresh:
            fresh = True
            continue
        return resp.status, resp.getheader("Content-Type", "application/json"), data
    return resp.status, resp.getheader("Content-Type", "application/json"), data


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def reply(self, status, ctype, data):
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def error(self, status, message):
        body = json.dumps(
            {"jsonrpc": "2.0", "id": None, "error": {"code": -32603, "message": message}}
        ).encode()
        self.reply(status, "application/json", body)

    def do_GET(self):
        if self.path == "/healthz":
            self.reply(200, "text/plain", b"ok")
        else:
            self.error(404, "not found")

    def do_POST(self):
        host = GATES.get(self.path.strip("/"))
        length = int(self.headers.get("Content-Length") or 0)
        if host is None:
            self.rfile.read(length)
            self.error(404, "own-node-auth: unknown network")
            return
        if length > MAX_BODY_BYTES:
            self.close_connection = True
            self.error(413, "own-node-auth: body too large")
            return
        body = self.rfile.read(length)
        try:
            status, ctype, data = forward(host, body)
        except Exception as e:  # noqa: BLE001 - any failure is a 502 eRPC fails over on
            self.error(502, f"own-node-auth: {type(e).__name__}")
            return
        self.reply(status, ctype, data)


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--claims":
        token, _ = mint(audience(GATES[sys.argv[2]]))
        c = claims(token)
        keep = ["aud", "repository", "repository_owner_id", "job_workflow_ref", "ref", "event_name"]
        shown = {k: c.get(k) for k in keep}
        shown["lifetime_secs"] = int(c["exp"]) - int(c["iat"])
        print(json.dumps(shown))
        return
    if len(sys.argv) == 3 and sys.argv[1] == "serve":
        server = ThreadingHTTPServer(("127.0.0.1", int(sys.argv[2])), Handler)
        server.daemon_threads = True
        server.serve_forever()
        return
    sys.exit(__doc__)


if __name__ == "__main__":
    main()
