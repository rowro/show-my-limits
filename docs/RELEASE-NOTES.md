Show My Limits 0.1.2 — test release.

- Match capture readiness by launch ID rather than launcher/worker PID; supports standalone mitmdump's child process.
- Allow cancellation while capture is starting; stop startup after 90 seconds without readiness.
- Cancel the owned Linux process group, including pending user-level authentication.
- Distinguish unavailable network routes from TLS certificate errors.
- Mock client v2 adds --ipv4 for a controlled IPv4-only TCP test while retaining TLS validation.

Includes 0.1.1's Linux dragging, sudo/zenity password prompt, platform-neutral setup text and certificate trust instructions in Setup and Settings.

The Ubuntu VM's direct HTTPS test passed; full captured requests and graphical startup remain under validation. Windows capture still needs an end-to-end test. Builds and tests do not establish clean-machine runtime support.

macOS remains ad-hoc signed without Apple notarization. Windows has no Authenticode signature. Linux AppImage supports in-app updates; DEB packages are installed manually.
