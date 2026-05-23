# Packaging

The hook installer is part of the binary:

```sh
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

macOS and Linux should use a user-level installer or call the hook commands during
first run. System package hooks such as `postinst` or `postinstall` often run as
root, so they should not edit `~/.claude/settings.json` unless the target user home
is known. The scripts in `packaging/unix/` install into `$HOME/.local` and update
Claude Code hooks for the current user.

```sh
cargo build --release
sh packaging/unix/install-user.sh
sh packaging/unix/uninstall-user.sh
```

Current platform support:

- Windows: full desktop pet UI, hook/proxy servers, settings panel, and
  permission/choice overlays.
- macOS/Linux: headless hook/proxy servers and hook install/uninstall CLI.
  Desktop interaction UI is not available yet, so permission requests are denied
  immediately instead of waiting for buttons that do not exist.
