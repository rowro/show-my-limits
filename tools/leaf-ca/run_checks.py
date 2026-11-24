"""Checks for the leaf-only CA prototype.

1. mitmproxy --certs: a client trusting only ca.pem reaches the configured host
   through mitmproxy, sees our leaf and the quota headers.
2. A host outside --certs is signed by mitmproxy's own (untrusted) CA and the
   client rejects it.
3. Name Constraints: leaves for other names, signed by the same CA before its
   key was dropped, are rejected by every available verifier.

By default no OS trust store is touched. Chrome on Linux uses a temporary NSS
database under a temporary HOME. `--os-trust` additionally adds the CA to the
machine trust store so Chrome uses the platform verifier on macOS/Windows; it
refuses to run outside GitHub Actions and removes the CA afterwards.

Usage: run_checks.py <mitmdump path or name in PATH> [--host corp-llm.test] [--json out.json] [--os-trust]
"""

from __future__ import annotations

import argparse
import contextlib
import http.server
import json
import os
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import leaf_ca  # noqa: E402

BODY = b"sml-quota-ok"
QUOTA_HEADERS = {
    "anthropic-ratelimit-unified-status": "allowed",
    "anthropic-ratelimit-unified-5h-utilization": "0.42",
    "anthropic-ratelimit-unified-5h-reset": "1767225600",
}

# Verifiers known to deviate from RFC 5280: (verifier substring, host predicate).
# LibreSSL (macOS /usr/bin/curl) matches dNSName constraints as a plain string
# suffix, so "notcorp.example" passes a "corp.example" constraint. Exploiting it
# still needs the CA key, which is never stored. Reported as WARN, not failure.
KNOWN_WEAK = [("LibreSSL", lambda host, base: host.endswith(base) and not host.endswith("." + base))]
results: list[dict] = []


