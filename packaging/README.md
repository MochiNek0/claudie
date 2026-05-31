# Packaging

The hook installer is part of the binary:

```powershell
claudie --install-claude-hooks --quiet
claudie --uninstall-claude-hooks --quiet
```

Windows installers can call those commands directly because the installer runs in the
interactive user's profile. The Inno Setup script in `packaging/windows/claudie.iss`
installs the binary and bundled GIF pet assets, adds hooks during install,
and removes claudie hooks during uninstall.

Build the Windows installer:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

This produces `dist\claudie-setup.exe`. End users only need to double-click that
installer and follow the wizard.

claudie is Windows-only. Non-Windows installers and headless service packages are
not supported.
