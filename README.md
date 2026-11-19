# ollama-cosmic

[![CI](https://github.com/ashrafpy/ollama-cosmic/actions/workflows/ci.yml/badge.svg)](https://github.com/ashrafpy/ollama-cosmic/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built with libcosmic](https://img.shields.io/badge/COSMIC-libcosmic-63b3ed)](https://github.com/pop-os/libcosmic)

A native **COSMIC™ Desktop Panel Applet** (built with `libcosmic`) for Linux systems running local [Ollama](https://ollama.com) inference (`ollama.service`).

## Features

- **Minimalist Icon-Only Panel Indicator**: Displays the symbolic **outline** Ollama icon when idle (`0` models in memory) and automatically switches to a **filled** Ollama icon when one or more models are loaded in memory—keeping your COSMIC panel clean with zero text clutter.
- **Focused, Opaque Native Popup**: 100% solid COSMIC-themed popup and cards (zero background bleed-through) showing loaded models at the top, a compact token summary, installed models (auto-expanded when 3 or fewer, collapsible under `N more models` when more), and the API endpoint footer.
- **Token Usage & Live Stream Telemetry**:
  - Passively tails `journalctl -u ollama.service -o json` across all apps (`/api/chat`, `/api/generate`, `/v1/chat/completions`, `/v1/messages`).
  - **Compact Today Summary**: Displays today's total tokens (or live streaming token count) and `tok/s` in a single line alongside a **History** button.
  - **7-Day Usage History (`History`)**: Slim `6px` horizontal stacked bar chart showing daily **Input** (uncached), **Cached** (KV-cache reuse), and **Output** tokens over the last 7 days alongside an **All-time** summary, persisted across reboots and journal rotations in `${XDG_DATA_HOME:-~/.local/share}/cosmic-applet-ollama/token_usage.json` using systemd journal cursors.
- **One-Click Endpoint Copy**:
  - Displays the configured Ollama endpoint (`OLLAMA_HOST`, defaulting to `http://127.0.0.1:11434`) in the popup footer so you can copy it to the clipboard with one click.
- **Loaded Model Cards (`/api/ps` + `/api/tags` + `/api/show`)**:
  - Model name, `⚙` configure button, and one-click **Unload** (or **Load** on installed models).
  - Single-line hardware & memory summary (e.g. `1.7B · Q4_K_M · 1.09 GiB VRAM`, or RAM / `% VRAM` split).
  - Live auto-unload countdown (`4m 33s` / `∞`) paired with `keep_alive` residency buttons (`5m`, `15m`, `1h`, `∞`).
- **Interactive Parameter Configuration (`⚙`)**:
  - Click the compact `⚙` icon button on any loaded or installed model to adjust `temperature`, `top_p`, `top_k`, `repeat_penalty`, `num_ctx`, and `num_predict` via sliders, preset chips, and inputs.
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

This builds the release binary, installs the binary, `.desktop` entry, metainfo, and symbolic icons (outline + active filled) into `~/.local` (respecting `XDG_DATA_HOME`), and automatically restarts any running `cosmic-applet-ollama` panel instance.

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

- **`OLLAMA_HOST`**: Defaults to `http://127.0.0.1:11434`. Set `OLLAMA_HOST` in your session environment if your Ollama instance listens on a custom host or port.
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
