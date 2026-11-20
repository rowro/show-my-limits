import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("limits_addon", Path(__file__).with_name("limits_addon.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AddonTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.environment = patch.dict(os.environ, {"SML_STATE_DIR": self.temporary.name,
                                                  "SML_HOSTS": '["gateway.example"]'})
        self.environment.start()
        self.addon = module.ShowMyLimitsAddon()

    def tearDown(self):
        self.environment.stop()
        self.temporary.cleanup()

    def test_state_carries_launch_identity(self):
        with patch.dict(os.environ, {"SML_RUN_ID": "launch-123"}):
            addon = module.ShowMyLimitsAddon()
        addon.write()
        data = json.loads((Path(self.temporary.name) / "state.json").read_text())
        self.assertEqual(data["runId"], "launch-123")
        self.assertEqual(data["pid"], os.getpid())

    def flow(self, path="/any/provider/prefix/messages?beta=false", org="org-private"):
        return NS(request=NS(host="192.0.2.1", path=path, stream=False),
                  server_conn=NS(sni="gateway.example"),
                  response=NS(stream=False, headers={
                      "Anthropic-Ratelimit-Unified-5h-utilization": "0.75",
                      "Anthropic-Ratelimit-Unified-5h-reset": "2000000000",
                      "Anthropic-Ratelimit-Unified-7d-utilization": "0.32",
                      "anthropic-organization-id": org, "Authorization": "secret-token"}))

    def read(self):
        return json.loads((Path(self.temporary.name) / "state.json").read_text())

    def test_any_path_sni_and_metadata_only(self):
        flow = self.flow()
        before = dict(flow.response.headers)
        self.addon.requestheaders(flow)
        self.addon.responseheaders(flow)
        self.assertTrue(flow.request.stream)
        self.assertTrue(flow.response.stream)
        self.assertEqual(flow.response.headers, before)
        state = self.read()
        self.assertEqual(state["profiles"][0]["host"], "gateway.example")
        self.assertEqual(state["profiles"][0]["windows"]["5h"]["utilization"], 0.75)
        self.assertNotIn("org-private", json.dumps(state))
        self.assertNotIn("secret-token", json.dumps(state))

    def test_other_hosts_and_missing_headers_ignored(self):
        flow = self.flow()
        flow.server_conn.sni = "unrelated.example"
        self.addon.responseheaders(flow)
        self.assertFalse((Path(self.temporary.name) / "state.json").exists())
        flow = self.flow()
        flow.response.headers = {"Content-Type": "text/event-stream"}
        self.addon.responseheaders(flow)
        self.assertFalse((Path(self.temporary.name) / "state.json").exists())

    def test_organizations_stay_separate_and_bad_values_are_unknown(self):
        self.addon.responseheaders(self.flow(org="one"))
        self.addon.responseheaders(self.flow(org="two"))
        self.assertEqual(len(self.read()["profiles"]), 2)
        self.assertIsNone(module.number("nan"))
        self.assertIsNone(module.number("inf"))
        self.assertIsNone(module.number(-1))
        self.assertEqual(module.number("1.2"), 1.2)

    def test_tls_eof_not_certificate_and_success_recovers(self):
        def event(message):
            return NS(context=NS(client=NS(sni="gateway.example"), server=NS(sni=None)), conn=NS(error=message, sni=None))
        self.addon.tls_failed_client(event("client disconnected during handshake"))
        self.assertIsNone(self.addon.health)
        self.addon.tls_failed_client(event("tlsv1 alert unknown ca"))
        self.assertEqual(self.read()["health"]["kind"], "certificate_client")
        self.addon.responseheaders(self.flow())
        self.assertEqual(self.read()["health"]["kind"], "quota_received")
