Show My Limits 0.1.1 — test release.

- Linux: move the panel by dragging its logo/title; keep the chosen position on resize.
- Linux: sudo can request network-component authentication through a zenity dialog. The GUI remains unprivileged; sudoers is unchanged.
- Ubuntu/Debian: copy certificate trust commands from the setup wizard or Settings. Trust confirmation is now a visible button.
- Distinguish the certificate's display name from its exported filename.
- Explain DNS, TLS, timeout and capture-permission errors without retaining raw traffic logs.
- Use platform-neutral helper-installation text.
- macOS arm64/Intel, Windows x64 and Linux x64/arm64 installers and signed updater packages.

Linux graphical authentication still needs validation on the target Ubuntu VM. DEB recommends zenity; AppImage users may need to install it separately. Linux requires a supported kernel and appropriate capture privileges. A successful build does not prove capture works on a clean machine.

macOS builds remain ad-hoc signed without Apple notarization; Windows builds do not have Authenticode signing. Linux in-app updates use AppImage; DEB updates are installed manually.
