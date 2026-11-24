Show My Limits 0.3.0

- New certificates work only for the server you choose. The setup wizard asks for the server first and then creates a certificate authority limited to that server. Its private key is never saved, so stolen app files cannot be used to impersonate other sites.
- Certificate trust is added for the current user only. Settings show the trust scope and warn about certificates from older versions, which can be replaced explicitly with a step-by-step flow.
- When upgrading from a version whose setup was never finished, an untrusted or incomplete old certificate is set aside instead of being offered for trust. Its key is removed by a separate cleanup action.
- On macOS, capture start now explains when macOS is waiting for you to allow the Mitmproxy Redirector network extension, with a button that opens System Settings. Start no longer times out while that approval is pending.
- After trusting a certificate, restart Claude Desktop and open Claude Code sessions: running processes do not see the new certificate.
- The main screen shows the selected server, which helps when sharing a screenshot.
- Linux: fixed a focus race that could close the panel right after opening it from the tray menu. The DEB package now installs `sml` and depends on `sudo` and `zenity`.
- Diagnostics: GUI and `sml doctor --share` produce a report without server names, paths or traffic.

Existing settings and certificates are preserved until you replace the certificate. Quit older app instances before installing the new version.

macOS packages remain ad-hoc signed without Apple notarization. Windows packages do not have Authenticode signing. Standalone CLI archives are separate from the GUI installers.
