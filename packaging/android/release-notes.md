**ShulkDS v1.0.0 — First Public Dual-Screen Release**

ShulkDS brings Horizon/Starling (Minecraft: Java Edition 26.3 rewritten cleanly in Rust) to dual-screen Android clamshell handhelds like the **Retroid Pocket Duo** and **AYN Thor**, pairing an uncluttered 3D top display with a touch-driven bottom dashboard and authentic Legacy4J Console Edition GUIs.

> **No Minecraft files are included.** On first launch, sign in with a Microsoft account that owns *Minecraft: Java Edition* and ShulkDS downloads and prepares official 26.3 directly on your device.

## Install

- Download **`shulkds-1.0.0-arm64-v8a.apk`** below and open it on your Android handheld (`Android 8.0+`, `arm64-v8a`, Vulkan).
- If you previously installed a debug-signed `0.8.0` test APK, uninstall it once before installing this release-signed `1.0.0` APK (your `/sdcard/ShulkDS` worlds, settings, and packs remain untouched).

## What's Included in v1.0.0

### Dual-Screen Handheld Architecture & 120 Hz Tuning
- **Native Multi-Surface Presentation:** Drives Display 0 (16:9 top) and the secondary 4:3 touchscreen (`SecondaryDisplay` `Presentation`) on a shared `wgpu::Device` with isolated HUD buffers and `120 Hz` refresh support.
- **Triple-Buffered `Fifo` VSync & On-Demand `ADPF` Boost:** Low-latency hardware VSync without `Mailbox` frame-queue lag, dynamic `APerformanceHint` boost sessions, and efficiency-core worker caps (`2` chunk + `1` Distant Horizons worker).
- **Bottom-Screen IME (`TextMirror`):** Typing in chat, signs, worlds, or searches opens the soft keyboard on the bottom screen while keeping top-screen camera/gameplay focus intact.

### Bottom Touch Dashboard (`Map`, `Inv`, `Opt`) & Device HUD
- **Map:** Live terrain minimap with zoom bar (`0.5x`–`4x`), 8-way compass, `DAY: N` and in-game clock, `X`/`Y`/`Z` chips, biome card, and live `FPS` / `FPS (FG)` readout.
- **Inv:** Armor slots, tap-to-auto-equip 3D player preview, offhand slot, rarity-colored item hover tooltip, `3x9` touch backpack, and recessed `Quick`/`Split`/`Drop`/(`Sort`|`Clear`) tray.
- **Opt:** Quick sliders (`Render Distance`, `FOV`, `Brightness`, `Volume`), `FSR`, `Frame Gen`, `Reach-Around`, and `Camera` toggles.
- **Persistent Hotbar & Telemetry:** Scaled hotbar flanked by a translucent local clock and pixelated battery icon on the bottom screen, plus a clean dirt backdrop during loading/saving screens.

### Top-Screen Legacy4J Console UI & 3DS-Style Title Hub
- **3DS-Inspired Title Hub:** Centered logo, splash, and rotatable 3D player avatar on top; dirt-backdrop world, server, creation, and options pages on the bottom with spatial 2D D-pad navigation.
- **1:1 Legacy4J GUIs:** Complete Console Edition crafting (8 tabs, 110 organized sections across all 1,202 26.3 recipes, smooth scrollbar, non-ghosting variant popouts), stonecutter, merchant, creative catalog, and container screens with full gamepad support.
- **Controller Features:** Controlify-inspired `LookCurve`, `ReachAround`, 8-slice quick-action `RadialMenu`, `DPadDown` smooth hold-to-zoom, and `DPadLeft` chat shortcut.

### Distant Horizons, Single-Pass FSR 1.0 & Mobile FrameGen
- **Built-in Distant Horizons (`0.1.0`):** Native Rust LOD terrain up to `1024` render distance, early-Z opaque culling, Adreno `MultiDrawIndirect` path, 600 ms Bayer crossfading, and internal `dh-cache` storage.
- **Single-Pass FSR 1.0 & Mobile 2x Reprojection:** Catmull-Rom + 75% RCAS single-pass spatial upscaling and lightweight 1-pass depth-reprojected `Frame Gen: 2x`.
