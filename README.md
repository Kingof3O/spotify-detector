# Stream Manager

`spotify-overlay` (Stream Manager) is a low-resource, local Windows desktop utility that mirrors Spotify's current Windows media session into a transparent, customizable OBS Browser Source. It runs entirely on the streaming PC:

```text
Spotify Desktop → Windows GSMTC events → Rust agent → localhost WebSocket → OBS
```

The optional chat/request integration connects a dedicated Twitch bot account and the Spotify Web API only when configured in the local setup dashboard. The OBS overlay itself uses Windows media events and does not require OAuth or cloud relays.

The Stream Manager can also optionally switch OBS scenes for League of Legends. Enable OBS WebSocket and League detection from `/setup`; the detector listens to Windows window events and keeps the configured game scene during the short client-return grace period. OBS Studio 28+ with WebSocket v5 is supported. This automation is disabled by default.

---

## ✨ Features

- **Zero-Poll Event-Driven Engine**: Listens directly to Windows Global System Media Transport Controls (GSMTC) events with near-zero CPU and memory usage.
- **Dynamic Artwork Palette**: Automatically derives stable, darkened dominant colors from the current album cover while keeping the OBS canvas transparent.
- **Seamless Long Title Marquee**: Smoothly loops long track titles with a duplicated marquee track and pause boundaries without clipping or shrinking text.
- **Modern Dark Gradient Purple Control Center**: Sleek, glassmorphic local dashboard for settings (`/setup`) and live system diagnostics (`/check`).
- **OBS Custom Browser Dock Optimized**: Fully responsive layout that automatically collapses into a clean single-column view for narrow OBS Browser Docks (350px–450px).
- **Stream-Safe Token Masking**: Client IDs and sensitive credentials feature interactive visibility toggles (👁 / 🔒) to prevent accidental on-stream leaks.
- **Twitch Bot & Spotify Song Requests**:
  - `!song` / `!np` — answers current track details in chat.
  - `!sr` / `!request <query/link>` — searches and queues songs directly into Spotify.
  - Customizable roles, user/global cooldowns, and categorized message templates with click-to-insert placeholder chips (`{user}`, `{track}`, `{artist}`, `{seconds}`).
- **Local Health Diagnostics Dashboard (`/check`)**: Real-time status pulse ring, service summary metric tiles (Server, Media Session, Twitch, Spotify), and 1-click remediation links.
- **Lightweight Windows System Tray**: Minimizes to the taskbar notification area with quick links to open the overlay or gracefully exit.
- **Optional League → OBS Automation**: Event-driven League game/client detection with configurable scenes, grace period, foreground policy, and OBS reconnect handling.

---

## 🖥️ Requirements

For the released executable:
- **Windows 10 (version 1809+) or Windows 11 (x64)**
- **Spotify Desktop**
- **OBS Studio** (with Browser Source support)

---

## 🚀 Quick Start & OBS Setup

1. Download and extract **`spotify-overlay-windows-x64.zip`** from the [Latest Release](https://github.com/Kingof3O/spotify-detector/releases).
2. Run **`spotify-overlay.exe`** (starts silently in the system tray).
3. Open Spotify Desktop and play a track.
4. In OBS Studio:
   - Add a **Browser Source**.
   - Set URL to: `http://127.0.0.1:18923/`
   - Set Dimensions to: **650 × 250**
   - Keep the custom CSS background transparent (default).

*(Optional)* **Add as an OBS Custom Browser Dock**:
1. In OBS, go to **Docks** → **Custom Browser Docks...**
2. Dock Name: `Spotify Setup`
3. URL: `http://127.0.0.1:18923/setup` (or `http://127.0.0.1:18923/check` for Diagnostics)
4. Click **Apply** and dock it anywhere in your OBS workspace.

---

## ⚙️ Optional Twitch & Spotify Integration

Right-click the system tray icon and select **Open Stream Manager Setup**, or open [http://127.0.0.1:18923/setup](http://127.0.0.1:18923/setup):

1. **Twitch Bot**: Enter your Twitch Developer Client ID, target channel, and authorize the bot via Device Code flow. Make the bot a moderator in your channel.
2. **Spotify API**: Enter your Spotify Developer Client ID, add the 1-click copied Redirect URI (`http://127.0.0.1:18923/auth/spotify/callback`) to your Spotify Developer Dashboard, and authorize.
3. **Commands & Customization**: Configure aliases (`!song`, `!sr`), roles (Everyone, Sub, VIP, Mod), cooldowns, and customizable message templates.

4. **League → OBS scenes (optional)**: Enable OBS WebSocket and League detection in the same dashboard. Set the Game, Client, and Idle scene names, choose a transition grace period, and optionally require League to be foreground. OBS Studio 28+ with WebSocket v5 is required; the OBS password is stored with Windows DPAPI and never written to `config.json`.

All sensitive tokens are stored locally and encrypted using **Windows DPAPI** (`%LOCALAPPDATA%\SpotifyOverlay`).

---

## 🌐 Local Endpoints

```text
GET http://127.0.0.1:18923/               # Transparent OBS Overlay
GET http://127.0.0.1:18923/setup          # Configuration Dashboard
GET http://127.0.0.1:18923/check          # Live Diagnostics & Health Checks
GET http://127.0.0.1:18923/test           # Visual Overlay Preview & Marquee Tester
GET http://127.0.0.1:18923/health         # JSON Health Status
GET http://127.0.0.1:18923/artwork        # Dynamic Track Artwork
WS  ws://127.0.0.1:18923/ws               # Realtime Media State WebSocket
```

---

## 🛠️ Development & Building

Rust is the only build dependency. All overlay HTML, CSS, JavaScript, and SVG assets are compiled directly into the binary using `include_str!`.

```bash
# Run local server
cargo run

# Run unit test suite
cargo test

# Build optimized release binary (Windows x64)
cargo build --release --target x86_64-pc-windows-msvc
```

On macOS or Linux development environments, double-click or run `scripts/test-mac.command` to spin up a mock server and inspect the visual overlay at `http://127.0.0.1:18923/test?long=1`.

---

## 📂 Project Architecture

```text
spotify-detector/
├── overlay/                      # Frontend UI & Assets
│   ├── theme.css                 # Shared design tokens, typography & card surfaces
│   ├── setup.html / setup.css    # Setup dashboard & responsive layout
│   ├── setup.js                  # OOP SetupController & reactive SettingsStore
│   ├── check.html / check.css    # Health diagnostics dashboard
│   ├── check.js                  # OOP HealthCheckController & DiagnosticRenderer
│   ├── index.html / overlay.css  # Production transparent OBS Overlay
│   └── overlay.js                # Overlay animation & WebSocket client
├── src/                          # Rust Backend
│   ├── app/                      # Main application loop & tray controls
│   ├── automation/               # League window events, state machine & OBS v5 adapter
│   ├── chat/                     # Twitch EventSub WebSocket & command processor
│   ├── config/                   # Local configuration & DPAPI credential storage
│   ├── integration/              # Twitch & Spotify OAuth API services
│   ├── media/                    # Windows GSMTC session monitor & artwork manager
│   ├── server/                   # Axum HTTP server & WebSocket dispatcher
│   └── main.rs                   # Entry point
└── scripts/                      # Testing & restart helper scripts
```

---

## 📄 License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
