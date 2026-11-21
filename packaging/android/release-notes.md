**ShulkDS v1.0.1**

> **No Minecraft files are included.** On first launch, sign in with a Microsoft account that owns *Minecraft: Java Edition* and ShulkDS downloads and prepares official 26.3 directly on your device.

## What's New in v1.0.1

- **Distant Horizons OFF by Default:** Distant Horizons no longer runs automatically on fresh setups or unless enabled — you can still toggle it on anytime in **Options → Mods → Distant Horizons**.
- **16-Chunk Base Render Distance:** Default `renderDistance` across handheld tiers is now **16 chunks** out of the box.
- **No Fresh-World Tutorial Tip Popups:** Disabled the top-right tutorial toast hints (`Move`, `Look around`, `Find a tree`, etc.) when starting a new world for an uncluttered HUD.

## Install

- Download **`shulkds-1.0.1-arm64-v8a.apk`** below and open it on your Android handheld (`Android 8.0+`, `arm64-v8a`, Vulkan).
- **Upgrading from `v1.0.0`:** Installs directly over `v1.0.0` (signed with the same release key).
- **Upgrading from a debug-signed `0.8.0` test APK:** Uninstall the debug build once before installing `v1.0.1` (your `/sdcard/ShulkDS` worlds, settings, and packs remain untouched).

## Included Features

- **Dual-Screen Handheld Architecture & 120 Hz Tuning:** Multi-surface `wgpu` rendering for Display 0 (16:9 top) and `SecondaryDisplay` (4:3 touch bottom), triple-buffered hardware `Fifo` VSync, on-demand `ADPF` boost, bottom-screen IME (`TextMirror`), and `2` chunk worker cap.
- **Bottom Touch Dashboard (`Map`, `Inv`, `Opt`) & Device Telemetry:** Live minimap + `DAY: N`/biome/`FPS` cards, touch armor/3D doll/tooltip/backpack with recessed action tray, quick `FSR`/`Frame Gen`/`Render Distance` controls, and flanking local clock & battery icon.
- **1:1 Legacy4J Console UI & 3DS Title Hub:** Authentic Console Edition crafting (8 tabs, 110 sections, 1,202 26.3 recipes), stonecutter, merchant, creative catalog, and container screens with full gamepad support + Controlify-inspired `LookCurve`, `ReachAround`, and 8-slice `RadialMenu`.
- **Single-Pass FSR 1.0 & Mobile 2x Reprojection FrameGen:** Catmull-Rom + 75% RCAS upscaling and lightweight depth-reprojected `Frame Gen: 2x`.
