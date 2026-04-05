---
title: Install Neo Zed - macOS, Linux, Windows
description: Download and install Neo Zed on macOS, Linux, or Windows. Includes Homebrew, direct download, and package manager options.
---

# Installing Neo Zed

## Download Neo Zed

### macOS

Get the latest stable builds via [the download page](https://neozed.dev/download). If you want to download the preview build, you can find it on the [preview releases page](https://neozed.dev/releases/preview). After the first manual installation, Neo Zed will periodically check for updates.

You can also install Neo Zed stable via Homebrew:

```sh
brew install --cask neozed
```

For the preview build:

```sh
brew install --cask neozed@preview
```

### Windows

Get the latest stable builds via [the download page](https://neozed.dev/download). If you want to download the preview build, you can find it on the [preview releases page](https://neozed.dev/releases/preview). After the first manual installation, Neo Zed will periodically check for updates.

Additionally, you can install Neo Zed using winget:

```sh
winget install -e --id dev.neozed
```

For the preview build:

```sh
winget install -e --id dev.neozed.Preview
```

### Linux

For most Linux users, the easiest way to install Neo Zed is through our installation script:

```sh
curl -f https://neozed.dev/install.sh | sh
```

You can now optionally specify a **version** of Neo Zed to install using the `ZED_VERSION` environment variable:

```sh
# Install the latest stable version (default)
curl -f https://neozed.dev/install.sh | sh

# Install a specific version
curl -f https://neozed.dev/install.sh | ZED_VERSION=0.216.0 sh
```

To install the preview build, which receives updates about a week ahead of stable:

```sh
curl -f https://neozed.dev/install.sh | ZED_CHANNEL=preview sh
```

This script supports `x86_64` and `aarch64`, as well as common Linux distributions: Ubuntu, Arch, Debian, RedHat, CentOS, Fedora, and more.

If Neo Zed is installed using this installation script, it can be uninstalled at any time by running the shell command `neozed --uninstall`. The shell will then prompt you whether you'd like to keep your preferences or delete them. After making a choice, you should see a message that Neo Zed was successfully uninstalled.

If this script is insufficient for your use case, you run into problems running Neo Zed, or there are errors in uninstalling Neo Zed, please see our [Linux-specific documentation](./linux.md).

## System Requirements

### macOS

Neo Zed supports the following macOS releases:

| Version       | Codename | Apple Status   | Neo Zed Status      |
| ------------- | -------- | -------------- | ------------------- |
| macOS 26.x    | Tahoe    | Supported      | Supported           |
| macOS 15.x    | Sequoia  | Supported      | Supported           |
| macOS 14.x    | Sonoma   | Supported      | Supported           |
| macOS 13.x    | Ventura  | Supported      | Supported           |
| macOS 12.x    | Monterey | EOL 2024-09-16 | Supported           |
| macOS 11.x    | Big Sur  | EOL 2023-09-26 | Partially Supported |
| macOS 10.15.x | Catalina | EOL 2022-09-12 | Partially Supported |

The macOS releases labeled "Partially Supported" (Big Sur and Catalina) do not support screen sharing via Neo Zed Collaboration. These features use the [LiveKit SDK](https://livekit.io) which relies upon [ScreenCaptureKit.framework](https://developer.apple.com/documentation/screencapturekit/) only available on macOS 12 (Monterey) and newer.

#### Mac Hardware

Neo Zed supports machines with Intel (`x86_64`) or Apple (`aarch64`) processors that meet the above macOS requirements:

- MacBook Pro (Early 2015 and newer)
- MacBook Air (Early 2015 and newer)
- MacBook (Early 2016 and newer)
- Mac Mini (Late 2014 and newer)
- Mac Pro (Late 2013 or newer)
- iMac (Late 2015 and newer)
- iMac Pro (all models)
- Mac Studio (all models)

### Linux

Neo Zed supports 64-bit Intel/AMD (`x86_64`) and 64-bit Arm (`aarch64`) processors.

Neo Zed requires a Vulkan 1.3 driver and the following desktop portals:

- `org.freedesktop.portal.FileChooser`
- `org.freedesktop.portal.OpenURI`
- `org.freedesktop.portal.Secret` or `org.freedesktop.Secrets`

### Windows

Neo Zed supports the following Windows releases:

| Version                            | Neo Zed Status |
| ---------------------------------- | -------------- |
| Windows 11, version 22H2 and later | Supported      |
| Windows 10, version 1903 and later | Supported      |

A 64-bit operating system is required to run Neo Zed.

#### Windows Hardware

Neo Zed supports machines with x64 (Intel, AMD) or Arm64 (Qualcomm) processors that meet the following requirements:

- Graphics: A GPU that supports DirectX 11 (most PCs from 2012+).
- Driver: Current NVIDIA/AMD/Intel/Qualcomm driver (not the Microsoft Basic Display Adapter).

### FreeBSD

Not yet available as an official download. Can be built [from source](./development/freebsd.md).

### Web

Not supported at this time.
