"""Home Assistant WebSocket transport, stdlib only.

Shared by every skill in this Pack that needs the WS API. It exists because
Home Assistant's registries -- device, entity, area -- are reachable *only*
over WebSocket: the REST API answers 404 for all three, while config entries
themselves are plain REST. Any skill that wants to say what deleting a config
entry will take with it has to speak both.

## Why it is here and not in a skill

`ha-dashboard` grew this client first. Copying it into a second skill would put
two hand-rolled RFC 6455 implementations in one Pack, and framing bugs found in
one would live on in the other.

## How a skill reaches it

Packs are installed by symlinking each skill directory into `~/.claude/skills`,
so a script's own path runs *through* that symlink while `os.path.realpath`
resolves back into the Pack. Three levels up from `scripts/` is the Pack root,
and `lib/` sits beside `skills/` there:

    import os, sys
    sys.path.insert(0, os.path.join(
        os.path.dirname(os.path.realpath(__file__)), "..", "..", "..", "lib"))
    from ha_ws import HAWebSocket, WSError

`realpath`, not `abspath`: the latter stops at the symlink and lands in
`~/.claude/lib`, which does not exist. No change to `tools/packs.sh` is needed
for any of this -- the symlink already points into the Pack, so the lib travels
with the skills that use it.

## Scope

Transport only. Credential composition stays in each skill, because how a
script finds its URL and token is the skill's business and differs between them.
"""
import base64
import hashlib
import json
import os
import socket
import ssl
import struct
from urllib.parse import urlparse


class WSError(Exception):
    pass


class HAWebSocket:
    """Just enough of RFC 6455 to drive Home Assistant's WS API: masked client
    frames, 7/16/64-bit lengths, fragmentation reassembly, ping/pong handling."""

    def __init__(self, url: str, token: str, timeout: int = 20):
        self.token = token
        self.timeout = timeout
        u = urlparse(url)
        if (u.scheme not in ("http", "https", "ws", "wss") or not u.hostname
                or u.username is not None or u.password is not None):
            raise WSError("HA_URL must be an http(s) or ws(s) URL with a host and no embedded credentials")
        self.secure = u.scheme in ("https", "wss")
        self.host = u.hostname
        self.port = u.port or (443 if self.secure else 80)
        self.path = (u.path or "").rstrip("/") + "/api/websocket"
        self.sock = None
        self._buf = b""
        self._id = 0
        self.ha_version = None

    def _connect(self) -> None:
        raw = socket.create_connection((self.host, self.port), timeout=self.timeout)
        if self.secure:
            # create_default_context() verifies certs and hostnames but does not floor the
            # protocol version: TLS 1.0 and 1.1 stay permitted unless the OpenSSL build or a
            # system policy forbids them, which is not something a skill can assume about the
            # machine it lands on. Set the floor here so the guarantee travels with the script.
            ctx = ssl.create_default_context()
            ctx.minimum_version = ssl.TLSVersion.TLSv1_2
            raw = ctx.wrap_socket(raw, server_hostname=self.host)
        raw.settimeout(self.timeout)
        self.sock = raw

        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET {self.path} HTTP/1.1\r\n"
            f"Host: {self.host}:{self.port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "\r\n"
        )
        self.sock.sendall(req.encode())

        resp = b""
        while b"\r\n\r\n" not in resp:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise WSError("connection closed during WebSocket handshake")
            resp += chunk
        head, _, rest = resp.partition(b"\r\n\r\n")
        status = head.split(b"\r\n", 1)[0].decode(errors="replace")
        if "101" not in status:
            raise WSError(f"WebSocket upgrade rejected: {status}")
        self._buf = rest  # any bytes past the handshake are the first frame(s)

    def _read(self, n: int) -> bytes:
        while len(self._buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise WSError("connection closed by server")
            self._buf += chunk
        data, self._buf = self._buf[:n], self._buf[n:]
        return data

    def _recv_frame(self):
        b1, b2 = self._read(2)
        fin = b1 & 0x80
        opcode = b1 & 0x0F
        masked = b2 & 0x80
        length = b2 & 0x7F
        if length == 126:
            length = struct.unpack(">H", self._read(2))[0]
        elif length == 127:
            length = struct.unpack(">Q", self._read(8))[0]
        mask = self._read(4) if masked else b""
        payload = self._read(length)
        if masked:
            payload = bytes(payload[i] ^ mask[i % 4] for i in range(length))
        return fin, opcode, payload

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        length = len(payload)
        header = bytearray([0x80 | opcode])  # FIN + opcode
        if length < 126:
            header.append(0x80 | length)     # MASK bit + len
        elif length < 65536:
            header.append(0x80 | 126)
            header += struct.pack(">H", length)
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", length)
        mask = os.urandom(4)
        header += mask
        masked = bytes(payload[i] ^ mask[i % 4] for i in range(length))
        self.sock.sendall(bytes(header) + masked)

    def _recv_message(self) -> str:
        chunks = []
        while True:
            fin, opcode, payload = self._recv_frame()
            if opcode == 0x8:            # close
                raise WSError("server closed the connection")
            if opcode == 0x9:            # ping -> pong
                self._send_frame(0xA, payload)
                continue
            if opcode == 0xA:            # pong
                continue
            chunks.append(payload)
            if fin:
                break
        return b"".join(chunks).decode("utf-8")

    def _send_json(self, obj) -> None:
        self._send_frame(0x1, json.dumps(obj).encode("utf-8"))

    def connect_and_auth(self) -> None:
        self._connect()
        banner = json.loads(self._recv_message())
        if banner.get("type") != "auth_required":
            raise WSError(f"unexpected banner: {banner.get('type')!r}")
        self._send_json({"type": "auth", "access_token": self.token})
        msg = json.loads(self._recv_message())
        if msg.get("type") != "auth_ok":
            raise WSError("authentication failed (check the HA_TOKEN field on the bw item)")
        self.ha_version = msg.get("ha_version")

    def command(self, payload: dict) -> dict:
        self._id += 1
        self._send_json({"id": self._id, **payload})
        while True:
            msg = json.loads(self._recv_message())
            if msg.get("id") == self._id and msg.get("type") == "result":
                return msg

    def close(self) -> None:
        try:
            if self.sock:
                self._send_frame(0x8, b"")
                self.sock.close()
        except OSError:
            pass
