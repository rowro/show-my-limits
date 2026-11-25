Show My Limits 0.3.1

- Replacing a certificate from an older version is now one button on the main screen. The app creates the new certificate and asks the system to trust it. It then checks the new certificate with a test capture and switches to it after a fresh response from Claude. Restart Claude Desktop and Claude Code sessions when asked.
- The old certificate is removed with a "Delete old certificate" button. Only the exact certificate is removed from the system trust stores, after one system prompt (password on macOS, confirmation or UAC on Windows, sudo on Debian/Ubuntu). Its key is deleted after the app re-checks that nothing still trusts it.
- Replacement is offered only when it is needed: a certificate from an older version, a certificate issued for a different server, or one that expires within 30 days.
- An untrusted certificate left by an unfinished older installation is now cleaned up automatically during setup.
- On Linux the app shows the trust commands for the new certificate and continues on its own once they have been run.

Removing the old certificate has not yet been tested on every OS. If the button reports an error, the old certificate stays in place and can be removed manually by the fingerprint shown in Settings.

macOS packages remain ad-hoc signed without Apple notarization. Windows packages do not have Authenticode signing. Standalone CLI archives are separate from the GUI installers.
