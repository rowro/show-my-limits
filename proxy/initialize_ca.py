"""Create an app-named CA in a fresh staging directory, without intercepting traffic."""
import os
from pathlib import Path
from mitmproxy import certs, ctx


def load(loader):
    output = Path(os.environ["SML_CA_OUTPUT"])
    # Never replace existing key material, including partially initialized stores.
    output.mkdir(parents=True, exist_ok=False)
    certs.CertStore.create_store(
        output, "mitmproxy", 2048,
        organization="Show My Limits", cn="limits-mitmproxy",
    )


def running():
    ctx.master.shutdown()
