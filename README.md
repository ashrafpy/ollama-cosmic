# ollama-cosmic

[![CI](https://github.com/ashrafpy/ollama-cosmic/actions/workflows/ci.yml/badge.svg)](https://github.com/ashrafpy/ollama-cosmic/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built with libcosmic](https://img.shields.io/badge/COSMIC-libcosmic-63b3ed)](https://github.com/pop-os/libcosmic)

A native **COSMIC™ Desktop Panel Applet** (built with `libcosmic`) for Linux systems running local [Ollama](https://ollama.com) inference (`ollama.service`).

## Features

- **Official Ollama Icon + Active Model Status**: Displays the official Ollama logo + active model name directly on horizontal COSMIC panels (e.g. `lfm3`, `llama3.2:3b`, `lfm3 +1` when multiple models are loaded, `Idle` when 0 models are in memory, or `Offline` if `ollama.service` is stopped). Automatically adapts to icon-only mode on vertical docks/panels.
- **Opaque Native Popup**: 100% solid COSMIC-themed popup and cards (zero background bleed-through).
- **Per-Day Token Usage & Live Stream Telemetry**:
  - Passively tails `journalctl -u ollama.service -o json` across all apps (`/api/chat`, `/api/generate`, `/v1/chat/completions`, `/v1/messages`).
  - Displays **Today's Input Tokens** (including KV-cache reuse count), **Output Tokens**, **Total Today**, **Request Count**, and **Live / Last `tok/s`**.
  - **Daily Usage History (`History`)**: Clean **horizontal stacked bar chart** showing daily **Input** (uncached), **Cached**, and **Output** tokens alongside 7-day & all-time summaries, persisted across reboots and journal rotations in `~/.local/share/cosmic-applet-ollama/token_usage.json` using systemd journal cursors.
- **One-Click Endpoint Copy**: Click the `Endpoint` footer row (`http://127.0.0.1:11434`) to copy the active Ollama URL directly to the clipboard with instant `Copied!` feedback.
- **Model Specs & Modelfile Parameters (`/api/ps` + `/api/tags` + `/api/show`)**:
  - Architecture, parameter count, quantization, and runner (`lfm3 · 1.7B · Q4_K_M · llamacpp`)
  - Total memory & VRAM allocation + GPU/CPU placement split (`100% GPU`)
  - Context window (`4K (max 256K)`) & capabilities (`tools`, `thinking`, `vision`)
  - Active sampling parameters (`temp · top_p · top_k · rep`)
- **Interactive Parameter Configuration (`Configure`)**:
  - Sliders & preset chips for `temperature`, `top_p`, `top_k`, `repeat_penalty`, `num_ctx`, and `num_predict`.
  - Update a model **in-place** (auto-reloading if currently in VRAM) or save as a **new custom tag** (e.g. `:coding`, sharing weights with 0 extra disk space) via `POST /api/create`.
- **Residency & Model Controls**:
  - Quick `keep_alive` buttons (`5m`, `15m`, `1h`, `Pin ∞`) + live auto-unload countdown.
  - One-click **Load** and **Unload** (`keep_alive: 0`).

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

This builds the release binary, installs the binary, `.desktop` entry, and symbolic icon into `~/.local`, and automatically restarts any running `cosmic-applet-ollama` panel instance.

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
