# Releases and updates

The public repository is `rowro/show-my-limits`. Run **Release builds** manually on the desired commit, or push a `vX.Y.Z` tag matching the versions in package.json, Cargo.toml, and tauri.conf.json. The workflow builds macOS arm64/Intel, Windows x64 and Linux x64 using native GitHub runners.

All outputs go to a **draft** GitHub Release. A final job assembles one complete `latest.json` only after all four target builds succeed. Test the installers before publishing the draft as a stable release. The app reads `https://github.com/rowro/show-my-limits/releases/latest/download/latest.json`; draft and prerelease builds are not delivered by that endpoint. Increment all three version fields before the next release.

`TAURI_SIGNING_PRIVATE_KEY` in GitHub Actions Secrets signs updater packages. The public verification key is in tauri.conf.json. Keep a separate secure backup of the private key; losing it prevents updates to installations trusting that key. Never commit private keys, CA keys, settings, credentials or local state. The updater key is separate from Apple Developer ID and Windows Authenticode.

Local ordinary builds do not create updater artifacts. To create signed update packages locally, provide `TAURI_SIGNING_PRIVATE_KEY` and run `npm run tauri build -- --config src-tauri/tauri.release.conf.json`. The value can be an absolute path to the private key. Do not put the key in shell history as literal key contents.

The app checks once at release startup, and Settings has a manual check. Download and signature verification happen before capture is stopped. Installation is explicit and restarts the application; CA, settings and helper installation remain in the user data directory. If installation fails after capture stops, the error is shown and capture can be started manually. Linux AppImage is the initial supported self-update format; .deb users download the new package.

mitmproxy is not bundled into the application. The wizard installs a pinned official distribution into app data after verifying SHA-256. This avoids mixing helper architectures into cross-compiled macOS bundles and preserves its vendor-signed capture components.

macOS builds are ad-hoc signed without Apple notarization until a Developer ID is provided. This does not disable TLS or Gatekeeper. Windows builds are unsigned with respect to Authenticode. Linux local capture requires a supported kernel and privilege handling; verify this on a clean machine.

Before publication, smoke-test first installation, CA trust, capture start/stop, quota receipt, app quit, and an update from the preceding version on every supported OS. A successful build is not proof of capture permissions or certificate trust.
