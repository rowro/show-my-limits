Show My Limits 0.2.0

- Settings now have compact General, Capture and Updates tabs. Switching preserves unsaved fields.
- New standalone sml terminal utility for macOS, Windows and Linux: setup, explicit CA trust, capture, live viewing, JSON status and diagnostics.
- GUI and CLI reuse the same settings, pinned mitmproxy installation and certificate. OS locks prevent simultaneous capture or setup; viewers do not stop another owner's capture.
- Windows taskbar widget and tray text improvements from the Windows development session are included.
- Existing certificates and settings are preserved. Quit older app instances before using the new GUI or CLI.

macOS packages remain ad-hoc signed without Apple notarization. CLI archives are separate from GUI installers; CLI updates by replacing its executable.
