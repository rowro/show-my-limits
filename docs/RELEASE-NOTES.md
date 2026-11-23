Show My Limits 0.2.1

- The setup wizard is easier to read and prepares mitmproxy and the app certificate automatically. Certificate trust remains an explicit user action.
- On Windows, quota values now fit a vertical taskbar. The widget layout was adjusted after testing on a Windows machine.
- The update action now sits beside the version in the app footer. It shows when an update is available.
- Version probing for an existing mitmdump now uses an owned process tree, including on Windows.

Existing settings and certificates are preserved. Quit older app instances before installing the new version.

macOS packages remain ad-hoc signed without Apple notarization. Windows packages do not have Authenticode signing. Standalone CLI archives are separate from the GUI installers.
