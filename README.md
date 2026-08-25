# Spotify OBS Overlay

`spotify-overlay` is a small Windows desktop utility that mirrors Spotify's current Windows media session into a transparent OBS Browser Source. It runs entirely on the streaming PC:

```text
Spotify Desktop → Windows GSMTC events → Rust agent → localhost WebSocket → OBS
```

The optional chat/request integration uses Twitch EventSub and the Spotify Web API only after you explicitly connect both accounts in the local setup page. The OBS overlay itself still uses Windows media events and does not require OAuth.

## What it does

- Watches Windows Global System Media Transport Controls instead of polling Spotify.
- Tracks Spotify session replacement, track changes, play/pause, seek, timeline corrections, artwork changes, Spotify startup, and Spotify exit.
- Keeps one canonical media state in Rust and sends the full state immediately to each WebSocket client.
- Keeps only the current artwork in memory and exposes it at `/artwork` with a versioned URL.
- Updates only when media events arrive; the browser does not run a continuous progress-rendering loop.
- Smoothly loops long track titles with a duplicated marquee track, pausing briefly at both ends without shrinking the text.
- Serves the overlay, health endpoint, artwork, and WebSocket from `127.0.0.1:18923`.
- Reconnects the OBS page with bounded exponential backoff when the native process restarts.
- Optionally connects a dedicated Twitch bot account locally and answers current-song commands.
- Optionally resolves Spotify track requests and adds them to the active Spotify playback queue.

The default overlay is a clean 650 × 250 broadcast layout: a floating square album cover, a single-line cream title, and a muted artist line on a compact rounded card. The card automatically derives a stable, darkened dominant color from the current artwork while the page around it stays transparent.

## Requirements

For the released executable:

- Windows 10 version 1809 or newer, or Windows 11
- x64 PC
- Spotify Desktop
- OBS Studio with Browser Source support

The GSMTC media APIs are Windows APIs. On non-Windows development hosts, the server and overlay can still be exercised, but the media monitor logs that Windows integration is unavailable.

## Use the release executable

1. Start `spotify-overlay.exe`.
2. Start Spotify Desktop and play a track.
3. In OBS, add a **Browser** source.
4. Set the URL to `http://127.0.0.1:18923/`.
5. Set the Browser Source size to **650 × 250** for the intended proportions.
6. Leave the browser source background transparent. Do not add a browser custom CSS background.

The release executable starts silently in the background on Windows; it does not keep a terminal window open. It adds a **Spotify OBS Overlay** icon to the notification area beside the clock (Windows may place it under the **Show hidden icons** arrow). Right-click the icon to:

- **Open overlay** — opens the local overlay preview in your default browser.
- **Stop Spotify Overlay** — gracefully stops the local server and exits the application.

Double-clicking the icon also opens the overlay preview. The tray component uses the native Windows message queue and does not poll, open a hidden browser, or add a heavyweight GUI framework.

Starting the executable again asks a running copy to stop cleanly, then the new copy takes over the local port. If an older or stuck version still prevents startup, double-click **`restart-and-check.cmd`** from the extracted release folder. It closes previous `spotify-overlay.exe` processes, starts a fresh copy, checks the `/health` endpoint, and prints either a clear success message or the latest diagnostic log entries.

Release builds keep a lightweight `spotify-overlay.log` beside the executable. The file is capped by rotating it at 1 MiB and is useful when the background application cannot start normally.
Fatal startup errors also appear in a Windows message box instead of failing silently.

To start it automatically with Windows, press **Win+R**, open `shell:startup`, and place a shortcut to `spotify-overlay.exe` in that folder.

The overlay hides itself when Spotify has no usable media metadata. When Spotify pauses, the card remains visible.

## Optional Twitch commands and Spotify requests

