---
title: Neo Zed on Windows
description: "Download Neo Zed for Windows, install it with winget, and troubleshoot common setup issues."
---

# Neo Zed on Windows

## Installing Neo Zed

Get the latest stable builds via [the download page](https://neozed.dev/download). If you want to download the preview build, you can find it on the [preview releases page](https://neozed.dev/releases/preview). After the first manual installation, Neo Zed will periodically check for updates.

You can also build Neo Zed from source. See the [Windows development documentation](./development/windows.md) for instructions.

### Package managers

Additionally, you can install Neo Zed using winget:

```sh
winget install -e --id dev.neozed
```

For the preview build:

```sh
winget install -e --id dev.neozed.Preview
```

## Uninstall

- Installed via installer: Use `Settings` → `Apps` → `Installed apps`, search for Neo Zed, and click Uninstall.
- Built from source: Remove the build output directory you created (for example, your `target/install` folder).

Your settings and extensions live in your user profile. When uninstalling, you can choose to keep or remove them.

## Remote Development (SSH)

Neo Zed supports remote development on Windows through both SSH and WSL. You can connect to remote servers via SSH or work with files inside WSL distributions directly from Neo Zed.

For detailed instructions on setting up and using remote development features, including SSH configuration, WSL setup, and troubleshooting, see the [Remote Development documentation](./remote-development.md).

## Troubleshooting

### Neo Zed fails to start or shows a blank window

- Check that your hardware and operating system version are compatible with Neo Zed. See our [installation guide](./installation.md) for more information.
- Update your GPU drivers from your GPU vendor (Intel/AMD/NVIDIA/Qualcomm).
- Ensure hardware acceleration is enabled in Windows and not blocked by third-party software.
- Try launching Neo Zed with no extensions or custom settings to isolate conflicts.

### Terminal issues

If activation scripts do not run, update to the latest version and verify your shell profile files are not exiting early. For Git operations, confirm Git Bash or PowerShell is available and on PATH.

### SSH remoting problems

When prompted for credentials, use the graphical askpass dialog. If it does not appear, check for credential manager conflicts and that GUI prompts are not blocked by your terminal.

### Graphics issues

#### Neo Zed fails to open or shows degraded performance

Neo Zed requires a DirectX 11-compatible GPU to run. If Neo Zed fails to open, your GPU may not meet the minimum requirements.

To check whether your GPU supports DirectX 11, run the following command:

```sh
dxdiag
```

This opens the DirectX Diagnostic Tool, which shows the DirectX version your GPU supports under `System` → `System Information` → `DirectX Version`.

If you're running Neo Zed inside a virtual machine, it will use the emulated adapter provided by your VM. Neo Zed can still run in this environment, but performance may be degraded.
