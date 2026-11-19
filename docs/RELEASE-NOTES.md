Show My Limits — first public test build.

- macOS: Apple Silicon and Intel (.dmg and updater archive).
- Windows: x64 NSIS installer.
- Linux: x64 AppImage and Debian package.
- Signed in-app updates from GitHub Releases; installation requires a click.
- mitmproxy is installed separately by the connection wizard after a checksum-verified download.

macOS builds use ad-hoc signing and are not Apple-notarized. Approve the first launch for this specific app in macOS Privacy & Security if you trust the source. Windows builds do not have an Authenticode certificate.

Linux AppImage supports in-app installation of updates; Debian packages are updated manually. Local capture on Linux needs elevated privileges and a supported kernel. Windows/Linux capture and installation still need end-to-end validation on clean systems. Compilation is not a clean-install test.