Right-click the notification-area icon and choose **Setup Twitch & Spotify**, or open [http://127.0.0.1:18923/setup](http://127.0.0.1:18923/setup). The setup page is local to the streaming PC.

1. Create your own Twitch application configured for Device Code flow and enter its Client ID.
2. Enter the Twitch channel name and authorize the separate bot account.
3. Make that bot account a moderator in the channel.
4. Create your own Spotify developer app, enter its Client ID, and add the displayed local callback URI to its redirect allowlist.
5. Authorize the Spotify account that is playing the stream music.
6. Enable Twitch commands and save.

The default commands are:

- `!song` or `!playingnow` — reports the current Windows media-session track.
- `!sr <song title, Spotify URL, or Spotify URI>` or `!songrequest ...` — searches Spotify and adds the selected track to Spotify’s native queue.

Command aliases, minimum role, per-viewer/global cooldowns, and every user-facing bot response are configurable on the setup page. Message templates support placeholders such as `{user}`, `{track}`, `{title}`, `{artist}`, `{seconds}`, and `{command}`. Requests require Spotify Premium and an active Spotify playback device. The app does not provide an editable queue or remove/reorder tracks already handed to Spotify.

Detailed Twitch and Spotify API failures are written to `spotify-overlay.log` beside the executable. Twitch viewers receive only the configured friendly error message; raw HTTP responses are never posted in chat.

Each streamer supplies their own Twitch and Spotify Client IDs. The app stores non-sensitive settings under `%LOCALAPPDATA%\SpotifyOverlay` and protects OAuth tokens with Windows DPAPI. No public relay or Nightbot URL is required; Nightbot cannot reach the local `127.0.0.1` service.

## Development

Rust is the only runtime dependency. Node/npm is not needed because the overlay is ordinary static HTML/CSS/JavaScript embedded into the executable at compile time.

```bash
cargo run
cargo test
cargo build --release
```

On macOS or any development machine, the Windows media integration is unavailable, but the real server and production overlay can still be tested with hard-coded media data. Double-click `scripts/test-mac.command`, or run:

```bash
chmod +x scripts/test-mac.command
./scripts/test-mac.command
```

The launcher starts `cargo run`, opens [http://127.0.0.1:18923/test?long=1](http://127.0.0.1:18923/test?long=1), and stops the server with Ctrl+C. The `/test` page uses the production overlay code, so the layout, artwork-driven color, track transition, and seamless long-title marquee can be checked without Spotify. Use a 650 × 250 browser window for the intended canvas size. Add `?title=All%20Of%20The%20Girls%20You%20Loved%20Before` to reproduce a specific title.

If Rust reports that no toolchain is configured, run `rustup default stable` once and start the launcher again.

To build the Windows release from a Windows development machine:

```powershell
cargo build --release --target x86_64-pc-windows-msvc
```

The resulting executable is under `target\x86_64-pc-windows-msvc\release\spotify-overlay.exe`.

The project uses the official `windows` Rust projection for `Windows.Media.Control` and keeps the Windows media monitor on a dedicated COM/MTA thread. The HTTP/WebSocket service uses Tokio and Axum.

## Optional configuration

Configuration is optional. If present, `config.json` is read from the executable directory (or the current working directory during development):

```json
{
  "bind_address": "127.0.0.1",
  "port": 18923,
  "source_mode": "spotify_only",
  "specific_app_user_model_id": null,
  "log_level": "info"
}
```

Supported source modes are:

- `spotify_only` — the default and recommended OBS source.
- `current_media_session` — follows the session Windows marks current.
- `specific_application` — follows the configured `specific_app_user_model_id`.

The bind address defaults to loopback. Keep it at `127.0.0.1` unless there is a deliberate reason to expose the service to another machine.

## Local endpoints

```text
GET http://127.0.0.1:18923/
GET http://127.0.0.1:18923/test
GET http://127.0.0.1:18923/health
GET http://127.0.0.1:18923/artwork
WS  ws://127.0.0.1:18923/ws
```

The WebSocket sends a flat JSON state message on connect and after event-driven updates. A typical message is:

```json
{
  "type": "state",
  "available": true,
  "source": "spotify",
  "title": "Track title",
  "artist": "Artist",
  "album": "Album",
  "duration_ms": 212000,
  "position_ms": 35000,
  "playing": true,
  "artwork_url": "/artwork?v=25",
  "artwork_version": 25,
  "timestamp": 1787086800000
}
```

When Spotify is unavailable, the message contains `"available": false` and no placeholder metadata.

## Reliability notes

- Normal updates come from `SessionsChanged`, `CurrentSessionChanged`, `MediaPropertiesChanged`, `PlaybackInfoChanged`, and `TimelinePropertiesChanged`.
- A 20-second recovery check exists only to recover from a lost Windows media manager/session service; it is not the normal update path.
- Session event handlers are removed when a session is replaced or disappears.
- Artwork is never continuously written to disk.
- The browser ignores incoming commands; V1 exposes a read-only state surface.

## Project layout

```text
src/
├── app/                 process wiring
├── config/              optional local JSON configuration
├── media/               canonical state, artwork, source selection, Windows monitor
└── server/              HTTP, WebSocket, and shared server state
overlay/                 embedded HTML/CSS/JavaScript
```
