"""Show My Limits's mitmproxy addon. Saves quota metadata only; never logs traffic bodies or credentials."""
import asyncio
import hashlib
import json
import math
import os
from pathlib import Path
import re
import tempfile
import time

PREFIX = "anthropic-ratelimit-unified-"
WINDOW = re.compile(r"^" + PREFIX + r"(\d+[hd](?:_[a-z0-9]+)?)-(utilization|reset|status)$")


def number(value):
    try:
        parsed = float(value)
        return parsed if math.isfinite(parsed) and parsed >= 0 else None
    except (ValueError, TypeError, OverflowError):
        return None


def parse_headers(headers):
    headers = {str(k).lower(): str(v) for k, v in headers.items()}
    windows = {}
    for key, value in headers.items():
        match = WINDOW.fullmatch(key)
        if match:
            name, field = match.groups()
            windows.setdefault(name, {})[field] = number(value) if field != "status" else value[:80]
    if not windows:
        return None
    return {"windows": windows, "status": headers.get(PREFIX + "status"),
            "overageReason": headers.get(PREFIX + "overage-disabled-reason")}


def host_name(value):
    return str(value or "").lower().rstrip(".")


class ShowMyLimitsAddon:
    def __init__(self):
        self.directory = Path(os.environ.get("SML_STATE_DIR", str(Path.home() / ".local/share/show-my-limits/proxy")))
        self.hosts = {host_name(h) for h in json.loads(os.environ.get("SML_HOSTS", "[]"))}
        self.profiles = {}
        self.health = None
        self.started_at = time.time()
        self.handle = None

    def write(self, running=True):
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        data = {"version": 1, "pid": os.getpid(), "running": running,
                "updatedAt": time.time(), "startedAt": self.started_at,
                "profiles": list(self.profiles.values()), "health": self.health}
        descriptor, name = tempfile.mkstemp(prefix=".show-my-limits-", dir=str(self.directory))
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as output:
                json.dump(data, output, ensure_ascii=False, allow_nan=False)
            os.replace(name, self.directory / "state.json")
        finally:
            if os.path.exists(name):
                os.unlink(name)

    def publish(self):
        try:
            self.write()
        except OSError:
            print("SML_ERROR:state_write", flush=True)

    def heartbeat(self):
        self.publish()
        self.handle = asyncio.get_running_loop().call_later(5, self.heartbeat)

    def running(self):
        self.started_at = time.time()
        self.heartbeat()

    def done(self):
        if self.handle:
            self.handle.cancel()
        try:
            self.write(False)
        except OSError:
            pass

    def requestheaders(self, flow):
        flow.request.stream = True

    def responseheaders(self, flow):
        if flow.response is None:
            return
        flow.response.stream = True
        server = getattr(flow, "server_conn", None)
        candidates = [host_name(getattr(server, "sni", None)), host_name(flow.request.host)]
        host = next((name for name in candidates if name in self.hosts), None)
        if host is None:
            return
        quota = parse_headers(flow.response.headers)
        if quota is None:
            # An HTTP response proves a completed TLS connection even when this
            # endpoint carries no quota headers. Do not leave an older TLS
            # failure on screen while the client is successfully communicating.
            if self.health and self.health.get("host") == host and self.health.get("kind") in {
                "certificate_client", "certificate_server", "tls_client", "tls_server"
            }:
                self.health = {"kind": "response_received", "host": host, "observedAt": time.time()}
                self.publish()
            return
        # Distinguish organizations without persisting IDs or authentication headers.
        organization = flow.response.headers.get("anthropic-organization-id", "default")
        identity = hashlib.sha256((host + "\0" + organization).encode()).hexdigest()[:16]
        self.profiles[identity] = dict(quota, id=identity, host=host, observedAt=time.time())
        if len(self.profiles) > 128:
            oldest = min(self.profiles, key=lambda key: self.profiles[key]["observedAt"])
            del self.profiles[oldest]
        self.health = {"kind": "quota_received", "host": host, "observedAt": time.time()}
        self.publish()

    def tls_failure(self, data, side):
        context = data.context
        candidates = [host_name(getattr(context.client, "sni", None)),
                      host_name(getattr(context.server, "sni", None)), host_name(getattr(data.conn, "sni", None))]
        host = next((name for name in candidates if name in self.hosts), None)
        if host is None:
            return
        error = str(getattr(data.conn, "error", "") or "").lower()
        # EOF and cancellation do not prove rejection of a certificate.
        if any(token in error for token in ("unknown ca", "unknown_ca", "certificate unknown",
                                            "bad certificate", "certificate verify failed", "certificate_verify_failed")):
            kind = "certificate_" + side
        elif any(token in error for token in ("closed", "disconnected", "eof")):
            return
        else:
            kind = "tls_" + side
        self.health = {"kind": kind, "host": host, "observedAt": time.time()}
        self.publish()

    def tls_failed_client(self, data):
        self.tls_failure(data, "client")

    def tls_failed_server(self, data):
        self.tls_failure(data, "server")


addons = [ShowMyLimitsAddon()]
