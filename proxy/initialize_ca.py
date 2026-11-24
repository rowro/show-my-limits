"""Create an app-named, one-year CA in a fresh staging directory."""
from datetime import datetime, timedelta, timezone
import os
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509 import ExtendedKeyUsageOID, NameOID
from mitmproxy import certs, ctx


def create_store(output):
    # Keep the file format expected by mitmproxy 12.3.1, but avoid its
    # hard-coded 10-year CA validity. No trust or traffic capture occurs here.
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([
        x509.NameAttribute(NameOID.COMMON_NAME, "limits-mitmproxy"),
        x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Show My Limits"),
    ])
    now = datetime.now(timezone.utc)
    ca = (x509.CertificateBuilder()
        .serial_number(x509.random_serial_number())
        .subject_name(name).issuer_name(name)
        .public_key(key.public_key())
        .not_valid_before(now - timedelta(hours=1))
        .not_valid_after(now + timedelta(days=365))
        .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
        .add_extension(x509.KeyUsage(
            digital_signature=False, content_commitment=False,
            key_encipherment=False, data_encipherment=False,
            key_agreement=False, key_cert_sign=True, crl_sign=True,
            encipher_only=False, decipher_only=False,
        ), critical=True)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()), critical=False)
        .sign(private_key=key, algorithm=hashes.SHA256()))
    key_pem = key.private_bytes(serialization.Encoding.PEM,
        serialization.PrivateFormat.TraditionalOpenSSL,
        serialization.NoEncryption())
    cert_pem = ca.public_bytes(serialization.Encoding.PEM)
    (output / "mitmproxy-ca.pem").write_bytes(key_pem + cert_pem)
    (output / "mitmproxy-ca-cert.pem").write_bytes(cert_pem)
    (output / "mitmproxy-ca-cert.cer").write_bytes(cert_pem)
    (output / "mitmproxy-dhparam.pem").write_bytes(certs.DEFAULT_DHPARAM)


def load(loader):
    output = Path(os.environ["SML_CA_OUTPUT"])
    # Never replace existing key material, including partially initialized stores.
    previous_umask = os.umask(0o077)
    try:
        output.mkdir(parents=True, exist_ok=False)
        create_store(output)
    finally:
        os.umask(previous_umask)


def running():
    ctx.master.shutdown()
