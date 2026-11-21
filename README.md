<p align="center">
  <img src="https://img.shields.io/badge/ShulkDS-v1.0.0-6B428A" alt="ShulkDS v1.0.0">
  <img src="https://img.shields.io/badge/Minecraft-Java%2026.3-5D9C3F" alt="Minecraft Java 26.3">
  <img src="https://img.shields.io/badge/Rust-stable-B7410E?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/platform-Android%20Dual--Screen-2A2470" alt="Android Dual-Screen">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-LGPL--3.0%20%2F%20MIT-3B2F7E" alt="LGPL-3.0 / MIT license"></a>
</p>

# ShulkDS

**ShulkDS v1.0.0** is an open-source dual-screen Android fork of [Horizon](https://github.com/horizon-rust/horizon) and [Starling](https://github.com/starling-client/starling) — the clean-room Rust rewrite of *Minecraft: Java Edition 26.3* — tailored for dual-screen clamshell Android handhelds such as the **Retroid Pocket Duo** (`1920x1080` 16:9 top + `1280x960` 4:3 touch bottom) and **AYN Thor** (`1920x1080` top + `1240x1080` bottom).

> **No Minecraft files included.** Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft. ShulkDS ships **no** Minecraft `.jar` files, assets, textures, or decompiled code: on first launch you sign in with a Microsoft account that owns *Minecraft: Java Edition*, and ShulkDS downloads and prepares official 26.3 directly on your device.

## Highlights in v1.0.0

- **120 Hz Dual-Surface Vulkan Rendering:** Dedicated `SecondaryDisplay` Android `Presentation` + `SurfaceView` driving the lower touchscreen on a shared `wgpu::Device` (`Rgba8Unorm`/`Rgba8UnormSrgb`), triple-buffered hardware `Fifo` VSync on Display 0, on-demand `ADPF` (`APerformanceHint`) frame boost, and decoupled bottom-screen HUD buffers.
- **3-Tab Touch Dashboard (Bottom Screen):**
  - **Map:** Live terrain minimap with zoom controls (`0.5x`–`4x`), 8-way compass, `DAY: N` and in-game clock, `X`/`Y`/`Z` coordinate chips, biome card, and live `FPS` / `FPS (FG)` readout.
  - **Inv:** Full touch backpack, `1x4` armor + auto-equip 3D player doll + offhand, Minecraft-style rarity hover tooltip, and recessed `Quick`/`Split`/`Drop`/(`Sort`|`Clear`) action tray.
  - **Opt:** Quick sliders (`Render Distance`, `FOV`, `Brightness`, `Volume`), `FSR` (`OFF`/`UltraQ`/`Quality`/`Balanced`/`Perf`), `Frame Gen` (`OFF`/`2x`), `Reach-Around`, and `Camera` toggles.
  - **Flanking Device Telemetry & IME:** Translucent local device clock and pixel battery indicator beside the persistent hotbar, plus bottom-screen Android soft keyboard routing (`TextMirror`) that preserves top-screen gameplay focus.
- **1:1 Legacy4J Console Edition Top-Screen GUIs:** Authentic Legacy Console Edition crafting (8 category tabs, 110 sections covering all 1,202 26.3 recipes, horizontal scrollbar, variant popouts), stonecutter, merchant trading, Creative catalog, containers, and workstations with full D-pad/stick controller navigation.
- **3DS-Inspired Dual-Screen Title Hub:** Panorama showcase + interactive 3D skin avatar on top (rotatable via touch or Right Stick) paired with dirt-backdrop touch/D-pad world, server, creation, and account screens on the bottom, requiring Microsoft sign-in on first setup.
- **Integrated Distant Horizons LOD Terrain:** Built-in Rust port (`0.1.0`, LGPL-3.0) supporting up to `1024`-chunk view distances, Adreno `MultiDrawIndirect`, early-Z opaque culling, 600 ms Bayer chunk/LOD crossfading, and internal app cache storage (`dh-cache`) to avoid `/sdcard` FUSE overhead.
- **Single-Pass FSR 1.0 & Mobile 2x Reprojection FrameGen:** Lightweight mobile spatial upscaling (Catmull-Rom + 75% RCAS) and 1-pass depth-reprojected `Frame Gen: 2x` tuned for handheld GPUs.
- **Controller Polish:** Controlify-inspired `LookCurve`, `ReachAround`, 8-slice quick-action `RadialMenu`, `DPadDown` smooth hold-to-zoom, and `DPadLeft` chat shortcut.

## Install (`v1.0.0`)

1. Download **`shulkds-1.0.0-arm64-v8a.apk`** from the [Releases page](https://github.com/NaiSenshin/ShulkDS/releases/latest) (package ID `dev.shulkds.client`).
   - *Upgrading from a debug-signed 0.8.0 test APK?* Uninstall the debug build once before installing the release-signed `1.0.0` APK (your worlds, packs, and settings in `/sdcard/ShulkDS` stay intact).
2. Open ShulkDS on your device, grant storage access if prompted so your `ShulkDS` folder appears in Android's Files app, and sign in with your Microsoft account owning *Minecraft: Java Edition* to prepare 26.3 resources on-device.

## Build from Source

Requires Rust (`1.90+`) with target `aarch64-linux-android`, [`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk), Android SDK (`platforms;android-36`, `build-tools;35.0.0`, NDK `28.2.13676358`), JDK 25, and Python 3.10+.

```sh
git clone https://github.com/NaiSenshin/ShulkDS.git
cd ShulkDS
python3 packaging/android/build_apk.py --profile fast
```

The compiled APK lands in `target/android/shulkds-1.0.0-arm64-v8a.apk`. Set `HORIZON_ANDROID_KEYSTORE`, `HORIZON_ANDROID_KEY_ALIAS`, `HORIZON_ANDROID_STORE_PASSWORD`, and `HORIZON_ANDROID_KEY_PASSWORD` to sign with your release keystore.

Minecraft `.jar` files, assets, and prepared resources are never committed or packaged.

## License & Attribution

ShulkDS combines MIT-licensed code from [Horizon](https://github.com/horizon-rust/horizon) (Ludovic Marie & contributors) and [Starling](https://github.com/starling-client/starling) (JuanLuci & Vasysik) with LGPL-3.0 components and UI definitions from **Distant Horizons**, **Legacy4J**, and **Controlify**. Accordingly, this combined distribution is provided under **LGPL-3.0** ([LICENSE](LICENSE), [licenses/LGPL-3.0.txt](licenses/LGPL-3.0.txt), [licenses/GPL-3.0.txt](licenses/GPL-3.0.txt)), while original Horizon, Starling, and ShulkDS code remains MIT-licensed.

Full third-party attributions and license notices are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