def record(group: str, name: str, ok: bool, detail: str = "", known: bool = False) -> None:
    results.append({"group": group, "name": name, "ok": ok or known, "known_weak": known, "detail": detail})
    label = "WARN" if known else ("PASS" if ok else "FAIL")
    print(f"[{label}] {group}: {name} {detail}".rstrip(), flush=True)


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        for k, v in QUOTA_HEADERS.items():
            self.send_header(k, v)
        self.send_header("content-type", "text/plain")
        self.send_header("content-length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)

    def log_message(self, *_):
        pass


class QuietServer(http.server.ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        pass  # rejected handshakes are expected


def tls_server(chain_pem: Path) -> int:
    port = free_port()
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(chain_pem)
    httpd = QuietServer(("127.0.0.1", port), Handler)
    httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return port


def curl(binary: str, url: str, ca: Path, resolve: str | None = None, proxy: str | None = None):
    # Schannel checks revocation by default; test leaves have no CRL/OCSP endpoint.
    schannel = "Schannel" in subprocess.run([binary, "--version"], capture_output=True, text=True).stdout
    args = [binary, "-sS", *(["--ssl-no-revoke"] if schannel else []), "-o", os.devnull, "-D", "-", "--cacert", str(ca),
            "-w", "\nVERIFY=%{ssl_verify_result}\n", "--max-time", "15", url]
    if resolve:
        args += ["--resolve", resolve]
    if proxy:
        args += ["--proxy", proxy, "--noproxy", ""]
    p = subprocess.run(args, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def last_line(text: str) -> str:
    lines = text.strip().splitlines()
    errors = [l for l in lines if l.startswith("curl: (")]
    if errors:
        return errors[0][:160]
    return lines[-1][:160] if lines else ""


NODE_CLIENT = r"""
const tls = require('tls'), fs = require('fs');
const [port, host, ca] = process.argv.slice(1);
const s = tls.connect({host: '127.0.0.1', port: +port, servername: host, ca: fs.readFileSync(ca)}, () => {
  console.log('OK'); s.end(); });
s.on('error', e => { console.log('ERR ' + (e.code || e.message)); process.exit(1); });
"""

# .NET X509Chain with an exclusive custom root: Windows CryptoAPI chain engine,
# no store modification. Prints the chain status flags on failure.
POWERSHELL_CHAIN = r"""
param($leaf, $ca)
$l = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($leaf)
$r = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($ca)
$c = [System.Security.Cryptography.X509Certificates.X509Chain]::new()
$c.ChainPolicy.TrustMode = 'CustomRootTrust'
$c.ChainPolicy.CustomTrustStore.Add($r) | Out-Null
$c.ChainPolicy.RevocationMode = 'NoCheck'
if ($c.Build($l)) { 'OK'; exit 0 }
($c.ChainStatus | ForEach-Object { $_.Status }) -join ','; exit 1
"""


def find_chrome() -> str | None:
    candidates = [
        os.environ.get("CHROME_BIN"),
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        shutil.which("google-chrome"), shutil.which("google-chrome-stable"),
        shutil.which("chromium"), shutil.which("chromium-browser"),
    ]
    return next((c for c in candidates if c and os.path.exists(c)), None)


class ChromeTrust:
    """Makes ca.pem trusted for Chrome, or reports why it cannot."""

    def __init__(self, ca: Path, work: Path, os_trust: bool):
        self.ca, self.work, self.os_trust = ca, work, os_trust
        self.env = dict(os.environ)
        self.ready = False
        self.reason = ""
        self._undo: list[list[str]] = []

    def __enter__(self):
        if sys.platform.startswith("linux"):
            # Chrome on Linux reads $HOME/.pki/nssdb: a temporary HOME keeps this local.
            if not shutil.which("certutil"):
                self.reason = "certutil (libnss3-tools) missing"
                return self
            home = self.work / "chrome-home"
            db = home / ".pki" / "nssdb"
            db.mkdir(parents=True)
            subprocess.run(["certutil", "-d", f"sql:{db}", "-N", "--empty-password"], check=True)
            subprocess.run(["certutil", "-d", f"sql:{db}", "-A", "-t", "C,,", "-n", "sml-test", "-i", str(self.ca)], check=True)
            self.env["HOME"] = str(home)
            self.ready = True
        elif not self.os_trust:
            self.reason = "needs --os-trust (platform trust store)"
        elif os.environ.get("GITHUB_ACTIONS") != "true":
            self.reason = "--os-trust refused outside GitHub Actions"
        elif sys.platform == "darwin":
            subprocess.run(["sudo", "security", "add-trusted-cert", "-d", "-r", "trustRoot",
                            "-k", "/Library/Keychains/System.keychain", str(self.ca)], check=True, timeout=60)
            self._undo.append(["sudo", "security", "remove-trusted-cert", "-d", str(self.ca)])
            self.ready = True
        elif sys.platform == "win32":
            thumb = subprocess.run(["powershell", "-NoProfile", "-Command",
                                    f"(Import-Certificate -FilePath '{self.ca}' -CertStoreLocation Cert:\\LocalMachine\\Root).Thumbprint"],
                                   capture_output=True, text=True, check=True, timeout=60).stdout.strip()
            self._undo.append(["powershell", "-NoProfile", "-Command",
                               f"Remove-Item Cert:\\LocalMachine\\Root\\{thumb}"])
            self.ready = True
        return self

    def __exit__(self, *_):
        for cmd in self._undo:
            subprocess.run(cmd, check=False)


def chrome_accepts(chrome: str, trust: ChromeTrust, host: str, port: int) -> tuple[bool, str]:
    profile = tempfile.mkdtemp(prefix="chrome-", dir=trust.work)
    # Containers run as root, where Chrome refuses its sandbox; CI runners do not.
    no_sandbox = ["--no-sandbox"] if hasattr(os, "geteuid") and os.geteuid() == 0 else []
    p = subprocess.run([
        chrome, *no_sandbox, "--headless=new", "--no-first-run", "--no-default-browser-check", "--disable-gpu",
        f"--user-data-dir={profile}", "--host-resolver-rules=MAP * 127.0.0.1",
        "--disable-features=HttpsUpgrades", "--timeout=15000", "--dump-dom", f"https://{host}:{port}/",
    ], capture_output=True, text=True, env=trust.env, timeout=90)
    accepted = BODY.decode() in p.stdout
    detail = "" if accepted else ("interstitial" if "interstitial" in p.stdout.lower() or "ERR_CERT" in p.stdout
                                  else last_line(p.stderr))
    return accepted, detail


def verifiers(host: str, port: int, chain: Path, ca: Path, work: Path,
              chrome: str | None, trust: ChromeTrust) -> dict[str, tuple[bool, str]]:
    """Return {verifier: (accepted, detail)} for a server presenting `chain`."""
    out: dict[str, tuple[bool, str]] = {}
    pem = chain.read_text()
    leaf = work / f"{host}-leaf.pem"
    leaf.write_text(pem[pem.index("-----BEGIN CERTIFICATE-----"):pem.index("-----END CERTIFICATE-----")]
                    + "-----END CERTIFICATE-----\n")

    if shutil.which("openssl"):
        p = subprocess.run(["openssl", "verify", "-CAfile", str(ca), "-verify_hostname", host, str(leaf)],
                           capture_output=True, text=True)
        out["openssl verify"] = (p.returncode == 0, last_line(p.stdout + p.stderr))

    try:
        c = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)  # trusts only ca.pem
        c.load_verify_locations(cafile=str(ca))
        with socket.create_connection(("127.0.0.1", port), timeout=5) as raw, \
                c.wrap_socket(raw, server_hostname=host):
            out["python ssl"] = (True, "")
    except ssl.SSLError as e:
        out["python ssl"] = (False, e.reason or str(e))

    if shutil.which("node"):
        p = subprocess.run(["node", "-e", NODE_CLIENT, str(port), host, str(ca)], capture_output=True, text=True)
        out["node tls"] = (p.returncode == 0, p.stdout.strip())

    seen = set()
    for binary in ("/usr/bin/curl", shutil.which("curl")):
        if not binary or not os.path.exists(binary) or os.path.realpath(binary) in seen:
            continue
        seen.add(os.path.realpath(binary))
        tokens = subprocess.run([binary, "--version"], capture_output=True, text=True).stdout.split()
        lib = [t for t in tokens[:12] if "SSL" in t or "Schannel" in t or t.startswith("libcurl/")]
        code, text = curl(binary, f"https://{host}:{port}/", ca, resolve=f"{host}:{port}:127.0.0.1")
        ok = code == 0 and "VERIFY=0" in text
        out[f"curl {' '.join(lib)}"] = (ok, "" if ok else last_line(text))

    if sys.platform == "darwin":
        # Apple trust evaluation with ca.pem as an anchor, without keychain changes.
        p = subprocess.run(["security", "verify-cert", "-c", str(leaf), "-r", str(ca), "-p", "ssl", "-s", host, "-L", "-l"],
                           capture_output=True, text=True)
        out["macOS Security.framework"] = (p.returncode == 0, last_line(p.stdout + p.stderr))

    if sys.platform == "win32":
        script = work / "chain.ps1"
        script.write_text(POWERSHELL_CHAIN)
        shell = shutil.which("pwsh") or "powershell"
        p = subprocess.run([shell, "-NoProfile", "-File", str(script), str(leaf), str(ca)], capture_output=True, text=True)
        out["Windows CryptoAPI (X509Chain)"] = (p.returncode == 0, last_line(p.stdout + p.stderr))

    if chrome and trust.ready:
        out["Chrome"] = chrome_accepts(chrome, trust, host, port)
    return out


def wait_port(port: int, seconds: float = 30) -> None:
    deadline = time.time() + seconds
    while time.time() < deadline:
        with contextlib.suppress(OSError):
            socket.create_connection(("127.0.0.1", port), timeout=0.2).close()
            return
        time.sleep(0.1)
    raise TimeoutError(f"port {port} not listening")


def check_mitmproxy(mitmdump: str, host: str, chain: Path, ca: Path, work: Path) -> None:
    upstream = leaf_ca.issue(host)  # separate, unrelated CA for the fake upstream server
    up_chain = work / "upstream.pem"
    up_chain.write_bytes(upstream.leaf_chain_pem)
    up_ca = work / "upstream-ca.pem"
    up_ca.write_bytes(upstream.ca_pem)
    up_port = tls_server(up_chain)
    redirect = work / "redirect.py"
    redirect.write_text("def server_connect(data):\n"
                        f"    data.server.address = ('127.0.0.1', {up_port})\n")
    confdir = work / "mitm-confdir"
    proxy_port = free_port()
    escaped = host.replace(".", "[.]")
    mitm = subprocess.Popen([
        mitmdump, "--listen-host", "127.0.0.1", "--listen-port", str(proxy_port),
        "--certs", f"{host}={chain}",
        "--allow-hosts", rf"^(?:{escaped}|other[.]test)(?::\d+)?$",
        "--set", f"confdir={confdir}",
        "--set", f"ssl_verify_upstream_trusted_ca={up_ca}",
        "--set", "flow_detail=1", "-s", str(redirect),
    ], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    curl_bin = shutil.which("curl") or "curl"
    try:
        wait_port(proxy_port)
        proxy = f"http://127.0.0.1:{proxy_port}"
        code, text = curl(curl_bin, f"https://{host}/", ca, proxy=proxy)
        record("mitmproxy", f"--certs leaf served for {host}", code == 0 and "VERIFY=0" in text,
               "" if code == 0 else last_line(text))
        record("mitmproxy", "quota headers reach client",
               "anthropic-ratelimit-unified-5h-utilization: 0.42" in text.lower())
        code, text = curl(curl_bin, "https://other.test/", ca, proxy=proxy)
        record("mitmproxy", "host outside --certs rejected by client", code != 0,
               "" if code != 0 else "unexpectedly trusted")
        record("mitmproxy", "confdir CA is not the trusted CA",
               (confdir / "mitmproxy-ca-cert.pem").read_bytes() != ca.read_bytes())
    finally:
        mitm.terminate()
        log, _ = mitm.communicate(timeout=10)
        if any(not r["ok"] for r in results if r["group"] == "mitmproxy"):
            print("--- mitmdump log ---\n" + log[-3000:])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("mitmdump")
    ap.add_argument("--host", default="corp-llm.test")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--os-trust", action="store_true")
    args = ap.parse_args()
    host = args.host
    work = Path(tempfile.mkdtemp(prefix="leaf-ca-"))
    try:
        sub = f"api.{host}"
        evil = ["evil.test", f"{host}.evil.test", f"not{host}"]
        issued = leaf_ca.issue(host, test_only_hosts=(sub, *evil))
        store = work / "store"
        leaf_ca.write(issued, store)
        ca = store / "ca.pem"
        chain = store / "leaf-chain.pem"
        record("store", "only the leaf key is on disk",
               [p.name for p in store.iterdir() if b"PRIVATE KEY" in p.read_bytes()] == ["leaf-chain.pem"])

        chrome = find_chrome()
        with ChromeTrust(ca, work, args.os_trust) as trust:
            if not chrome:
                print("[SKIP] Chrome: not installed")
            elif not trust.ready:
                print(f"[SKIP] Chrome: {trust.reason}")
            cases = [(host, chain, True), (sub, None, True)] + [(e, None, False) for e in evil]
            for name, path, expect in cases:
                if path is None:
                    path = work / f"{name}.pem"
                    path.write_bytes(issued.extra[name])
                port = tls_server(path)
                for verifier, (accepted, detail) in verifiers(name, port, path, ca, work, chrome, trust).items():
                    want = "accept" if expect else "reject"
                    known = accepted and not expect and any(
                        lib in verifier and pred(name, host) for lib, pred in KNOWN_WEAK)
                    record("constraints", f"{verifier} must {want} {name}", accepted == expect,
                           "" if accepted == expect else f"({detail})", known=known)

        check_mitmproxy(shutil.which(args.mitmdump) or args.mitmdump, host, chain, ca, work)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    failed = [r for r in results if not r["ok"]]
    print(f"\n{len(results) - len(failed)}/{len(results)} passed on {sys.platform}")
    if args.json:
        args.json.write_text(json.dumps({"platform": sys.platform, "results": results}, indent=1))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
