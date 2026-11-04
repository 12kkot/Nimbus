<p align="center">
  <img src="native/assets/icon.png" width="96" alt="Nimbus logo">
</p>

<h1 align="center">Nimbus</h1>

<p align="center">
  An unofficial Discord client for Windows, written in Rust. No browser inside.
</p>

<p align="center">
  <a href="https://github.com/debugown/Nimbus/releases/latest"><img alt="Latest version" src="https://img.shields.io/github/v/release/debugown/Nimbus?style=flat-square&color=white&labelColor=black&label=version"></a>
  <img alt="Windows" src="https://img.shields.io/badge/Windows-10%20%2F%2011-white?style=flat-square&labelColor=black">
  <img alt="Rust and Slint" src="https://img.shields.io/badge/Rust-Slint-white?style=flat-square&labelColor=black">
  <img alt="GPL-3.0 license" src="https://img.shields.io/badge/license-GPL--3.0-white?style=flat-square&labelColor=black">
</p>

<p align="center">
  <a href="https://github.com/debugown/Nimbus/releases/latest"><b>Download</b></a>
</p>

> [!WARNING]
> Nimbus is not affiliated with Discord Inc. Third-party clients violate the [Discord Terms of Service](https://discord.com/terms), and your account may be banned. Use it at your own risk.

## Features

**Chat**
- Servers, channels, direct messages, friends, threads and forums.
- Replies, reactions, pinned messages, forwarding and search.
- Mentions show names and role colours. A click on a name opens the profile card.
- Full profiles with badges, connections, notes, mutual friends and mutual servers.
- A full right-click menu for people and DMs. Moderator actions follow your real permissions.
- Group chats with a member list. Opening a server or the DM side returns you to the chat you left.
- Stickers, GIFs, animated emoji, polls and voice messages.
- Paste screenshots and files with Ctrl+V and preview them before sending.
- Open a picture at almost full window size.

**Voice and video**
- Voice channels, DM calls, camera and screen sharing.
- Watch several streams at once in a grid, each with its own sound. One click makes a stream big, another returns to the grid.
- Picture-in-picture you can drag anywhere in the window.
- RNNoise noise suppression that runs on your computer.
- Soundboard and a volume setting for each person.

**Bots and servers**
- Slash commands, bot buttons and select menus.
- Server events: the list, "Interested" and creating an event.
- Invites and notification settings for each server and channel.

**Settings**
- Privacy, status, "About me", activity and a profile editor.
- Black, graphite, midnight and light themes.
- Resizable side panels.
- Interface in English, Russian, Ukrainian, Spanish, German, French, Portuguese (Brazil), Polish and Turkish. Translations have not been reviewed by native speakers yet.

Some of the newer features have had little testing on real accounts. If something doesn't work, tell us what you see.

## Install

1. Download `Nimbus-Setup-<version>.exe` from the [releases page](https://github.com/debugown/Nimbus/releases/latest).
2. Run the installer. It doesn't need administrator rights.
3. Sign in with discord.com. The token is saved only on this computer.

## Updates

At start, Nimbus looks for a new version on GitHub and installs it. It checks the ed25519 signature and the SHA-256 hash first, so a swapped file won't run. Without a connection, the current version opens.

While running, it checks every hour and shows a green button in the title bar when an update is ready. After an update, it says so and shows what changed right in the app. The same list is in Settings, under About.

## Logs

Nimbus writes no log file by default. To look into a problem, set the `NIMBUS_LOG` environment variable before starting it. The log is then written to `%TEMP%\nimbus-native.log`.

## Build

You need Windows and [Rust](https://rustup.rs) with the MSVC toolchain.

```powershell
cd native
cargo build --release
cargo test --release
```

The first full build takes about 15 minutes. You'll find `nimbus-native.exe` in `target\release`.

Demo mode shows a window with made-up data and sends nothing:

```powershell
nimbus-native.exe --demo
```

## License

[GPL-3.0-or-later](https://www.gnu.org/licenses/gpl-3.0.html). The project contains no Discord code, icons or texts. A modified copy of the songbird voice library is in `native/vendor/songbird` under the ISC license.
