"""TLS warning recovery without network traffic or persisted request contents."""
import importlib.util
import unittest
from pathlib import Path
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("limits_addon", Path(__file__).resolve().parents[1] / "proxy" / "limits_addon.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class HealthRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.addon = module.ShowMyLimitsAddon()
        self.addon.hosts = {"gateway.example", "other.example"}
        self.addon.publish = lambda: None
        self.addon.health = {"kind": "certificate_client", "host": "gateway.example", "observedAt": 1}

    def response(self, host="gateway.example", headers=None):
        return SimpleNamespace(server_conn=SimpleNamespace(sni=host), request=SimpleNamespace(host=host),
                               response=SimpleNamespace(headers=headers or {}, status_code=200, stream=False))

    def test_http_response_without_quota_clears_old_tls_warning(self):
        self.addon.responseheaders(self.response())
        self.assertEqual(self.addon.health["kind"], "response_received")
        self.assertEqual(self.addon.profiles, {})

    def test_ipv6_route_failure_is_not_a_certificate_error(self):
        server = SimpleNamespace(sni="gateway.example", error="[Errno 101] Network is unreachable", address=("2001:db8::1", 443))
        self.addon.server_connect_error(SimpleNamespace(context=SimpleNamespace(server=server, client=SimpleNamespace(sni="gateway.example"))))
        self.assertEqual(self.addon.health["kind"], "network_ipv6")
        self.addon.responseheaders(self.response())
        self.assertEqual(self.addon.health["kind"], "response_received")

    def test_other_host_does_not_clear_failed_host(self):
        self.addon.responseheaders(self.response("other.example"))
        self.assertEqual(self.addon.health["kind"], "certificate_client")

    def test_actual_certificate_failure_remains_until_success(self):
        conn = SimpleNamespace(sni="gateway.example", error="tlsv1 alert unknown ca")
        context = SimpleNamespace(client=conn, server=SimpleNamespace(sni="gateway.example"))
        self.addon.tls_failed_client(SimpleNamespace(context=context, conn=conn))
        self.assertEqual(self.addon.health["kind"], "certificate_client")
        self.addon.responseheaders(self.response(headers={"Anthropic-Ratelimit-Unified-5h-Utilization": "0.5"}))
        self.assertEqual(self.addon.health["kind"], "quota_received")
        self.assertEqual(len(self.addon.profiles), 1)

if __name__ == "__main__":
    unittest.main()
