# Working on Show My Limits

## Project

Tauri 2 tray application with a TypeScript/Vite frontend, Rust backend and a mitmproxy addon. The README is for end users: keep it short, current and free of development history. Keep contributor notes here and release procedures in `docs/RELEASING.md`.

- `src/main.ts`, `src/styles.css`: UI, setup wizard, settings and browser-only mocks.
- `src-tauri/src/engine.rs`: settings, owned capture process and state.
- `src-tauri/src/quota.rs`: quota parsing and freshness.
- `src-tauri/src/certificate.rs`, `macos_trust.rs`: CA generation and trust.
- `src-tauri/src/installer.rs`: pinned official helper downloads and SHA-256 verification.
- `src-tauri/src/updates.rs`: signed application updates.
- `tools/mock-claude/`: standalone Go process named `claude` for real HTTPS capture smoke tests without corporate accounts; see `docs/MOCK-CLIENT.md`. Do not install it over the real client or bypass TLS verification.
- `proxy/limits_addon.py`: quota header extraction; do not persist traffic.
- `src/assets/show-my-limits.svg`: shared logo source.

## Git identity

Use repository-local Git identity `rowro <rowro.dev@gmail.com>` for both author and committer. Do not change global Git settings or use a work email in this repository.

## Development and checks

Use Node.js 22.12+, Rust and the platform prerequisites for Tauri.

```sh
npm ci
npm run tauri dev
```

Browser UI development: `npm run dev` at `http://localhost:1420/`. The browser-only controls outside the app panel switch mock scenes and themes without running capture. Do not ship these controls in the native app.

```sh
npm test
python3 -m unittest discover -s proxy -v
python3 -m unittest discover -s tests -p 'test_proxy*.py' -v
npm run test:rust
npm run build
npm run check:release
```

Browser regression check: open `/tests/click-regression.html` on the dev server and hold Save during background refresh; one release must produce exactly one submission and preserve the pressed DOM node. Background status rendering must not replace unchanged controls or resize the panel during a pointer interaction.

Run checks appropriate to the change; documentation-only edits do not need a native rebuild. Compile and test on the target OS. Successful CI builds do not prove CA trust, capture privileges, clean installation or end-to-end updates.

## Data and compatibility

The bundle identifier is `io.github.rowro.showmylimits`; Tauri resolves application data paths from it. Keep product naming consistent across the UI, installers and metadata. The initial identifier change deliberately starts with a fresh data directory; do not copy old CA keys or settings implicitly. Never silently replace an existing CA in the current data directory.

The app owns its settings and capture process. Do not inspect or modify Claude configuration, files or logs. Do not change system proxy settings or stop unrelated proxy processes. Do not reset the user's running app data as part of routine development.

Persist only quota state and limited diagnostics. Never commit CA private keys, updater private keys, credentials or personal app state. CA presence on disk or a saved completion flag does not prove current trust; macOS readiness must reflect actual system trust. Keep certificate authorization as an explicit user action.

## UI conventions

The undecorated panel opens from the tray and hides on blur; no tabs. macOS remains non-draggable. On Linux the brand area starts native dragging; keep buttons interactive and preserve manual placement across resizes/opens because tray geometry is not reliable under Wayland. Linux fallback and Windows use transparent percentage artwork including %, or a visible application mark without data; no empty icon/label gap or framed background. Native GTK drag input, rather than WebView mouse-up, releases the hide-on-blur guard. Verify both dragging and the next outside click on Ubuntu. Settings toggle through the gear. Use dynamic content height, compact spacing and all three themes. Until setup is complete show the wizard; after capture starts show quick-start instructions until a fresh response arrives. Default processes are `Claude` and `claude`, plus optional process names. The current UI configures one server; preserve existing stored data when changing the UI.

## Distribution

See `docs/RELEASING.md` for signing keys, workflow and publishing. The release workflow builds macOS arm64/Intel, Windows x64 and Linux x64/ARM64 into a draft release, then assembles the complete updater manifest. Do not publish a draft without completing the agreed release review.

Updater signatures are separate from OS application signing and the interception CA. Current macOS builds are ad-hoc signed without Apple notarization. Linux AppImage supports in-app updates; DEB users install a new package manually. Verify downloaded update signatures before stopping capture or installing anything.

Ordinary local builds do not require the updater private key. Official helper downloads are installed by the wizard; the release bundles do not include a developer's local helper or CA. Changing the pinned helper version requires updating archive names and SHA-256 hashes in `installer.rs`.

Linux testing: the macOS mock capture was confirmed by the user. Ubuntu ARM64 installed after fixing guest DNS; helper download and CA creation progressed, but capture exited and full Linux capture is not verified. The pinned mitmproxy uses sudo/root for its network component; GUI startup now sets SUDO_ASKPASS to the packaged zenity prompt, used only by the vendor sudo invocation; this needs a real Ubuntu smoke test. Passwords go directly to sudo and must never enter app logs or state. Do not claim a certificate fixes capture permissions or run the whole GUI as root. Capture diagnostics retain fixed categories only, never raw subprocess logs. Certificate trust instructions remain available in Settings after setup.

Capture readiness: standalone mitmdump may be a PyInstaller launcher plus Python worker. Match state by a per-launch SML_RUN_ID with a live owned launcher and fresh heartbeat, not by equality of launcher and worker PID. Cancel remains available while starting; startup has a 90-second deadline. Linux launches its own process group for cancellation.

Ubuntu VM diagnostics: direct mock HTTPS succeeds, but local capture logged Network is unreachable with IPv6 clients. Treat this as a routing diagnosis, not certificate rejection. Client v2 has opt-in --ipv4 to isolate IPv4 TCP without changing system IPv6 or disabling TLS. IPv4 capture and the final GUI path still require user verification.

GNOME integration: optional GNOME 46 extension in integrations/gnome, installed explicitly from Settings. Native popup on left click and context menu on right. DesktopState projects quota-only data to the user session bus; never expose AppStatus/settings/paths wholesale. A protocol-v1 heartbeat lease hides the fallback tray only while live, restoring it within 16 seconds after crash (explicit disable detaches immediately). Tolerate brief transport timeouts and only toggle tray visibility on transitions. No silent extension updates or global GNOME settings writes. Release CI smoke-tests the real bridge under Xvfb; actual Shell interaction still needs manual VM testing. See docs/GNOME.md.

Capture state files are per launch under proxy/runs/<run-id>/state.json, never the shared legacy proxy/state.json. Keep the last coherent same-run sample on short reads; its heartbeat expiry and owned launcher check still determine readiness. Never import legacy quota data as a fresh sample.
GNOME install is automatic at first wizard completion (GNOME 46), or explicit by Settings. A one-shot user autostart + app-owned pending marker enables it after next session; clear both after a healthy extension heartbeat. Never force logout; the UI opens normal GNOME confirmation. No terminal commands required in setup.
Windows certificate readiness compares exact CA DER against Current User/Local Machine Root, with Disallowed taking precedence. Installation by explicit button writes only the app CA to Current User Root. Store-matcher tests use ephemeral memory stores and do not alter machine/user trust. Never disable firewall or certificate verification to fix TLS.
