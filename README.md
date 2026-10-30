# ollama-cosmic

[![CI](https://github.com/ashrafpy/ollama-cosmic/actions/workflows/ci.yml/badge.svg)](https://github.com/ashrafpy/ollama-cosmic/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built with libcosmic](https://img.shields.io/badge/COSMIC-libcosmic-63b3ed)](https://github.com/pop-os/libcosmic)

A native **COSMIC™ Desktop Panel Applet** (built with `libcosmic`) for Linux systems running local [Ollama](https://ollama.com) inference (`ollama.service`).

## Features

- **Minimalist Icon-Only Panel Indicator**: Displays the symbolic **outline** Ollama icon when idle (`0` models in memory) and automatically switches to a **filled** Ollama icon when one or more models are loaded in VRAM—keeping your COSMIC panel clean with zero text clutter.
- **Focused, Opaque Native Popup**: 100% solid COSMIC-themed popup and cards (zero background bleed-through) with a lightweight layout that collapses the `LOADED IN MEMORY` section completely when Ollama is idle.
- **Visual Token Usage & Live Stream Telemetry**:
  - Passively tails `journalctl -u ollama.service -o json` across all apps (`/api/chat`, `/api/generate`, `/v1/chat/completions`, `/v1/messages`).
  - **Today's Usage Stacked Bar**: Displays today's total tokens, request count, and live/last `tok/s` above a sleek `6px` proportional stacked bar and colored legend for **Input** (uncached), **Cached** (KV-cache reuse), and **Output** tokens.
  - **Daily Usage History (`History`)**: Slim `6px` horizontal stacked bar chart showing daily **Input**, **Cached**, and **Output** tokens alongside 7-day & all-time summaries, persisted across reboots and journal rotations in `~/.local/share/cosmic-applet-ollama/token_usage.json` using systemd journal cursors.
- **One-Click WLAN / LAN Endpoint Copy**:
  - Automatically detects your machine's active WLAN/LAN IPv4 address (e.g. `http://192.168.x.x:11434`) so you can copy it with one click to use on your laptop or other devices on your local network.
  - Inspects `/proc/net/tcp{,6}` to indicate whether `ollama.service` is listening on all interfaces (`WLAN Endpoint`) or localhost only (`WLAN (local bind)`).
- **Lightweight Loaded Model Card (`/api/ps` + `/api/tags` + `/api/show`)**:
  - Single-line hardware & context summary (`1.7B · Q4_K_M · 1.09 GiB VRAM · 4K ctx`)
  - Full-width single-line active sampling parameters (`temp 0.10 · top_p 1.00 · top_k 50 · rep 1.10`)
  - Live auto-unload countdown (`Unloads in 4m 33s` / `Pinned`) paired with highlighted `keep_alive` residency buttons (`5m`, `15m`, `1h`, `Pin ∞`) and one-click **Load** / **Unload**.
- **Interactive Parameter Configuration (`⚙`)**:
  - Click the compact `⚙` icon button on any loaded or installed model to adjust `temperature`, `top_p`, `top_k`, `repeat_penalty`, `num_ctx`, and `num_predict` via sliders and preset chips.
  - Update a model **in-place** (auto-reloading if currently in VRAM) or save as a **new custom tag** (e.g. `:coding`, sharing weights with 0 extra disk space) via `POST /api/create`.

## Prerequisites

- **COSMIC Desktop** (`cosmic-panel`)
- **[Ollama](https://ollama.com)** running locally (systemd `ollama.service` enables live journal token telemetry)
- **Rust toolchain** (`cargo` & `rustc` via [rustup.rs](https://rustup.rs) or your package manager) and native Wayland/`xkbcommon` development headers:
  - **Arch Linux**: `sudo pacman -S rust libxkbcommon wayland pkgconf`
  - **Pop!\_OS / Ubuntu / Debian**: `sudo apt install cargo pkg-config libwayland-dev libxkbcommon-dev`
  - **Fedora**: `sudo dnf install cargo pkgconf-pkg-config wayland-devel libxkbcommon-devel`
- *Optional*: `wl-clipboard` (`wl-copy`) for persistent Wayland clipboard fallback when copying the endpoint URL.

> **Note on Journal Telemetry Permissions:** If `journalctl -u ollama.service` produces no entries for your user account on your distribution, ensure your user belongs to the `systemd-journal` (or `adm`) group:
> ```bash
> sudo usermod -aG systemd-journal "$USER"
> ```

## Installation

### User Install (`~/.local`, no `sudo` required)

```bash
git clone https://github.com/ashrafpy/ollama-cosmic.git
cd ollama-cosmic
./install.sh
```

This builds the release binary, installs the binary, `.desktop` entry, and symbolic icons (outline + active filled) into `~/.local`, and automatically restarts any running `cosmic-applet-ollama` panel instance.

To add the applet to your COSMIC Panel:
1. Open **COSMIC Settings → Desktop → Panel → Configure Panel Applets**
2. Click **+ Add Applet** and select **Ollama Model Status**

### System-Wide / Packaging Install (`just`)

If you use [`just`](https://github.com/casey/just) or are packaging `cosmic-applet-ollama` for a distribution:

```bash
just build-release
sudo just install
```

## Configuration & Storage

- **`OLLAMA_HOST`**: Defaults to `http://127.0.0.1:11434`. Set `OLLAMA_HOST` in your session environment if your Ollama instance listens on a custom host or port. To expose `ollama.service` on your local Wi-Fi/LAN so the footer `WLAN Endpoint` URL is reachable from another device, set `Environment="OLLAMA_HOST=0.0.0.0:11434"` in `sudo systemctl edit ollama.service`.
- **Local Data**: Daily token counts and journal cursors are stored locally at `${XDG_DATA_HOME:-~/.local/share}/cosmic-applet-ollama/token_usage.json`. No external telemetry or network requests are ever made outside your configured `OLLAMA_HOST`.

## Uninstallation

```bash
./uninstall.sh
```

To also delete saved token usage history (`~/.local/share/cosmic-applet-ollama`), run:

```bash
./uninstall.sh --purge
```

*(If installed system-wide via `just`, run `sudo just uninstall`.)*

## Development

```bash
cargo check --all-targets
cargo test
cargo build --release
```

## License

Licensed under the [MIT License](LICENSE).
