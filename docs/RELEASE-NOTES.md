Show My Limits 0.1.7

- Windows: helper launchers and workers are owned through a Windows Job Object. Stop, normal exit and abrupt app-owner termination clean up the owned process tree.
- The helper is created suspended and attached before running, so its worker cannot escape ownership during startup. Job setup failure aborts startup instead of running unmanaged capture.
- CA-initialization helpers use the same lifecycle control.
- Regression tests cover stop, drop, owner crash and preservation of an unrelated process. Tests do not install a packet driver or capture network traffic.
- Certificates, settings, tray rendering and GNOME integration are unchanged from 0.1.6.

Before installing, quit old app instances and manually end any previously identified orphaned helpers from older versions. This fixes helper lifetime; concurrent active capture with Throne TUN still needs a real-machine compatibility test.
