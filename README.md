<p align="center">
	<img src="assets/logo.png" width="128" alt="tidal-rs logo">
</p>

<h1 align="center">tidal-rs</h1>

<p align="center">
	A lightweight, optimized alternative TIDAL client for Windows, written in Rust.
</p>

tidal-rs is an unofficial desktop client for [TIDAL](https://tidal.com). It does less
than the official app on purpose: your playlists, search, a queue, and a player that
answers instantly. No feed, no mixes, no recommendations page.

It is a native app built with [GPUI](https://www.gpui.rs) (no Electron, no web view):
a single executable of about 35 MB that starts in a moment and stays light on memory.

## Features

- **Lossless playback**: FLAC, up to Hi-Res when your subscription has it, with
	TIDAL's loudness normalization.
- **Fast**: a track starts in a fraction of a second, the next one is fetched ahead,
	and skipping five times lands exactly five tracks further.
- **One queue that makes sense**: a single "Add to queue" action. What you add plays
	next, in the order you added it, before the rest of the playlist.
- **Shuffle everywhere**: inside a playlist it picks from the playlist; outside, it
	continues with tracks similar to the one you were listening to.
- **Right click anywhere**: every track, including the one in the player bar, has the
	same menu: add to queue, add to playlist, go to album, go to artist, copy link.
- **Playlists**: create, rename, delete, reorder by drag and drop, sort by clicking a
	column (remembered per playlist).
- **Recently played**: the tracks you listened to outside your playlists, so you can
	find them again and save them.
- **Picks up where you left off**: track, position, queue, page and volume are
	restored after a restart. It never starts playing on its own.
- **Windows integration**: media keys, the system media overlay, and anything that
	talks to it (Stream Deck, etc.).
- **English and French**, following your Windows language.

## Install

1. Download `tidal-rs-setup.exe` from the [latest release](../../releases/latest)
	and run it. Windows may warn that the installer is unsigned: choose "More info",
	then "Run anyway". It installs for your user only (no administrator rights), adds
	a Start menu shortcut and, if you want, a desktop one.
2. Provide your TIDAL client credentials (see below). This is done once.
3. Start tidal-rs and log in once: click "Open the TIDAL login page", sign in, then
	copy the address of the "Oops" page you land on and paste it into tidal-rs.

To remove it, use "Installed apps" in Windows settings. A portable `tidal-rs.exe` is
also attached to each release if you prefer not to install anything.

You need a paid TIDAL subscription. Your login is kept in Windows Credential Manager.

### TIDAL client credentials

tidal-rs talks to TIDAL's private API, which requires the client id and secret of a
TIDAL app. They are not shipped with tidal-rs, neither in this repository nor in the
released executable: every user provides their own in
`%APPDATA%\tidal-rs\client.toml`:

```toml
client_id = "..."
client_secret = "..."
client_id_pkce = "..."
client_secret_pkce = "..."
```

The simplest way to get them is from the community library
[tidalapi](https://github.com/tamland/python-tidal), which carries a set. With
[Python](https://www.python.org/downloads/) installed, run these two commands in
PowerShell; the second one writes the file for you:

```powershell
pip install tidalapi
python -c "import os, tidalapi; c = tidalapi.Config(); d = os.path.join(os.environ['APPDATA'], 'tidal-rs'); os.makedirs(d, exist_ok=True); open(os.path.join(d, 'client.toml'), 'w').write(f'client_id = \"{c.client_id}\"\nclient_secret = \"{c.client_secret}\"\nclient_id_pkce = \"{c.client_id_pkce}\"\nclient_secret_pkce = \"{c.client_secret_pkce}\"\n'); print('OK')"
```

If the file is missing, tidal-rs shows its expected path on the login screen. The
`_pkce` pair is the one that unlocks FLAC; with only the first pair, TIDAL serves AAC.

## Shortcuts

| Key | Action |
| --- | --- |
| `Space` | Play / pause |
| `Ctrl K` | Search |
| `Q` | Add the selected track to the queue |
| Double click, or click the play icon of a row | Play |

## Build from source

Requirements: Windows 10 or 11, [Rust](https://rustup.rs) 1.98 or newer.

```
cargo build --release
```

The executable is `target\release\tidal-rs.exe`.

### Releases

Every push to `main` builds the executable and its installer
(`installer/tidal-rs.iss`, [Inno Setup](https://jrsoftware.org/isinfo.php)) and
publishes both as a GitHub release
(`.github/workflows/release.yml`). No credentials are involved.

### Development

```
cargo test
cargo clippy --all-targets -- -D warnings
```

`TIDAL_RS_DEMO=1` starts the app with canned data and no TIDAL connection, which is
handy for working on the interface.

## Disclaimer

tidal-rs is not affiliated with, endorsed by, or supported by TIDAL. It uses TIDAL's
private API, which is against TIDAL's terms of use: use it at your own risk. It does
not download or save music; it only streams what your subscription gives you access to.
