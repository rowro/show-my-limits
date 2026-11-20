Show My Limits 0.1.5

- Linux/Windows: visible application logo without quota data; transparent percentage artwork includes the percent sign, without a frame or reserved label gap.
- Capture state is isolated per launch. Brief read errors retain a coherent sample; legacy workers cannot overwrite current-session data.
- GNOME: tolerate transient transport delays and change fallback tray visibility only on transitions.
- GNOME: install during first setup; update by button and finish enabling automatically after next login. A button opens GNOME's normal logout confirmation; no terminal commands needed.
- Windows: check the exact current CA in native Root/Disallowed stores and install it into Current User Root by button. No firewall or TLS verification changes.
- Mock client: show unverified peer issuer and certificate fingerprint when TLS validation fails.

GNOME interaction and Windows capture still need manual testing on the target systems. Existing CA and settings are preserved; legacy root proxy state is not used as a current quota sample.
