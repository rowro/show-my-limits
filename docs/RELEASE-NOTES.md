Show My Limits 0.3.2

- Windows: on computers where a policy forbids users to trust new root certificates, installing the certificate failed with "Access is denied". The app now explains the cause and offers a separate "Install for the computer" button that adds the same certificate to the computer's trusted roots after a UAC prompt. Without administrator rights, it shows the command to give to an administrator. The certificate still works only for the chosen server.
- The same option is available when replacing a certificate, and the automatic switch continues after it.
- `sml trust` on Windows prints the administrator command when the current user's store is refused.

The computer-wide install has not yet been tested on every Windows configuration. If it fails, run the shown command in PowerShell as administrator.

macOS packages remain ad-hoc signed without Apple notarization. Windows packages do not have Authenticode signing. Standalone CLI archives are separate from the GUI installers.
