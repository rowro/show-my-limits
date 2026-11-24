# Releases and updates

The public repository is `rowro/show-my-limits`. Run **Release builds** manually on the desired commit, or push a `vX.Y.Z` tag matching the versions in package.json, Cargo.toml, and tauri.conf.json. The workflow builds macOS arm64/Intel, Windows x64 and Linux x64/ARM64 using native GitHub runners.

All outputs go to a **draft** GitHub Release. A final job assembles one complete `latest.json` only after all five target builds succeed. Test the installers before publishing the draft as a stable release. The app reads `https://github.com/rowro/show-my-limits/releases/latest/download/latest.json`; draft and prerelease builds are not delivered by that endpoint. Increment the GUI and CLI versions together before the next release. The same five runners also build standalone `sml` archives; they are not included in the GUI updater manifest.

`TAURI_SIGNING_PRIVATE_KEY` in GitHub Actions Secrets signs updater packages. The public verification key is in tauri.conf.json. Keep a separate secure backup of the private key; losing it prevents updates to installations trusting that key. Never commit private keys, CA keys, settings, credentials or local state. The updater key is separate from Apple Developer ID and Windows Authenticode.

Local ordinary builds do not create updater artifacts. To create signed update packages locally, provide `TAURI_SIGNING_PRIVATE_KEY` and run `npm run tauri build -- --config src-tauri/tauri.release.conf.json`. The value can be an absolute path to the private key. Do not put the key in shell history as literal key contents.

The app checks once at release startup, and Settings has a manual check. Download and signature verification happen before capture is stopped. Installation is explicit and restarts the application; CA, settings and helper installation remain in the user data directory. If installation fails after capture stops, the error is shown and capture can be started manually. Linux AppImage is the initial supported self-update format; .deb users download the new package.

mitmproxy is not bundled into the application. The wizard installs a pinned official distribution into app data after verifying SHA-256. This avoids mixing helper architectures into cross-compiled macOS bundles and preserves its vendor-signed capture components.

macOS builds are ad-hoc signed without Apple notarization until a Developer ID is provided. This does not disable TLS or Gatekeeper. Windows builds are unsigned with respect to Authenticode. Linux local capture requires a supported kernel and privilege handling; verify this on a clean machine.

Before publication, smoke-test first installation, CA trust, capture start/stop, quota receipt, app quit, and an update from the preceding version on every supported OS. A successful build is not proof of capture permissions or certificate trust.

## CA and capture release gate

Do not publish a build with the one-year CA until an explicit, tested replacement path exists for users whose CA is approaching expiry. Test the following with disposable accounts or test machines; never reset a maintainer's live application data:

1. Fresh install on macOS, Windows and Debian/Ubuntu: create CA, verify key/certificate match and file permissions, authorize trust, restart the mock client, receive fresh quotas, and stop capture. Confirm no OS trust is added before the user's action.
2. Upgrade with an existing CA: verify its fingerprint and trust scope remain unchanged until the user starts replacement. On macOS include an old System/Admin CA; on Linux include a system-wide Debian CA. Check that the app warns about the wider scope.
3. Replacement: generate a different key in a separate private directory; trust the new CA explicitly; receive a fresh HTTPS response signed by it; then switch the active store. If trust or capture fails, verify the old CA and capture still work.
4. Cleanup: remove old trust by the exact fingerprint and only after a separate confirmation. Verify the old key is removed only after the trust-removal result is confirmed. Cancel the OS prompt and ensure the app reports unfinished cleanup instead of success.
5. Run GUI capture with `sml watch`, then exit the viewer: GUI capture must continue. Run standalone `sml run`, then stop it: the helper must exit. Check that both frontends use the same active CA and neither silently rewrites settings.
6. On graphical Debian 13, exercise the tray menu, left click, outside click, `zenity`/sudo prompt, cancel and retry, and a real local capture heartbeat. Test both X11 and Wayland where available. Verify the displayed server and the redacted diagnostic report on failure.
7. Reinstall/update from the preceding release and confirm user settings, CA and trust are preserved until the explicit replacement flow. Check AppImage updating separately from manual DEB upgrade.

Record OS, package architecture, exact build commit, trust scope and outcome for each case in the release review. CI and container tests do not replace these interactive checks.

Before publication, verify the draft locally using the maintainer's existing GitHub CLI authorization:

```sh
mkdir review
gh release download vX.Y.Z --pattern '*.app.tar.gz' --pattern '*.AppImage' --pattern '*setup.exe' --pattern latest.json --dir review
cargo run --locked --manifest-path tools/release-verifier/Cargo.toml -- src-tauri/tauri.conf.json review/latest.json review
```

The verifier checks all five payload signatures with the application public key and the same minisign library as Tauri; modified payloads must fail. The **Verify release artifacts** workflow keeps read-only permissions and can repeat this check after publication. GitHub hides drafts from that read-only workflow token; do not expand it to `contents:write` just to read a draft. Signature checks do not replace installer-permission or end-to-end update tests.
