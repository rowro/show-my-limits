"""Prototype: a name-constrained CA whose private key never touches disk.

The CA signs a leaf for the configured server host, then its key is dropped.
mitmproxy receives only the leaf chain via --certs; its own confdir CA stays
untrusted. Anything that steals files later cannot mint certificates for other
names, and Name Constraints limit the root even if its key had leaked.
"""

from __future__ import annotations

import datetime as dt
import ipaddress
from dataclasses import dataclass
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

# Apple rejects user-trusted TLS leaves valid for more than 825 days; keep the
# leaf within the public 398-day limit and inside the CA validity.
CA_DAYS = 400
LEAF_DAYS = 397


@dataclass
class Issued:
    ca_pem: bytes
    leaf_chain_pem: bytes  # leaf key + leaf cert + CA cert, for mitmproxy --certs
    extra: dict[str, bytes]  # test-only leaves signed before the key is dropped


def _name(common_name: str) -> x509.Name:
    return x509.Name([
        x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Show My Limits"),
        x509.NameAttribute(NameOID.COMMON_NAME, common_name),
    ])


def _pem_key(key: ec.EllipticCurvePrivateKey) -> bytes:
    return key.private_bytes(
        serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8,
        serialization.NoEncryption(),
    )


def _leaf(host: str, ca_key, ca_cert: x509.Certificate, now: dt.datetime):
    key = ec.generate_private_key(ec.SECP256R1())
    cert = (
        x509.CertificateBuilder()
        .subject_name(_name(host))
        .issuer_name(ca_cert.subject)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - dt.timedelta(minutes=5))
        .not_valid_after(now + dt.timedelta(days=LEAF_DAYS))
        .add_extension(x509.SubjectAlternativeName([x509.DNSName(host)]), critical=False)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(x509.KeyUsage(True, False, False, False, False, False, False, False, False), critical=True)
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(ca_key.public_key()), critical=False)
        .sign(ca_key, hashes.SHA256())
    )
    return key, cert


def issue(host: str, test_only_hosts: tuple[str, ...] = ()) -> Issued:
    """Create CA + leaf for `host`. The CA key exists only inside this call.

    `test_only_hosts` lets the test suite obtain leaves outside the constraint
    to prove clients reject them; production code must never pass it.
    """
    now = dt.datetime.now(dt.timezone.utc)
    ca_key = ec.generate_private_key(ec.SECP256R1())
    constraints = x509.NameConstraints(
        permitted_subtrees=[x509.DNSName(host)],
        # No IP-address certificates under this root at all.
        excluded_subtrees=[
            x509.IPAddress(ipaddress.ip_network("0.0.0.0/0")),
            x509.IPAddress(ipaddress.ip_network("::/0")),
        ],
    )
    ca_cert = (
        x509.CertificateBuilder()
        .subject_name(_name(f"Show My Limits CA for {host}"))
        .issuer_name(_name(f"Show My Limits CA for {host}"))
        .public_key(ca_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - dt.timedelta(minutes=5))
        .not_valid_after(now + dt.timedelta(days=CA_DAYS))
        .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
        .add_extension(x509.KeyUsage(False, False, False, False, False, True, True, False, False), critical=True)
        .add_extension(constraints, critical=True)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(ca_key.public_key()), critical=False)
        .sign(ca_key, hashes.SHA256())
    )
    ca_pem = ca_cert.public_bytes(serialization.Encoding.PEM)
    leaf_key, leaf_cert = _leaf(host, ca_key, ca_cert, now)
    chain = _pem_key(leaf_key) + leaf_cert.public_bytes(serialization.Encoding.PEM) + ca_pem
    extra = {}
    for other in test_only_hosts:
        k, c = _leaf(other, ca_key, ca_cert, now)
        extra[other] = _pem_key(k) + c.public_bytes(serialization.Encoding.PEM) + ca_pem
    del ca_key  # never serialized; unreachable after return
    return Issued(ca_pem, chain, extra)


def write(issued: Issued, directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "ca.pem").write_bytes(issued.ca_pem)
    leaf = directory / "leaf-chain.pem"
    leaf.write_bytes(issued.leaf_chain_pem)
    leaf.chmod(0o600)
