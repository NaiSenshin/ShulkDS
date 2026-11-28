//! ShulkDS: Legacy4J Touch-Oriented In-Game Dashboard.
//!
//! Combines the **Legacy4J (`Wilyicaro/Legacy-Minecraft`)** visual sprite theme
//! (`panel`, `panel_recess`, `high_tab_*`, `low_tab`, `button_slot*`,
//! `crafting_selection`, `arrow`, `hotbar_selection`) with finger- and
//! stylus-friendly touch ergonomics on the dual-screen bottom display:
//! - Large full-width slots across Inventory & Equipment (1-row equipment bar + 3x9 grid).
//! - Quick-Move touch mode, double-tap Quick-Move, finger drag-and-drop, and Split/Drop actions.
//! - Paged 5x3 touch Recipe Book side-by-side with 1-tap Craft 1 / Craft Max / Clear controls.
//! - Tap-to-zoom Map and interactive drag/tap Settings sliders.

use super::Game;
use horizon_client::{
    dual_screen::{
        BottomPointerAction, DashboardTab, HotbarPosition, LidSleepMode, PointerButton,
        ViewportRect,
    },
    graphics::{FrameGeneration, FsrUpscaling, Preset, Upscaling},
    hud::{HudBatch, MINIMAP_SPRITE},
    items::ItemStack,
    ui::{self, nine_slice},
};
use std::time::Instant;

#[inline]
fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from_le_bytes([r, g, b, a])
}

// Legacy4J UI palette
const PANEL_BG: u32 = u32::from_le_bytes([0xC6, 0xC6, 0xC6, 0xFF]);
const PANEL_BORDER: u32 = u32::from_le_bytes([0x14, 0x14, 0x14, 0xFF]);
const LCE_CONTENT_BG: u32 = u32::from_le_bytes([0xAD, 0xAD, 0xAD, 0xFF]);
const LCE_TAB_BACK: u32 = u32::from_le_bytes([0x8B, 0x8B, 0x8B, 0xFF]);
const LCE_BTN_BG: u32 = u32::from_le_bytes([0x7A, 0x7A, 0x7A, 0xFF]);
const LCE_BTN_HOVER: u32 = u32::from_le_bytes([0x78, 0x86, 0xC6, 0xFF]);
const LCE_BTN_HOVER_HI: u32 = u32::from_le_bytes([0xB4, 0xBE, 0xF5, 0xFF]);
const LCE_BTN_HOVER_SH: u32 = u32::from_le_bytes([0x49, 0x53, 0x88, 0xFF]);
const SLOT_BG: u32 = u32::from_le_bytes([0x8B, 0x8B, 0x8B, 0xFF]);
const TEXT_DARK: u32 = u32::from_le_bytes([0x3F, 0x3F, 0x3F, 0xFF]);
const TEXT_WHITE: u32 = u32::from_le_bytes([0xFF, 0xFF, 0xFF, 0xFF]);

/// Shared geometry for the bottom screen so rendering and hit-testing match 1:1.
struct Bottom3dsLayout {
    hotbar_pos: HotbarPosition,
    show_hotbar: bool,
    tab_y: f32,
    tab_h: f32,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    content_h: f32,
    hotbar_x: f32,
    hotbar_y: f32,
    hotbar_w: f32,
    hotbar_h: f32,
    hotbar_scale: f32,
}

impl Bottom3dsLayout {
    fn compute(bx: f32, by: f32, bw: f32, bh: f32, hotbar_pos: HotbarPosition) -> Self {
        let tab_h = 24.0;
        let show_hotbar = hotbar_pos != HotbarPosition::TopScreenClassic;

        let hotbar_scale = ((bw - 88.0) / 182.0).clamp(0.68, 1.25);
        let hotbar_w = (182.0 * hotbar_scale).round();
        let hotbar_h = if show_hotbar {
            (22.0 * hotbar_scale).round()
        } else {
            0.0
        };
        let hotbar_x = bx + ((bw - hotbar_w) / 2.0).floor();

        let content_x = bx + 3.0;
        let content_w = (bw - 6.0).max(40.0);
        let (content_h, total_stack_h) = if show_hotbar {
            let ch = (bh - tab_h - hotbar_h - 10.0).clamp(40.0, 206.0);
            (ch, tab_h + 2.0 + ch + 4.0 + hotbar_h)
        } else {
            let ch = (bh - tab_h - 16.0).clamp(40.0, 224.0);
            (ch, tab_h + 2.0 + ch + 10.0)
        };

        let top_margin = ((bh - total_stack_h) * 0.5).floor().max(2.0);
        let (hotbar_y, tab_y, content_y) = match hotbar_pos {
            HotbarPosition::BottomScreenBottom => {
                let ty = by + top_margin;
                let cy = ty + tab_h + 2.0;
                let hy = cy + content_h + 4.0;
                (hy, ty, cy)
            }
            HotbarPosition::BottomScreenTop => {
                let hy = by + top_margin;
                let ty = hy + hotbar_h + 4.0;
                let cy = ty + tab_h + 2.0;
                (hy, ty, cy)
            }
            HotbarPosition::TopScreenClassic => {
                let ty = by + top_margin;
                let cy = ty + tab_h + 2.0;
                let hy = cy + content_h + 4.0;
                (hy, ty, cy)
            }
        };

        Self {
            hotbar_pos,
            show_hotbar,
            tab_y,
            tab_h,
            content_x,
            content_y,
            content_w,
            content_h,
            hotbar_x,
            hotbar_y,
            hotbar_w,
            hotbar_h,
            hotbar_scale,
        }
    }

    /// X origin and width of top tab `idx`, spanning `content_x..content_x + content_w`
    /// symmetrically with `2px` gaps so left/right borders align with the main panel.
    fn tab_rect(&self, idx: usize) -> (f32, f32) {
        let count = DashboardTab::ALL.len();
        let gap = 2.0;
        let usable = (self.content_w - (count as f32 - 1.0) * gap).max(count as f32 * 8.0);
        let base_w = (usable / count as f32).floor();
        let rem = (usable - count as f32 * base_w).round().max(0.0) as usize;
        // Spread leftover sub-3 pixels middle-outward ([1, 0, 2]) to keep balance.
        let extra_for = |k: usize| -> f32 {
            let order = [1usize, 0, 2];
            if order.iter().take(rem).any(|&slot| slot == k) {
                1.0
            } else {
                0.0
            }
        };
        let mut x = self.content_x;
        for k in 0..idx {
            x += base_w + extra_for(k) + gap;
        }
        (x, base_w + extra_for(idx))
    }

    /// Interior rect inside `legacy/tiles/panel` with equal `3px` margins inside
    /// its flat `#C6C6C6` face (`1px` top/left highlight, `2px` bottom/right shadow).
    fn inner_rect(&self) -> (f32, f32, f32, f32) {
        (
            self.content_x + 4.0,
            self.content_y + 4.0,
            (self.content_w - 9.0).max(20.0),
            (self.content_h - 9.0).max(20.0),
        )
    }
}

struct SurvivalInvLayout {
    slot_size: f32,
    armor_x: f32,
    armor_y0: f32,
    player_x: f32,
    player_y: f32,
    player_w: f32,
    player_h: f32,
    bag_x0: f32,
    bag_y0: f32,
    include_hotbar_row: bool,
    hotbar_row_y: f32,
    offhand_x: f32,
    offhand_y: f32,
    offhand_sz: f32,
    info_x: f32,
    info_y: f32,
    info_w: f32,
    info_h: f32,
    tray_x: f32,
    tray_y: f32,
    tray_w: f32,
    tray_h: f32,
    act_x: f32,
    act_y: f32,
    act_w: f32,
    act_h: f32,
}

struct MapRightLayout {
    rx: f32,
    rw: f32,
    clock_y: f32,
    clock_h: f32,
    heading_y: f32,
    heading_h: f32,
    coord_ys: [f32; 3],
    coord_h: f32,
    biome_y: f32,
    biome_h: f32,
    fps_box_y: f32,
    fps_box_h: f32,
    zoom_y: f32,
    zoom_h: f32,
    minus_x: f32,
    step_w: f32,
    readout_x: f32,
    readout_w: f32,
    plus_x: f32,
}

struct BiomeCardLayout {
    icon_rect: Option<[f32; 4]>,
    text_x: f32,
    #[cfg_attr(not(test), allow(dead_code))]
    avail_w: f32,
    biome_lines: Vec<(String, f32, f32)>,
    dim_line: Option<(String, f32, f32)>,
}

impl BiomeCardLayout {
    fn truncate_measured(text: &str, max_w: f32, measure: &impl Fn(&str) -> f32) -> String {
        if measure(text) <= max_w {
            return text.to_owned();
        }
        let mut s = text.to_owned();
        while !s.is_empty() && measure(&format!("{s}..")) > max_w {
            s.pop();
        }
        if s.is_empty() {
            String::new()
        } else {
            format!("{s}..")
        }
    }

    fn fit_measured_line(
        text: &str,
        avail_w: f32,
        min_scale: f32,
        max_scale: f32,
        measure: &impl Fn(&str) -> f32,
    ) -> (String, f32) {
        let raw_w = measure(text).max(1.0);
        if raw_w * min_scale <= avail_w {
            let scale = (avail_w / raw_w).clamp(min_scale, max_scale);
            (text.to_owned(), scale)
        } else {
            let fitted = Self::truncate_measured(text, avail_w / min_scale, measure);
            (fitted, min_scale)
        }
    }

    fn split_two_lines<'a>(
        text: &'a str,
        measure: &impl Fn(&str) -> f32,
    ) -> Option<(&'a str, &'a str)> {
        let mut best: Option<(f32, &str, &str)> = None;
        for (idx, ch) in text.char_indices() {
            if ch != ' ' {
                continue;
            }
            let left = text[..idx].trim();
            let right = text[idx + 1..].trim();
            if left.is_empty() || right.is_empty() {
                continue;
            }
            let widest = measure(left).max(measure(right));
            if best.is_none_or(|(bw, _, _)| widest < bw) {
                best = Some((widest, left, right));
            }
        }
        best.map(|(_, l, r)| (l, r))
    }

    fn compute(
        rx: f32,
        ry: f32,
        rw: f32,
        rh: f32,
        biome_title: &str,
        dim_name: &str,
        measure: impl Fn(&str) -> f32,
    ) -> Self {
        let right_limit = rx + rw - 5.0;
        if rh < 22.0 || rw < 64.0 {
            let text_x = rx + 5.0;
            let avail_w = (right_limit - text_x).max(8.0);
            let max_b = (rh - 4.0).max(5.0) / 8.0;
            let min_b = 0.50_f32.min(max_b);
            let (b_str, b_scale) =
                Self::fit_measured_line(biome_title, avail_w, min_b, max_b.min(0.76), &measure);
            let y0 = ry + (rh - 8.0 * b_scale) * 0.5;
            return Self {
                icon_rect: None,
                text_x,
                avail_w,
                biome_lines: vec![(b_str, y0, b_scale)],
                dim_line: None,
            };
        }

        let thumb_sz = (rh - 10.0).min(rw * 0.22).floor().clamp(12.0, 21.0);
        let thumb_x = rx + 5.0;
        let thumb_y = (ry + (rh - thumb_sz) * 0.5).floor();
        let text_x = thumb_x + thumb_sz + 6.0;
        let avail_w = (right_limit - text_x).max(8.0);
        let icon_rect = Some([thumb_x, thumb_y, thumb_sz, thumb_sz]);

        let raw_b = measure(biome_title).max(1.0);
        let single_line_fits = raw_b * 0.70 <= avail_w;
        if single_line_fits || rh < 28.0 || Self::split_two_lines(biome_title, &measure).is_none() {
            let max_b = if rh >= 32.0 { 0.86 } else { 0.80 };
            let max_d = if rh >= 32.0 { 0.74 } else { 0.68 };
            let (mut b_str, mut b_scale) =
                Self::fit_measured_line(biome_title, avail_w, 0.54, max_b, &measure);
            let (mut d_str, mut d_scale) =
                Self::fit_measured_line(dim_name, avail_w, 0.52, max_d, &measure);
            let gap = if rh >= 32.0 { 3.0 } else { 2.2 };
            let max_total = (rh - 5.0).max(12.0);
            let text_avail = (max_total - gap).max(8.0);
            let needed = 9.0 * b_scale + 8.0 * d_scale;
            if needed > text_avail {
                let k = text_avail / needed;
                b_scale *= k;
                d_scale *= k;
                b_str = Self::truncate_measured(biome_title, avail_w / b_scale, &measure);
                d_str = Self::truncate_measured(dim_name, avail_w / d_scale, &measure);
            }
            let b_cell = 9.0 * b_scale;
            let d_cell = 8.0 * d_scale;
            let total = b_cell + gap + d_cell;
            let y0 = (ry + (rh - total) * 0.5).max(ry + 2.5);
            let dy = y0 + b_cell + gap;
            return Self {
                icon_rect,
                text_x,
                avail_w,
                biome_lines: vec![(b_str, y0, b_scale)],
                dim_line: Some((d_str, dy, d_scale)),
            };
        }

        let (part1, part2) = Self::split_two_lines(biome_title, &measure).unwrap();
        let max_b = ((rh - 7.0) / 26.5).clamp(0.54, 0.72);
        let widest = measure(part1).max(measure(part2)).max(1.0);
        let mut b_scale = (avail_w / widest).min(max_b).clamp(0.50, max_b);
        let max_d = (max_b * 0.92).clamp(0.50, 0.66);
        let (mut d_str, mut d_scale) =
            Self::fit_measured_line(dim_name, avail_w, 0.50, max_d, &measure);
        let gap1 = 1.6_f32;
        let gap2 = 2.2_f32;
        let max_total = (rh - 5.0).max(16.0);
        let text_avail = (max_total - gap1 - gap2).max(10.0);
        let needed = 2.0 * 8.8 * b_scale + 8.0 * d_scale;
        if needed > text_avail {
            let k = text_avail / needed;
            b_scale *= k;
            d_scale *= k;
            d_str = Self::truncate_measured(dim_name, avail_w / d_scale, &measure);
        }
        let s1 = Self::truncate_measured(part1, avail_w / b_scale, &measure);
        let s2 = Self::truncate_measured(part2, avail_w / b_scale, &measure);
        let b_cell = 8.8 * b_scale;
        let d_cell = 8.0 * d_scale;
        let total = 2.0 * b_cell + gap1 + gap2 + d_cell;
        let y1 = (ry + (rh - total) * 0.5).max(ry + 2.5);
        let y2 = y1 + b_cell + gap1;
        let dy = y2 + b_cell + gap2;
        Self {
            icon_rect,
            text_x,
            avail_w,
            biome_lines: vec![(s1, y1, b_scale), (s2, y2, b_scale)],
            dim_line: Some((d_str, dy, d_scale)),
        }
    }
}

struct SettingsTabMetrics {
    show_header: bool,
    header_y: f32,
    header_h: f32,
    all_opts_rect: [f32; 4],
    row_x: f32,
    row_w: f32,
    row_step: f32,
    row_h: f32,
    label_x: f32,
    label_w: f32,
    rows_y0: f32,
    minus_x: f32,
    step_btn_w: f32,
    track_x: f32,
    track_w: f32,
    plus_x: f32,
    val_x: f32,
    val_w: f32,
    perf_y: f32,
    toggle_y: f32,
    hud_y: f32,
    toggle_h: f32,
    reach_x: f32,
    reach_w: f32,
    cam_x: f32,
    cam_w: f32,
}

impl Game<'_> {
    /// Draw Legacy4J's `legacy:tiles/panel` 9-slice dialog panel.
    fn draw_3ds_panel(&self, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
        let a = &self.gui.atlas;
        if a.sprites.contains_key("legacy/tiles/panel") {
            nine_slice(batch, a, "legacy/tiles/panel", x, y, w, h, u32::MAX);
        } else {
            batch.fill(a, x, y, w, h, PANEL_BORDER);
            batch.fill(a, x + 1.0, y + 1.0, w - 2.0, h - 2.0, PANEL_BG);
        }
    }

    /// Draw Legacy4J's `legacy:tiles/panel_recess` 9-slice recessed inner panel.
    fn draw_lce_content_panel(&self, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
        let a = &self.gui.atlas;
        if a.sprites.contains_key("legacy/tiles/panel_recess") {
            nine_slice(batch, a, "legacy/tiles/panel_recess", x, y, w, h, u32::MAX);
        } else {
            batch.fill(a, x, y, w, h, LCE_CONTENT_BG);
        }
    }

    /// Draw vanilla Minecraft's sunken gray inventory slot (`#8B8B8B` fill, `#373737` top/left, `#FFFFFF` bottom/right).
    fn draw_3ds_slot_rect(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        highlight: bool,
    ) {
        let a = &self.gui.atlas;
        let bg = if highlight {
            rgba(0xA8, 0xA8, 0xA8, 0xFF)
        } else {
            SLOT_BG
        };
        let dark = rgba(0x37, 0x37, 0x37, 0xFF);
        let light = rgba(0xFF, 0xFF, 0xFF, 0xFF);
        batch.fill(a, x, y, w, h, bg);
        batch.fill(a, x, y, (w - 1.0).max(1.0), 1.0, dark);
        batch.fill(a, x, y, 1.0, (h - 1.0).max(1.0), dark);
        batch.fill(a, x + 1.0, y + h - 1.0, (w - 1.0).max(1.0), 1.0, light);
        batch.fill(a, x + w - 1.0, y + 1.0, 1.0, (h - 1.0).max(1.0), light);
    }

    /// Draw Legacy4J's `legacy:container/button_slot` (or `button_slot_selected`).
    /// Record `slot_idx` as the currently active slot, shifting any different prior slot into `prev_slot`.
    fn select_dashboard_slot(&mut self, slot_idx: usize) {
        if self.shulkds.dashboard.active_slot != Some(slot_idx) {
            if let Some(prev) = self.shulkds.dashboard.active_slot {
                self.shulkds.dashboard.prev_slot = Some(prev);
            }
            self.shulkds.dashboard.active_slot = Some(slot_idx);
        }
        if slot_idx < 9 {
            self.selected = slot_idx;
        }
    }

    /// Return any carried inventory item (`self.carried`) back to inventory so it cannot
    /// be carried onto non-inventory pages like Map, Settings, or the Creative catalog.
    pub(super) fn return_shulkds_carried_to_inventory(&mut self) {
        self.shulkds.dashboard.drag_slot = None;
        self.shulkds.dashboard.last_tap_slot = None;
        if self.carried.is_none() {
            return;
        }
        if self.remote.is_some() {
            let allow_offhand = matches!(
                self.remote_menu().1.as_str(),
                "inventory" | "minecraft:inventory"
            );
            for _ in 0..36 {
                let Some(carried) = self.carried.clone() else {
                    break;
                };
                let max_s = world_items!(self)
                    .get(&carried.item)
                    .map_or(64, |i| i.max_stack)
                    .max(1);
                let preferred = [
                    self.shulkds.dashboard.active_slot,
                    self.shulkds.dashboard.prev_slot,
                ]
                .into_iter()
                .flatten()
                .find(|&s| {
                    (s < 36 || (allow_offhand && s == 504))
                        && self
                            .slot_view(s)
                            .as_ref()
                            .is_none_or(|ex| ex.item == carried.item && ex.count < max_s)
                });
                let dest = preferred
                    .or_else(|| {
                        (9..36).chain(0..9).find(|&s| {
                            self.inventory[s]
                                .as_ref()
                                .is_some_and(|ex| ex.item == carried.item && ex.count < max_s)
                        })
                    })
                    .or_else(|| (9..36).chain(0..9).find(|&s| self.inventory[s].is_none()));
                let Some(dest) = dest else {
                    break;
                };
                let prev_count = carried.count;
                let prev_item = carried.item.clone();
                if !self.remote_menu_ops(&[(Some(dest), 0, horizon_client::menu::Input::Pickup)]) {
                    break;
                }
                if self
                    .carried
                    .as_ref()
                    .is_some_and(|c| c.item == prev_item && c.count >= prev_count)
                {
                    break;
                }
            }
            if self.carried.is_some() {
                if self.creative {
                    self.carried = None;
                } else {
                    let _ = self.remote_menu_ops(&[(None, 0, horizon_client::menu::Input::Pickup)]);
                    self.carried = None;
                }
            }
            if self.creative {
                self.send_creative_slots();
            }
            return;
        }
        let Some(mut carried) = self.carried.take() else {
            return;
        };
        let max_s = world_items!(self)
            .get(&carried.item)
            .map_or(64, |i| i.max_stack)
            .max(1);
        let preferred = [
            self.shulkds.dashboard.active_slot,
            self.shulkds.dashboard.prev_slot,
        ];
        // Check active/prev slot first if it has space or is empty, then partial stacks, then empty slots.
        let candidates: Vec<usize> = preferred
            .into_iter()
            .flatten()
            .filter(|&s| s < 36 || s == 504)
            .filter(|&s| {
                let current = if s == 504 {
                    self.offhand.as_ref()
                } else {
                    self.inventory[s].as_ref()
                };
                current.is_none_or(|ex| ex.item == carried.item && ex.count < max_s)
            })
            .chain((9..36).chain(0..9).filter(|&s| {
                self.inventory[s]
                    .as_ref()
                    .is_some_and(|ex| ex.item == carried.item && ex.count < max_s)
            }))
            .chain((9..36).chain(0..9).filter(|&s| self.inventory[s].is_none()))
            .collect();
        for slot_idx in candidates {
            if carried.count == 0 {
                break;
            }
            let target = self.slot_mut(slot_idx);
            match target {
                Some(existing) if existing.item == carried.item && existing.count < max_s => {
                    let add = (max_s - existing.count).min(carried.count);
                    existing.count += add;
                    carried.count -= add;
                }
                None => {
                    *target = Some(carried);
                    return;
                }
                _ => {}
            }
        }
    }

    /// Draw a 1-2px outline rectangle around `(x, y, w, h)`.
    fn draw_slot_outline(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        t: f32,
        color: u32,
    ) {
        let a = &self.gui.atlas;
        batch.fill(a, x, y, w, t, color);
        batch.fill(a, x, y + h - t, w, t, color);
        batch.fill(a, x, y, t, h, color);
        batch.fill(a, x + w - t, y, t, h, color);
    }

    /// Render the DS/Console active-slot cursor (`legacy/hud/hotbar_selection` + highlight).
    fn draw_slot_selection_visuals_rect(
        &self,
        batch: &mut HudBatch,
        slot_idx: usize,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
    ) {
        let is_active = self.shulkds.dashboard.active_slot == Some(slot_idx);

        if is_active {
            if self
                .gui
                .atlas
                .sprites
                .contains_key("legacy/hud/hotbar_selection")
            {
                batch.sprite_scaled(
                    &self.gui.atlas,
                    "legacy/hud/hotbar_selection",
                    sx - 2.0,
                    sy - 2.0,
                    sw + 4.0,
                    sh + 4.0,
                    u32::MAX,
                );
            } else {
                self.draw_slot_outline(
                    batch,
                    sx - 1.5,
                    sy - 1.5,
                    sw + 3.0,
                    sh + 3.0,
                    1.5,
                    rgba(255, 224, 72, 255),
                );
            }
        }
    }

    /// Draw a rectangular interactive slot, centering the square item/placeholder icon inside `(sw, sh)`.
    #[allow(clippy::too_many_arguments)]
    fn draw_3ds_interactive_slot_rect(
        &mut self,
        batch: &mut HudBatch,
        slot_idx: usize,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        accent_highlight: bool,
        placeholder_sprite: Option<&str>,
    ) {
        self.register_focus(sx, sy, sw, sh, Some(slot_idx));
        let is_active = self.shulkds.dashboard.active_slot == Some(slot_idx);
        self.draw_3ds_slot_rect(batch, sx, sy, sw, sh, is_active || accent_highlight);

        let icon_sz = sw.min(sh);
        let ix = sx + (sw - icon_sz) * 0.5;
        let iy = sy + (sh - icon_sz) * 0.5;
        if let Some(stack) = self.slot_view(slot_idx) {
            self.draw_touch_slot_item(batch, slot_idx, &stack, ix, iy, icon_sz);
        } else if let Some(ph) = placeholder_sprite
            && self.gui.atlas.sprites.contains_key(ph)
        {
            batch.sprite_scaled(
                &self.gui.atlas,
                ph,
                ix + 1.5,
                iy + 1.5,
                icon_sz - 3.0,
                icon_sz - 3.0,
                u32::MAX,
            );
        }

        self.draw_slot_selection_visuals_rect(batch, slot_idx, sx, sy, sw, sh);
    }

    /// Draw an interactive inventory/equipment/craft/container slot with active & last-used visuals.
    fn draw_3ds_interactive_slot(
        &mut self,
        batch: &mut HudBatch,
        slot_idx: usize,
        sx: f32,
        sy: f32,
        size: f32,
        accent_highlight: bool,
        placeholder_sprite: Option<&str>,
    ) {
        self.draw_3ds_interactive_slot_rect(
            batch,
            slot_idx,
            sx,
            sy,
            size,
            size,
            accent_highlight,
            placeholder_sprite,
        );
    }

    /// Draw an item stack centered inside a touch slot of `slot_size`, scaling cleanly to fit.
    fn draw_touch_slot_item(
        &self,
        batch: &mut HudBatch,
        slot_idx: usize,
        stack: &ItemStack,
        sx: f32,
        sy: f32,
        slot_size: f32,
    ) {
        let target = (slot_size * 0.78).clamp(10.0, 18.0);
        let scale = (target / 16.0).clamp(0.60, 1.125);
        let draw_sz = 16.0 * scale;
        let ix = sx + (slot_size - draw_sz) * 0.5;
        let iy = sy + (slot_size - draw_sz) * 0.5;
        let start = batch.vertices.len();
        self.draw_slot_item(batch, None, slot_idx, &Some(stack.clone()), ix, iy);
        if (scale - 1.0).abs() > 0.02 {
            batch.scale_since(start, ix, iy, scale, scale);
        }
    }

    /// Register an interactive widget rectangle for D-pad navigation & slot lookup on the bottom screen.
    fn register_focus(&mut self, x: f32, y: f32, w: f32, h: f32, slot: Option<usize>) {
        self.shulkds.dashboard.focus_rects.push([x, y, w, h]);
        self.shulkds.dashboard.focus_slots.push(slot);
        self.shulkds.dashboard.focus_buttons.push(false);
    }

    /// Register a non-slot button rectangle for D-pad navigation (highlighted blue instead of slot outline).
    fn register_focus_button(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.shulkds.dashboard.focus_rects.push([x, y, w, h]);
        self.shulkds.dashboard.focus_slots.push(None);
        self.shulkds.dashboard.focus_buttons.push(true);
    }

    fn is_bottom_dpad_focused(&self, _idx: usize) -> bool {
        false
    }

    /// Draw a Legacy4J `main_button_default` / `main_button_hover` button with auto-fitting scaled text.
    #[allow(clippy::too_many_arguments)]
    fn draw_3ds_button_scaled(
        &mut self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: &str,
        highlighted: bool,
        max_scale: f32,
    ) {
        let focus_idx = self.shulkds.dashboard.focus_rects.len();
        let dpad_focused = self.is_bottom_dpad_focused(focus_idx);
        self.register_focus_button(x, y, w, h);
        let highlighted = highlighted || dpad_focused;
        let a = &self.gui.atlas;
        let sprite = if highlighted {
            "gui/sprites/widget/button_highlighted"
        } else {
            "gui/sprites/widget/button"
        };
        if a.sprites.contains_key(sprite) {
            ui::nine_slice(batch, a, sprite, x, y, w, h, u32::MAX);
        } else {
            let (bg, hi, sh) = if highlighted {
                (LCE_BTN_HOVER, LCE_BTN_HOVER_HI, LCE_BTN_HOVER_SH)
            } else {
                (
                    LCE_BTN_BG,
                    rgba(0xA4, 0xA4, 0xA4, 0xFF),
                    rgba(0x4C, 0x4C, 0x4C, 0xFF),
                )
            };
            batch.fill(a, x, y, w, h, PANEL_BORDER);
            batch.fill(a, x + 1.0, y + 1.0, w - 2.0, h - 2.0, bg);
            batch.fill(a, x + 1.0, y + 1.0, w - 2.0, 1.0, hi);
            batch.fill(a, x + 1.0, y + 1.0, 1.0, h - 2.0, hi);
            batch.fill(a, x + 1.0, y + h - 2.0, w - 2.0, 1.0, sh);
            batch.fill(a, x + w - 2.0, y + 1.0, 1.0, h - 2.0, sh);
        }
        let lw = HudBatch::text_width(a, label);
        let fit_w = (w - 4.0).max(4.0) / lw.max(1.0);
        let fit_h = (h - 3.0).max(4.0) / 8.0;
        let scale = fit_w.min(fit_h).min(max_scale).clamp(0.45, 1.0);
        let lx = x + ((w - lw * scale) * 0.5).max(2.0);
        let ly = y + ((h - 8.0 * scale) * 0.5);
        let fg = if highlighted {
            rgba(0xFF, 0xFF, 0xA0, 0xFF)
        } else {
            TEXT_WHITE
        };
        batch.text_scaled(a, label, lx, ly, scale, fg, true);
    }

    /// 16x16 pixel-art Minecraft Iron Gear/Cog icon for the Options tab.
    pub(super) fn draw_3ds_options_icon(&self, batch: &mut HudBatch, x: f32, y: f32, size: f32) {
        const COG_ROWS: [&str; 16] = [
            "......####......",
            "......#HH#......",
            "..##..#HB#..##..",
            ".#HH#.#HB#.#HH#.",
            ".#HBH#HBBH#HBS#.",
            "..#HBH####HBS#..",
            "##HBB#....#HBS##",
            "#HBB#......#BBS#",
            "#HBB#......#BBS#",
            "##HBH#....#BBS##",
            "..#HBH####SBS#..",
            ".#HBS#HBBS#SBS#.",
            ".#HS#.#BS#.#SS#.",
            "..##..#BS#..##..",
            "......#SS#......",
            "......####......",
        ];
        let p = size / 16.0;
        let a = &self.gui.atlas;
        let color_for = |ch: u8| -> Option<u32> {
            match ch {
                b'#' => Some(rgba(0x24, 0x24, 0x24, 0xFF)),
                b'H' => Some(rgba(0xDC, 0xDC, 0xDC, 0xFF)),
                b'B' => Some(rgba(0x96, 0x96, 0x96, 0xFF)),
                b'S' => Some(rgba(0x58, 0x58, 0x58, 0xFF)),
                _ => None,
            }
        };
        for (ry, row) in COG_ROWS.iter().enumerate() {
            let bytes = row.as_bytes();
            let mut cx = 0usize;
            while cx < 16 {
                let Some(col) = color_for(bytes[cx]) else {
                    cx += 1;
                    continue;
                };
                let mut end = cx + 1;
                while end < 16 && bytes[end] == bytes[cx] {
                    end += 1;
                }
                batch.fill(
                    a,
                    x + cx as f32 * p,
                    y + ry as f32 * p,
                    (end - cx) as f32 * p,
                    p,
                    col,
                );
                cx = end;
            }
        }
    }

    /// Whether the top screen is showing a pause/overlay menu that locks the bottom screen.
    pub(super) fn is_shulkds_paused(&self) -> bool {
        self.shulkds.config.enabled
            && self.title.is_none()
            && !matches!(
                self.screen,
                super::Screen::Playing | super::Screen::Chat | super::Screen::Dead
            )
    }

    fn bottom_3ds_layout(&self, bx: f32, by: f32, bw: f32, bh: f32) -> Bottom3dsLayout {
        Bottom3dsLayout::compute(bx, by, bw, bh, self.settings.extras.shulkds_hotbar)
    }

    pub(super) fn should_auto_return_bottom_carried(
        is_top_paused: bool,
        has_carried: bool,
        active_tab: DashboardTab,
    ) -> bool {
        !is_top_paused && has_carried && active_tab != DashboardTab::Inventory
    }

    /// Draw the complete bottom-screen touch dashboard within `rect`.
    pub(super) fn draw_shulkds_bottom_dashboard(
        &mut self,
        batch: &mut HudBatch,
        rect: &ViewportRect,
    ) {
        if self.shulkds_title_active() {
            self.draw_shulkds_title_bottom(batch, rect);
            return;
        }
        if Self::should_auto_return_bottom_carried(
            self.is_shulkds_paused(),
            self.carried.is_some(),
            self.shulkds.dashboard.active_tab,
        ) {
            self.return_shulkds_carried_to_inventory();
        }
        self.shulkds.dashboard.focus_rects.clear();
        self.shulkds.dashboard.focus_slots.clear();
        self.shulkds.dashboard.focus_buttons.clear();
        let (bx, by, bw, bh) = (rect.x, rect.y, rect.width, rect.height);
        self.draw_shulkds_bottom_backdrop(batch, rect);

        let lay = self.bottom_3ds_layout(bx, by, bw, bh);

        // 3. Top Navigation Tabs (Map, Inv, Opt) with Legacy4J folder tab sprites
        let inv_icon = "item/chest_minecart";
        for (i, &tab) in DashboardTab::ALL.iter().enumerate() {
            let (tx, tw) = lay.tab_rect(i);
            let is_active = self.shulkds.dashboard.active_tab == tab;

            if is_active {
                let high_sprite = if i == 0 {
                    "legacy/tiles/high_tab_left"
                } else if i + 1 == DashboardTab::ALL.len() {
                    "legacy/tiles/high_tab_right"
                } else {
                    "legacy/tiles/high_tab_middle"
                };
                if self.gui.atlas.sprites.contains_key(high_sprite) {
                    nine_slice(
                        batch,
                        &self.gui.atlas,
                        high_sprite,
                        tx,
                        lay.tab_y,
                        tw,
                        lay.tab_h + 4.0,
                        u32::MAX,
                    );
                } else {
                    batch.fill(
                        &self.gui.atlas,
                        tx,
                        lay.tab_y,
                        tw,
                        lay.tab_h + 3.0,
                        PANEL_BG,
                    );
                }
            } else {
                let ty = lay.tab_y + 2.0;
                let th = lay.tab_h - 1.0;
                if self.gui.atlas.sprites.contains_key("legacy/tiles/low_tab") {
                    nine_slice(
                        batch,
                        &self.gui.atlas,
                        "legacy/tiles/low_tab",
                        tx,
                        ty,
                        tw,
                        th,
                        u32::MAX,
                    );
                } else {
                    batch.fill(&self.gui.atlas, tx, ty, tw, th, LCE_TAB_BACK);
                }
            }

            let inner_x = tx + 2.0;
            let inner_w = (tw - 4.0).max(16.0);
            let cy = if is_active {
                lay.tab_y + 0.5 + lay.tab_h * 0.5
            } else {
                lay.tab_y + 1.5 + lay.tab_h * 0.5
            };
            let label = if tw >= 68.0 {
                tab.title()
            } else {
                tab.icon_label()
            };
            let icon_sz = if inner_w >= 44.0 { 12.0 } else { 11.0 };
            let gap = if inner_w >= 44.0 { 3.0 } else { 2.0 };
            let raw_tw = (HudBatch::text_width(&self.gui.atlas, label) - 1.0).max(1.0);
            let shadow_w = if is_active { 0.0 } else { 1.0 };
            let avail_text = (inner_w - 4.0 - icon_sz - gap).max(8.0);
            let text_scale = (avail_text / raw_tw).clamp(0.72, 1.0);
            let vis_text_w = raw_tw * text_scale + shadow_w;
            let group_w = icon_sz + gap + vis_text_w;
            let start_x = (inner_x + (inner_w - group_w) * 0.5).round();
            let icon_x = start_x;
            let icon_y = (cy - icon_sz * 0.5).round();

            match tab {
                DashboardTab::Minimap => {
                    if self.gui.atlas.sprites.contains_key("item/filled_map") {
                        batch.sprite_scaled(
                            &self.gui.atlas,
                            "item/filled_map",
                            icon_x,
                            icon_y,
                            icon_sz,
                            icon_sz,
                            u32::MAX,
                        );
                    }
                }
                DashboardTab::Inventory => {
                    if self.gui.atlas.sprites.contains_key(inv_icon) {
                        batch.sprite_scaled(
                            &self.gui.atlas,
                            inv_icon,
                            icon_x,
                            icon_y,
                            icon_sz,
                            icon_sz,
                            u32::MAX,
                        );
                    }
                }
                DashboardTab::Settings => {
                    self.draw_3ds_options_icon(batch, icon_x, icon_y, icon_sz);
                }
            }

            let fg = if is_active { TEXT_DARK } else { TEXT_WHITE };
            let text_x = icon_x + icon_sz + gap;
            let vis_text_h = 7.0 * text_scale + shadow_w * 0.5;
            let text_y = (cy - vis_text_h * 0.5).round();
            batch.text_scaled(
                &self.gui.atlas,
                label,
                text_x,
                text_y,
                text_scale,
                fg,
                !is_active,
            );
        }

        // 4. Main Beveled Content Panel
        self.draw_3ds_panel(
            batch,
            lay.content_x,
            lay.content_y,
            lay.content_w,
            lay.content_h,
        );

        let (inner_x, inner_y, inner_w, inner_h) = lay.inner_rect();

        match self.shulkds.dashboard.active_tab {
            DashboardTab::Minimap => {
                self.draw_3ds_map_tab(batch, inner_x, inner_y, inner_w, inner_h);
            }
            DashboardTab::Inventory => {
                self.draw_3ds_inventory_tab(batch, &lay, inner_x, inner_y, inner_w, inner_h);
            }
            DashboardTab::Settings => {
                self.draw_3ds_settings_tab(batch, inner_x, inner_y, inner_w, inner_h);
            }
        }

        // 5. Persistent Bottom Hotbar (when placed on the bottom screen) + device telemetry
        if lay.show_hotbar {
            self.draw_3ds_persistent_hotbar(batch, &lay);
        }
        self.draw_shulkds_bottom_device_hud(batch, rect, &lay);

        if self.shulkds.dashboard.active_tab == DashboardTab::Minimap {
            self.shulkds.dashboard.bottom_focus = false;
        }
        if self.shulkds.dashboard.bottom_focus
            && !self.is_shulkds_paused()
            && matches!(
                self.shulkds.dashboard.active_tab,
                DashboardTab::Inventory | DashboardTab::Settings
            )
            && !self.shulkds.dashboard.focus_rects.is_empty()
        {
            let idx = self
                .shulkds
                .dashboard
                .focus_index
                .min(self.shulkds.dashboard.focus_rects.len() - 1);
            if !self
                .shulkds
                .dashboard
                .focus_buttons
                .get(idx)
                .copied()
                .unwrap_or(false)
            {
                let [fx, fy, fw, fh] = self.shulkds.dashboard.focus_rects[idx];
                if self
                    .gui
                    .atlas
                    .sprites
                    .contains_key("legacy/hud/hotbar_selection")
                {
                    batch.sprite_scaled(
                        &self.gui.atlas,
                        "legacy/hud/hotbar_selection",
                        fx - 1.5,
                        fy - 1.5,
                        fw + 3.0,
                        fh + 3.0,
                        u32::MAX,
                    );
                } else {
                    self.draw_slot_outline(
                        batch,
                        fx - 1.0,
                        fy - 1.0,
                        fw + 2.0,
                        fh + 2.0,
                        1.5,
                        rgba(255, 224, 72, 255),
                    );
                }
            }
        }

        // 6. Ephemeral status toast on bottom screen
        if let Some((msg, at)) = &self.shulkds.dashboard.status_message {
            if at.elapsed().as_secs_f32() < 2.5 {
                let tw = HudBatch::text_width(&self.gui.atlas, msg);
                let tx = bx + (bw - tw) / 2.0;
                let ty = if lay.hotbar_pos == HotbarPosition::BottomScreenBottom {
                    lay.hotbar_y - 14.0
                } else {
                    lay.content_y + lay.content_h - 15.0
                };
                batch.fill(
                    &self.gui.atlas,
                    tx - 4.0,
                    ty - 2.0,
                    tw + 8.0,
                    12.0,
                    rgba(20, 20, 24, 215),
                );
                batch.text(&self.gui.atlas, msg, tx, ty, TEXT_WHITE, true);
            }
        }

        // 7. Carried item stack following finger/pointer
        if !self.is_shulkds_paused()
            && let Some(carried) = self.carried.clone()
        {
            let px = self.shulkds.dashboard.pointer_x;
            let py = self.shulkds.dashboard.pointer_y;
            if px >= bx && px <= bx + bw && py >= by && py <= by + bh {
                self.draw_slot_item(batch, None, 999, &Some(carried), px - 8.0, py - 8.0);
            }
        }

        // 8. When paused on top, darken bottom screen, block touches, and overlay PAUSED
        if self.is_shulkds_paused() {
            self.shulkds.dashboard.focus_rects.clear();
            self.shulkds.dashboard.focus_slots.clear();
            self.shulkds.dashboard.focus_buttons.clear();
            batch.fill(&self.gui.atlas, bx, by, bw, bh, rgba(0, 0, 0, 185));
            let pause_label = "PAUSED";
            let scale = 2.0;
            let tw = HudBatch::text_width(&self.gui.atlas, pause_label) * scale;
            let th = 8.0 * scale;
            let tx = (bx + (bw - tw) * 0.5).floor();
            let ty = (by + (bh - th) * 0.5).floor();
            batch.text_scaled(
                &self.gui.atlas,
                pause_label,
                tx,
                ty,
                scale,
                TEXT_WHITE,
                true,
            );
        }
    }

    /// Persistent vanilla 9-slot Hotbar anchored at the bottom of the touch screen.
    fn draw_3ds_persistent_hotbar(&mut self, batch: &mut HudBatch, lay: &Bottom3dsLayout) {
        let s = lay.hotbar_scale;
        if self
            .gui
            .atlas
            .sprites
            .contains_key("gui/sprites/hud/hotbar")
        {
            batch.sprite_scaled(
                &self.gui.atlas,
                "gui/sprites/hud/hotbar",
                lay.hotbar_x,
                lay.hotbar_y,
                lay.hotbar_w,
                lay.hotbar_h,
                u32::MAX,
            );
        } else {
            self.draw_3ds_panel(
                batch,
                lay.hotbar_x,
                lay.hotbar_y,
                lay.hotbar_w,
                lay.hotbar_h,
            );
        }

        // Persistent touch hotbar item slots
        let item_sz = 17.5 * s;
        for col in 0..9 {
            let sx = lay.hotbar_x + (1.0 + col as f32 * 20.0) * s;
            let sy = lay.hotbar_y + 1.0 * s;
            let slot_sz = 20.0 * s;
            self.register_focus(sx, sy, slot_sz, slot_sz, Some(col));
            if let Some(stack) = self.slot_view(col) {
                let ix = sx + (slot_sz - item_sz) * 0.5;
                let iy = sy + (slot_sz - item_sz) * 0.5;
                self.draw_touch_slot_item(batch, col, &stack, ix, iy, item_sz);
            }
        }

        // Selection frame (Legacy4J `legacy:hud/hotbar_selection`) for held weapon/hotbar slot
        let sel_x = lay.hotbar_x - s + (self.selected as f32) * (20.0 * s);
        let sel_y = lay.hotbar_y - s;
        let sel_sprite = if self
            .gui
            .atlas
            .sprites
            .contains_key("legacy/hud/hotbar_selection")
        {
            "legacy/hud/hotbar_selection"
        } else {
            "gui/sprites/hud/hotbar_selection"
        };
        if self.gui.atlas.sprites.contains_key(sel_sprite) {
            batch.sprite_scaled(
                &self.gui.atlas,
                sel_sprite,
                sel_x,
                sel_y,
                24.0 * s,
                24.0 * s,
                u32::MAX,
            );
        }
    }

    fn map_tab_metrics(x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32, f32) {
        let map_size = (h - 4.0).min(w - 116.0).max(68.0);
        let map_x = x + 4.0;
        let map_y = y + ((h - map_size) / 2.0).floor();
        let info_panel_x = map_x + map_size + 6.0;
        let info_panel_w = (x + w - info_panel_x - 2.0).max(68.0);
        (map_x, map_y, map_size, info_panel_x, info_panel_w)
    }

    fn map_right_layout(panel_x: f32, panel_y: f32, panel_w: f32, panel_h: f32) -> MapRightLayout {
        let rx = panel_x + 4.0;
        let rw = (panel_w - 8.0).max(52.0);
        let pad_y = if panel_h >= 176.0 { 4.0 } else { 3.0 };
        let ry0 = panel_y + pad_y;
        let rh = (panel_h - pad_y * 2.0).max(56.0);
        let gap = if rh >= 178.0 {
            3.0
        } else if rh >= 146.0 {
            2.0
        } else {
            1.0
        };
        let usable = (rh - 7.0 * gap).max(48.0);
        let row_h = (usable / 8.75).floor().clamp(6.0, 20.0);
        let biome_h = (usable - 7.0 * row_h).max(row_h);

        let clock_y = ry0;
        let clock_h = row_h;
        let heading_y = clock_y + clock_h + gap;
        let heading_h = row_h;
        let coord_ys = [
            heading_y + heading_h + gap,
            heading_y + heading_h + gap + row_h + gap,
            heading_y + heading_h + gap + (row_h + gap) * 2.0,
        ];
        let coord_h = row_h;
        let biome_y = coord_ys[2] + coord_h + gap;
        let fps_box_y = biome_y + biome_h + gap;
        let fps_box_h = row_h;
        let zoom_y = fps_box_y + fps_box_h + gap;
        let zoom_h = row_h;

        let step_w = (rw * 0.24).floor().clamp(18.0, 26.0);
        let minus_x = rx;
        let readout_x = rx + step_w + 3.0;
        let readout_w = (rw - step_w * 2.0 - 6.0).max(24.0);
        let plus_x = rx + rw - step_w;

        MapRightLayout {
            rx,
            rw,
            clock_y,
            clock_h,
            heading_y,
            heading_h,
            coord_ys,
            coord_h,
            biome_y,
            biome_h,
            fps_box_y,
            fps_box_h,
            zoom_y,
            zoom_h,
            minus_x,
            step_w,
            readout_x,
            readout_w,
            plus_x,
        }
    }

    fn clock_frame_index(day_time: i64) -> usize {
        let ticks = day_time.rem_euclid(24_000);
        (((ticks - 6_000).rem_euclid(24_000) as f32 / 24_000.0) * 64.0).round() as usize % 64
    }

    fn compass_heading_info(yaw: f32) -> (usize, &'static str) {
        let y = yaw.rem_euclid(360.0);
        let frame = ((y / 360.0) * 32.0).round() as usize % 32;
        let name = match ((y + 22.5) / 45.0) as usize % 8 {
            0 => "South",
            1 => "Southwest",
            2 => "West",
            3 => "Northwest",
            4 => "North",
            5 => "Northeast",
            6 => "East",
            _ => "Southeast",
        };
        (frame, name)
    }

    /// Tab 1 (Default): Touch-Oriented Map + Modern Telemetry Cards & Controls.
    fn draw_3ds_map_tab(&mut self, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
        let (map_x, map_y, map_size, info_panel_x, info_panel_w) =
            Self::map_tab_metrics(x, y, w, h);

        // Tap-to-zoom focus target covers the parchment map square
        self.register_focus(map_x, map_y, map_size, map_size, None);

        // Left parchment minimap
        self.draw_lce_content_panel(
            batch,
            map_x - 2.0,
            map_y - 2.0,
            map_size + 4.0,
            map_size + 4.0,
        );
        if self.gui.atlas.sprites.contains_key("map/map_background") {
            batch.sprite_scaled(
                &self.gui.atlas,
                "map/map_background",
                map_x,
                map_y,
                map_size,
                map_size,
                u32::MAX,
            );
        } else {
            batch.fill(
                &self.gui.atlas,
                map_x,
                map_y,
                map_size,
                map_size,
                rgba(212, 190, 146, 255),
            );
        }

        let pad = (map_size * 0.06).max(4.0);
        let inner = map_size - pad * 2.0;
        batch.fill(
            &self.gui.atlas,
            map_x + pad - 1.0,
            map_y + pad - 1.0,
            inner + 2.0,
            inner + 2.0,
            rgba(30, 28, 24, 255),
        );
        if self.gui.atlas.sprites.contains_key(MINIMAP_SPRITE) {
            batch.sprite_scaled(
                &self.gui.atlas,
                MINIMAP_SPRITE,
                map_x + pad,
                map_y + pad,
                inner,
                inner,
                u32::MAX,
            );
        } else {
            batch.fill(
                &self.gui.atlas,
                map_x + pad,
                map_y + pad,
                inner,
                inner,
                rgba(64, 92, 64, 255),
            );
        }

        let cx = map_x + map_size / 2.0;
        let cy = map_y + map_size / 2.0;
        batch.text(
            &self.gui.atlas,
            "N",
            cx - 3.0,
            map_y + pad + 2.0,
            TEXT_WHITE,
            true,
        );
        batch.text(
            &self.gui.atlas,
            "S",
            cx - 3.0,
            map_y + map_size - pad - 10.0,
            TEXT_WHITE,
            true,
        );
        batch.text(
            &self.gui.atlas,
            "W",
            map_x + pad + 2.0,
            cy - 4.0,
            TEXT_WHITE,
            true,
        );
        batch.text(
            &self.gui.atlas,
            "E",
            map_x + map_size - pad - 8.0,
            cy - 4.0,
            TEXT_WHITE,
            true,
        );

        if let Some(r) = self
            .gui
            .atlas
            .sprites
            .get("map/decorations/player")
            .copied()
        {
            let arrow = 180.0 + self.yaw;
            let (sin_a, cos_a) = (arrow + 180.0).to_radians().sin_cos();
            let pt =
                |dx: f32, dy: f32| [cx + dx * cos_a - dy * sin_a, cy + dx * sin_a + dy * cos_a];
            let hs = 4.5;
            batch.quad(
                [pt(-hs, hs), pt(hs, hs), pt(hs, -hs), pt(-hs, -hs)],
                [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
                u32::MAX,
            );
        }

        // Right-hand redesigned 8-card telemetry column
        let panel_y = map_y - 2.0;
        let panel_h = map_size + 4.0;
        self.draw_lce_content_panel(batch, info_panel_x, panel_y, info_panel_w, panel_h);
        let rm = Self::map_right_layout(info_panel_x, panel_y, info_panel_w, panel_h);

        let icon_sz = (rm.clock_h - 5.0).clamp(9.0, 12.0);
        let icon_x = rm.rx + 5.0;
        let text_left = icon_x + icon_sz + 4.0;
        let right_edge = rm.rx + rm.rw - 5.0;
        let row_text_scale = if rm.clock_h < 15.0 { 0.72 } else { 0.85 };

        // Card 1: Live Minecraft Clock + Day N | HH:MM
        self.draw_slate_card(batch, rm.rx, rm.clock_y, rm.rw, rm.clock_h, None);
        let clock_frame = format!(
            "item/clock_{:02}",
            Self::clock_frame_index(self.world.day_time)
        );
        let clock_sprite = if self.gui.atlas.sprites.contains_key(&clock_frame) {
            Some(clock_frame.as_str())
        } else if self.gui.atlas.sprites.contains_key("item/clock_00") {
            Some("item/clock_00")
        } else if self.gui.atlas.sprites.contains_key("item/clock") {
            Some("item/clock")
        } else {
            None
        };
        if let Some(sp) = clock_sprite {
            batch.sprite_scaled(
                &self.gui.atlas,
                sp,
                icon_x,
                rm.clock_y + (rm.clock_h - icon_sz) * 0.5,
                icon_sz,
                icon_sz,
                u32::MAX,
            );
        }
        let ticks = self.world.day_time.rem_euclid(24_000);
        let day = self.world.day_time.div_euclid(24_000);
        let hour = ((ticks / 1000) + 6) % 24;
        let min = ((ticks % 1000) * 60) / 1000;
        let is_day = (6..18).contains(&hour);
        let day_str = format!("Day {day}");
        let clock_str = format!("{hour:02}:{min:02}");
        let clock_col = if is_day {
            rgba(255, 214, 102, 255)
        } else {
            rgba(164, 188, 255, 255)
        };
        let a = &self.gui.atlas;
        let avail_clock_w = (right_edge - text_left).max(16.0);
        let raw_day_w = (HudBatch::text_width(a, &day_str) - 1.0).max(1.0);
        let raw_time_w = (HudBatch::text_width(a, &clock_str) - 1.0).max(1.0);
        let min_sep = 7.0;
        let clock_scale = ((avail_clock_w - min_sep) / (raw_day_w + raw_time_w))
            .min(row_text_scale)
            .clamp(0.48, row_text_scale);
        let day_w = raw_day_w * clock_scale;
        let time_w = raw_time_w * clock_scale;
        let day_y = rm.clock_y + (rm.clock_h - 7.0 * clock_scale) * 0.5;
        let clock_x = right_edge - time_w;
        batch.text_scaled(
            a,
            &day_str,
            text_left,
            day_y,
            clock_scale,
            rgba(214, 220, 232, 255),
            false,
        );
        let gap_start = text_left + day_w;
        if clock_x - gap_start >= 4.0 {
            let sep_x = ((gap_start + clock_x) * 0.5).floor();
            let sep_h = (8.0 * clock_scale).round().max(5.0);
            let sep_y = rm.clock_y + (rm.clock_h - sep_h) * 0.5;
            batch.fill(a, sep_x, sep_y, 1.0, sep_h, rgba(126, 134, 148, 255));
        }
        batch.text_scaled(a, &clock_str, clock_x, day_y, clock_scale, clock_col, false);

        // Card 2: Rotating Compass + Full Heading Name
        self.draw_slate_card(batch, rm.rx, rm.heading_y, rm.rw, rm.heading_h, None);
        let (compass_idx, heading_name) = Self::compass_heading_info(self.yaw);
        let compass_frame = format!("item/compass_{compass_idx:02}");
        let compass_sp = if self.gui.atlas.sprites.contains_key(&compass_frame) {
            Some(compass_frame.as_str())
        } else if self.gui.atlas.sprites.contains_key("item/compass_16") {
            Some("item/compass_16")
        } else if self.gui.atlas.sprites.contains_key("item/compass") {
            Some("item/compass")
        } else {
            None
        };
        if let Some(sp) = compass_sp {
            batch.sprite_scaled(
                &self.gui.atlas,
                sp,
                icon_x,
                rm.heading_y + (rm.heading_h - icon_sz) * 0.5,
                icon_sz,
                icon_sz,
                u32::MAX,
            );
        }
        let a = &self.gui.atlas;
        let heading_raw_w = (HudBatch::text_width(a, heading_name) - 1.0).max(1.0);
        let heading_scale = ((right_edge - text_left).max(16.0) / heading_raw_w)
            .min(row_text_scale)
            .clamp(0.55, row_text_scale);
        let heading_y = rm.heading_y + (rm.heading_h - 7.0 * heading_scale) * 0.5;
        batch.text_scaled(
            a,
            heading_name,
            text_left,
            heading_y,
            heading_scale,
            TEXT_WHITE,
            false,
        );

        // Cards 3-5: Stacked Color-Accented X / Y / Z Rows
        let [px, py, pz] = self.player_position();
        let axes = [
            ("X", px.floor() as i32, rgba(255, 118, 118, 255)),
            ("Y", py.floor() as i32, rgba(116, 227, 138, 255)),
            ("Z", pz.floor() as i32, rgba(116, 192, 252, 255)),
        ];
        for (i, (axis, val, col)) in axes.into_iter().enumerate() {
            let cy = rm.coord_ys[i];
            self.draw_slate_card(batch, rm.rx, cy, rm.rw, rm.coord_h, Some(col));
            let a = &self.gui.atlas;
            let lbl_scale = (row_text_scale * 0.92).clamp(0.58, 0.82);
            let lbl_x = rm.rx + 8.0;
            let lbl_w = HudBatch::text_width(a, axis) * lbl_scale;
            let val_s = format!("{val}");
            let val_raw_w = (HudBatch::text_width(a, &val_s) - 1.0).max(1.0);
            let avail_val_w = (right_edge - (lbl_x + lbl_w + 6.0)).max(16.0);
            let val_scale = (avail_val_w / val_raw_w)
                .min(row_text_scale)
                .clamp(0.52, row_text_scale);
            let vw = val_raw_w * val_scale;
            let ly = cy + (rm.coord_h - 7.0 * lbl_scale) * 0.5;
            let vy = cy + (rm.coord_h - 7.0 * val_scale) * 0.5;
            batch.text_scaled(a, axis, lbl_x, ly, lbl_scale, col, false);
            batch.text_scaled(a, &val_s, right_edge - vw, vy, val_scale, TEXT_WHITE, false);
        }

        // Card 6: Dimension Diorama + Non-Overlapping Biome & Dimension Label
        let (_, biome_title) = self.current_biome_id_and_name();
        let (dim_name, dim_col) = self.current_dimension_info();
        let dim_id = self.dim.id();
        self.draw_slate_card(batch, rm.rx, rm.biome_y, rm.rw, rm.biome_h, None);
        let card = BiomeCardLayout::compute(
            rm.rx,
            rm.biome_y,
            rm.rw,
            rm.biome_h,
            &biome_title,
            dim_name,
            |s| HudBatch::text_width(&self.gui.atlas, s),
        );
        if let Some([ix, iy, iw, _ih]) = card.icon_rect {
            self.draw_dimension_thumbnail(batch, ix, iy, iw, dim_id, dim_col);
        }
        for (line, ly, ls) in &card.biome_lines {
            batch.text_scaled(
                &self.gui.atlas,
                line,
                card.text_x,
                *ly,
                *ls,
                TEXT_WHITE,
                false,
            );
        }
        if let Some((dim_txt, dy, ds)) = &card.dim_line {
            batch.text_scaled(
                &self.gui.atlas,
                dim_txt,
                card.text_x,
                *dy,
                *ds,
                dim_col,
                false,
            );
        }

        // Card 7: FPS Telemetry Card
        let reading = self.frame_reading().0;
        let base_fps = if reading > 0 {
            reading
        } else {
            self.fps.round().max(0.0) as u32
        };
        let fg_share = self
            .renderer
            .as_ref()
            .and_then(|r| r.frame_generation_share())
            .filter(|&s| s > 0.01);
        let fps = fg_share.map_or(base_fps, |s| ((base_fps as f32) * (1.0 + s)).round() as u32);
        let fps_col = if fps >= 55 {
            rgba(116, 227, 138, 255)
        } else if fps >= 30 {
            rgba(255, 214, 102, 255)
        } else {
            rgba(255, 118, 118, 255)
        };
        self.draw_slate_card(batch, rm.rx, rm.fps_box_y, rm.rw, rm.fps_box_h, None);
        let fps_scale = row_text_scale;
        let fy = rm.fps_box_y + (rm.fps_box_h - 7.0 * fps_scale) * 0.5;
        let fps_label = if fg_share.is_some() {
            "FPS (FG)"
        } else {
            "FPS"
        };
        batch.text_scaled(
            &self.gui.atlas,
            fps_label,
            rm.rx + 7.0,
            fy,
            fps_scale,
            rgba(188, 194, 208, 255),
            false,
        );
        let fps_val = format!("{fps}");
        let fvw = (HudBatch::text_width(&self.gui.atlas, &fps_val) - 1.0).max(1.0) * fps_scale;
        batch.text_scaled(
            &self.gui.atlas,
            &fps_val,
            right_edge - fvw,
            fy,
            fps_scale,
            fps_col,
            false,
        );

        // Card 8: Zoom Controls (`-` | `[2x ••••]` | `+`)
        self.draw_3ds_button_scaled(
            batch, rm.minus_x, rm.zoom_y, rm.step_w, rm.zoom_h, "-", false, 1.0,
        );
        self.draw_slate_card(
            batch,
            rm.readout_x,
            rm.zoom_y,
            rm.readout_w,
            rm.zoom_h,
            None,
        );
        self.register_focus_button(rm.readout_x, rm.zoom_y, rm.readout_w, rm.zoom_h);
        let z_str = format!("{}x", self.shulkds.dashboard.map_zoom);
        let z_scale = if rm.zoom_h < 15.0 { 0.72 } else { 0.85 };
        let zw = (HudBatch::text_width(&self.gui.atlas, &z_str) - 1.0).max(1.0) * z_scale;
        batch.text_scaled(
            &self.gui.atlas,
            &z_str,
            rm.readout_x + (rm.readout_w - zw) * 0.5,
            rm.zoom_y + (rm.zoom_h - 7.0 * z_scale) * 0.5 - 1.5,
            z_scale,
            TEXT_WHITE,
            false,
        );
        // 4 subtle zoom-tier pips along bottom of zoom readout
        let tier = if self.shulkds.dashboard.map_zoom < 1.0 {
            0usize
        } else if self.shulkds.dashboard.map_zoom < 1.5 {
            1
        } else if self.shulkds.dashboard.map_zoom < 3.0 {
            2
        } else {
            3
        };
        let pip_w = 4.0;
        let pip_gap = 2.0;
        let pips_w = 4.0 * pip_w + 3.0 * pip_gap;
        let pip_x0 = rm.readout_x + (rm.readout_w - pips_w) * 0.5;
        let pip_y = rm.zoom_y + rm.zoom_h - 3.5;
        for p in 0..4 {
            let pcol = if p == tier {
                rgba(116, 227, 138, 255)
            } else {
                rgba(76, 84, 100, 255)
            };
            batch.fill(
                &self.gui.atlas,
                pip_x0 + p as f32 * (pip_w + pip_gap),
                pip_y,
                pip_w,
                1.5,
                pcol,
            );
        }
        self.draw_3ds_button_scaled(
            batch, rm.plus_x, rm.zoom_y, rm.step_w, rm.zoom_h, "+", false, 1.0,
        );
    }

    /// Geometry shared between draw and tap for the Concept-matched Dual-Screen Survival Inventory.
    fn survival_inv_layout(&self, x: f32, y: f32, w: f32, h: f32) -> SurvivalInvLayout {
        let include_hotbar_row =
            self.settings.extras.shulkds_hotbar == HotbarPosition::TopScreenClassic;
        let grid_rows = if include_hotbar_row { 5.2 } else { 4.0 };
        let slot_size = ((h - 30.0) / grid_rows)
            .floor()
            .min(((w - 4.0) / 11.0).floor())
            .clamp(12.0, 28.0);
        let hotbar_gap = if include_hotbar_row {
            (slot_size * 0.2).round().max(2.0)
        } else {
            0.0
        };
        let grid_h = if include_hotbar_row {
            5.0 * slot_size + hotbar_gap
        } else {
            4.0 * slot_size
        };
        let player_w = (w - 4.0 - 10.0 * slot_size)
            .floor()
            .clamp(slot_size, (slot_size * 1.6).floor());
        let player_h = grid_h;
        let total_w = 10.0 * slot_size + player_w + 4.0;
        let start_x = x + ((w - total_w) * 0.5).round().max(0.0);

        let tray_h = (h - grid_h - 8.0).clamp(20.0, 32.0);
        let block_h = grid_h + 5.0 + tray_h;
        let start_y = y + ((h - block_h) * 0.5).floor().max(1.0);

        let armor_x = start_x;
        let armor_y0 = start_y;
        let player_x = armor_x + slot_size + 2.0;
        let player_y = start_y;
        let bag_x0 = player_x + player_w + 2.0;
        let bag_y0 = start_y + slot_size;
        let hotbar_row_y = bag_y0 + 3.0 * slot_size + hotbar_gap;

        // Top-right shelf sits strictly above the 9 backpack columns (`bag_x0..bag_x0 + 9 * slot_size`)
        let offhand_x = bag_x0;
        let offhand_y = start_y;
        let offhand_sz = slot_size;

        let info_y = start_y;
        let info_h = (slot_size - 3.0).max(12.0);
        let shelf_right = bag_x0 + 9.0 * slot_size;
        let info_x = offhand_x + offhand_sz + 3.0;
        let info_w = (shelf_right - info_x).max(40.0);

        // Recessed grey action tray below the inventory grid, inside the outer white panel
        let grid_bottom = start_y + grid_h;
        let tray_x = start_x;
        let tray_w = total_w;
        let tray_y = grid_bottom + 5.0;
        let act_pad_x = 6.0;
        let act_x = tray_x + act_pad_x;
        let act_w = (tray_w - act_pad_x * 2.0).max(40.0);
        let act_h = (tray_h - 10.0).clamp(14.0, 24.0);
        let act_y = tray_y + ((tray_h - act_h) * 0.5).floor();

        SurvivalInvLayout {
            slot_size,
            armor_x,
            armor_y0,
            player_x,
            player_y,
            player_w,
            player_h,
            bag_x0,
            bag_y0,
            include_hotbar_row,
            hotbar_row_y,
            offhand_x,
            offhand_y,
            offhand_sz,
            info_x,
            info_y,
            info_w,
            info_h,
            tray_x,
            tray_y,
            tray_w,
            tray_h,
            act_x,
            act_y,
            act_w,
            act_h,
        }
    }

    fn surv_action_buttons(
        &self,
        bar_x: f32,
        bar_w: f32,
        is_creative_surv: bool,
    ) -> [(f32, f32, &'static str, &'static str, bool); 4] {
        let quick = self.shulkds.dashboard.quick_move_mode;
        let gap = 4.0;
        let avail = (bar_w - gap * 3.0).max(40.0);
        let bw = (avail / 4.0).floor();
        let last_w = (bar_w - 3.0 * (bw + gap)).max(16.0);
        let q_lbl = if quick { "Quick: ON" } else { "Quick" };
        let last = if is_creative_surv {
            ("clear_bag", "Clear", false)
        } else {
            ("sort", "Sort", false)
        };
        [
            ("quick", q_lbl, bar_x, bw, quick),
            ("split", "Split", bar_x + (bw + gap), bw, false),
            ("drop", "Drop", bar_x + 2.0 * (bw + gap), bw, false),
            (last.0, last.1, bar_x + 3.0 * (bw + gap), last_w, last.2),
        ]
        .map(|(id, lbl, bx, w, hl)| (bx, w, id, lbl, hl))
    }

    /// Minecraft-style hover tooltip frame: dark indigo interior with notched corners and a
    /// lighter blue-indigo inner border gradient.
    fn draw_mc_tooltip_box(&self, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
        if w < 6.0 || h < 6.0 {
            return;
        }
        let a = &self.gui.atlas;
        let bg = rgba(16, 4, 24, 242);
        batch.fill(a, x + 1.0, y, w - 2.0, 1.0, bg);
        batch.fill(a, x, y + 1.0, w, (h - 2.0).max(0.0), bg);
        batch.fill(a, x + 1.0, y + h - 1.0, w - 2.0, 1.0, bg);

        let ix = x + 1.0;
        let iy = y + 1.0;
        let iw = (w - 2.0).max(2.0);
        let ih = (h - 2.0).max(2.0);
        let top_col = rgba(80, 48, 255, 230);
        let mid_top = rgba(68, 40, 215, 230);
        let mid_bot = rgba(48, 24, 158, 230);
        let bot_col = rgba(40, 20, 138, 230);
        batch.fill(a, ix, iy, iw, 1.0, top_col);
        batch.fill(a, ix, iy + ih - 1.0, iw, 1.0, bot_col);
        let inner_h = (ih - 2.0).max(0.0);
        let top_half = (inner_h * 0.5).floor();
        let bot_half = (inner_h - top_half).max(0.0);
        batch.fill(a, ix, iy + 1.0, 1.0, top_half, mid_top);
        batch.fill(a, ix + iw - 1.0, iy + 1.0, 1.0, top_half, mid_top);
        batch.fill(a, ix, iy + 1.0 + top_half, 1.0, bot_half, mid_bot);
        batch.fill(
            a,
            ix + iw - 1.0,
            iy + 1.0 + top_half,
            1.0,
            bot_half,
            mid_bot,
        );
    }

    fn selected_inventory_item_display(&mut self) -> (String, u32) {
        let slot_stack;
        let stack = if let Some(carried) = self.carried.as_ref() {
            carried
        } else {
            let slot = self.shulkds.dashboard.active_slot.unwrap_or(self.selected);
            slot_stack = self.slot_view(slot);
            let Some(s) = slot_stack.as_ref() else {
                return ("Empty".to_owned(), rgba(154, 162, 186, 255));
            };
            s
        };
        let name =
            horizon_client::item_tooltip::name_line(stack, &|key| self.pack.names.display_get(key));
        let default_rarity = self.item_rarity(&stack.item);
        let rarity = horizon_client::item_tooltip::rarity(stack, &default_rarity);
        let color = match rarity {
            "uncommon" => rgba(255, 255, 85, 255),
            "rare" => rgba(85, 255, 255, 255),
            "epic" => rgba(255, 85, 255, 255),
            _ => TEXT_WHITE,
        };
        (name, color)
    }

    fn draw_3ds_inventory_tab(
        &mut self,
        batch: &mut HudBatch,
        lay: &Bottom3dsLayout,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        self.draw_3ds_survival_inventory_body(batch, lay, x, y, w, h, self.creative);
    }

    /// Classic Concept-Matched Dual-Screen Survival Inventory:
    /// Left: 4 vertical Armor slots + tall Black 3D Player Preview Box.
    /// Right: Contiguous 3x9 Main Inventory Grid + Top-Right Offhand/Inspector/Action Shelf.
    #[allow(clippy::too_many_arguments)]
    fn draw_3ds_survival_inventory_body(
        &mut self,
        batch: &mut HudBatch,
        lay: &Bottom3dsLayout,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        is_creative_surv: bool,
    ) {
        let m = self.survival_inv_layout(x, y, w, h);

        // 1. 4 Vertical Armor Slots (Helmet, Chest, Legs, Boots)
        let armor_hints = [
            "gui/sprites/container/slot/helmet",
            "gui/sprites/container/slot/chestplate",
            "gui/sprites/container/slot/leggings",
            "gui/sprites/container/slot/boots",
        ];
        for i in 0..4 {
            let sx = m.armor_x;
            let sy = m.armor_y0 + i as f32 * m.slot_size;
            self.draw_3ds_interactive_slot(
                batch,
                500 + i,
                sx,
                sy,
                m.slot_size,
                false,
                Some(armor_hints[i]),
            );
        }

        // 3. Left Panel: Tall Sunken Black 3D Player Preview Box (tap to auto-equip)
        let (px, py, pw, ph) = (m.player_x, m.player_y, m.player_w, m.player_h);
        {
            let a = &self.gui.atlas;
            let dark = rgba(55, 55, 55, 255);
            let light = rgba(255, 255, 255, 255);
            batch.fill(a, px, py, pw, 1.0, dark);
            batch.fill(a, px, py, 1.0, ph, dark);
            batch.fill(a, px, py + ph - 1.0, pw, 1.0, light);
            batch.fill(a, px + pw - 1.0, py, 1.0, ph, light);
            batch.fill(
                a,
                px + 1.0,
                py + 1.0,
                pw - 2.0,
                ph - 2.0,
                rgba(0, 0, 0, 255),
            );
        }
        let player_scale = (m.player_h * 0.36).min(m.player_w * 0.75).clamp(18.0, 38.0);
        let target_slot = self.shulkds.dashboard.active_slot.unwrap_or(self.selected);
        let target_center = match target_slot {
            0..=8 => {
                if m.include_hotbar_row {
                    (
                        m.bag_x0 + (target_slot as f32 + 0.5) * m.slot_size,
                        m.hotbar_row_y + 0.5 * m.slot_size,
                    )
                } else {
                    (
                        lay.hotbar_x + (11.0 + target_slot as f32 * 20.0) * lay.hotbar_scale,
                        lay.hotbar_y + 11.0 * lay.hotbar_scale,
                    )
                }
            }
            9..=35 => {
                let idx = target_slot - 9;
                let row = idx / 9;
                let col = idx % 9;
                (
                    m.bag_x0 + (col as f32 + 0.5) * m.slot_size,
                    m.bag_y0 + (row as f32 + 0.5) * m.slot_size,
                )
            }
            500..=503 => {
                let i = target_slot - 500;
                (
                    m.armor_x + m.slot_size * 0.5,
                    m.armor_y0 + (i as f32 + 0.5) * m.slot_size,
                )
            }
            504 => (
                m.offhand_x + m.offhand_sz * 0.5,
                m.offhand_y + m.offhand_sz * 0.5,
            ),
            _ => (px + pw * 0.5, py + ph * 0.35),
        };
        let saved_cursor = self.cursor;
        self.cursor = target_center;
        self.draw_player_preview(
            batch,
            px + 2.0,
            py + 2.0,
            px + pw - 2.0,
            py + ph - 2.0,
            player_scale,
            0.0625,
        );
        self.cursor = saved_cursor;

        // 3. Top-Right Shelf (above 3x9 backpack): Offhand/Shield + Selected Item Tooltip Box
        self.draw_3ds_interactive_slot(
            batch,
            504,
            m.offhand_x,
            m.offhand_y,
            m.offhand_sz,
            false,
            Some("gui/sprites/container/slot/shield"),
        );
        self.draw_mc_tooltip_box(batch, m.info_x, m.info_y, m.info_w, m.info_h);
        {
            let (mut item_name, item_col) = self.selected_inventory_item_display();
            let a = &self.gui.atlas;
            let max_w = (m.info_w - 10.0).max(16.0);
            let min_scale = 0.72f32;
            let mut raw_w = (HudBatch::text_width(a, &item_name) - 1.0).max(1.0);
            if raw_w * min_scale > max_w {
                while !item_name.is_empty()
                    && (HudBatch::text_width(a, &format!("{item_name}...")) - 1.0) * min_scale
                        > max_w
                {
                    item_name.pop();
                }
                item_name.push_str("...");
                raw_w = (HudBatch::text_width(a, &item_name) - 1.0).max(1.0);
            }
            let scale = (max_w / raw_w).clamp(min_scale, 1.0);
            let tx = (m.info_x + (m.info_w - raw_w * scale) * 0.5).floor();
            let ty = (m.info_y + (m.info_h - 8.0 * scale) * 0.5).floor();
            batch.text_scaled(a, &item_name, tx, ty, scale, item_col, true);
        }

        // 4. Recessed grey action tray + buttons inside outer white panel below inventory grid
        self.draw_lce_content_panel(batch, m.tray_x, m.tray_y, m.tray_w, m.tray_h);
        for (bx, bw, _, lbl, hl) in self.surv_action_buttons(m.act_x, m.act_w, is_creative_surv) {
            self.draw_3ds_button_scaled(batch, bx, m.act_y, bw, m.act_h, lbl, hl, 0.95);
        }

        // 4. Contiguous 3x9 Main Backpack Grid (slots 9..36)
        self.draw_3ds_inventory_grid(
            batch,
            m.bag_x0,
            m.bag_y0,
            m.slot_size,
            m.slot_size,
            m.slot_size,
            m.slot_size,
        );
        if m.include_hotbar_row {
            for col in 0..9 {
                let sx = m.bag_x0 + col as f32 * m.slot_size;
                self.draw_3ds_interactive_slot(
                    batch,
                    col,
                    sx,
                    m.hotbar_row_y,
                    m.slot_size,
                    col == self.selected,
                    None,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_3ds_inventory_grid(
        &mut self,
        batch: &mut HudBatch,
        gx: f32,
        gy: f32,
        col_step: f32,
        row_step: f32,
        slot_w: f32,
        slot_h: f32,
    ) {
        for row in 0..3 {
            for col in 0..9 {
                let slot_idx = 9 + (row * 9 + col);
                let sx = gx + (col as f32 * col_step);
                let sy = gy + (row as f32 * row_step);
                self.draw_3ds_interactive_slot_rect(
                    batch, slot_idx, sx, sy, slot_w, slot_h, false, None,
                );
            }
        }
    }

    fn draw_slate_card(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        left_accent: Option<u32>,
    ) {
        let a = &self.gui.atlas;
        batch.fill(a, x, y, w, h, rgba(18, 18, 20, 255));
        batch.fill(
            a,
            x + 1.0,
            y + 1.0,
            (w - 2.0).max(0.0),
            (h - 2.0).max(0.0),
            rgba(36, 36, 38, 238),
        );
        batch.fill(
            a,
            x + 1.0,
            y + 1.0,
            (w - 2.0).max(0.0),
            1.0,
            rgba(72, 72, 76, 255),
        );
        if let Some(col) = left_accent {
            batch.fill(a, x + 1.0, y + 1.0, 3.0, (h - 2.0).max(0.0), col);
        }
    }

    fn settings_tab_metrics(x: f32, y: f32, w: f32, h: f32) -> SettingsTabMetrics {
        let compact = h < 122.0;
        let show_header = !compact;
        let header_y = y + 1.0;
        let header_h = 16.0;
        let all_w = (w * 0.30).floor().clamp(66.0, 92.0);
        let all_opts_rect = [x + w - all_w - 2.0, header_y, all_w, header_h];

        let row_x = x + 2.0;
        let row_w = (w - 4.0).max(60.0);
        let rows_y0 = if show_header {
            y + header_h + 4.0
        } else {
            y + 1.0
        };
        let avail_h = (y + h - rows_y0 - 2.0).max(74.0);
        let row_step = (avail_h / 7.0).floor().clamp(10.0, 22.0);
        let row_h = (row_step - 2.0).max(9.0);

        let pad = 4.0;
        let label_x = row_x + pad;
        let label_w = (row_w * 0.28).floor().clamp(52.0, 82.0);
        let step_btn_w = if row_w < 230.0 { 15.0 } else { 18.0 };
        let val_w = if row_w < 230.0 { 30.0 } else { 36.0 };
        let val_x = row_x + row_w - pad - val_w;
        let minus_x = label_x + label_w + 3.0;
        let plus_x = val_x - 4.0 - step_btn_w;
        let track_x = minus_x + step_btn_w + 4.0;
        let track_w = (plus_x - 4.0 - track_x).max(24.0);

        let perf_y = rows_y0 + 4.0 * row_step + 1.0;
        let toggle_y = rows_y0 + 5.0 * row_step + 1.0;
        let hud_y = rows_y0 + 6.0 * row_step + 1.0;
        let toggle_h = (row_step - 2.0)
            .clamp(10.0, 20.0)
            .min((y + h - hud_y - 1.0).max(10.0));
        let gap = 4.0;
        let reach_w = ((row_w - gap) * 0.5).floor();
        let reach_x = row_x;
        let cam_x = reach_x + reach_w + gap;
        let cam_w = (row_w - reach_w - gap).max(24.0);

        SettingsTabMetrics {
            show_header,
            header_y,
            header_h,
            all_opts_rect,
            row_x,
            row_w,
            row_step,
            row_h,
            label_x,
            label_w,
            rows_y0,
            minus_x,
            step_btn_w,
            track_x,
            track_w,
            plus_x,
            val_x,
            val_w,
            perf_y,
            toggle_y,
            hud_y,
            toggle_h,
            reach_x,
            reach_w,
            cam_x,
            cam_w,
        }
    }

    fn settings_slider_norms(&self) -> [f32; 4] {
        [
            ((self.settings.render_distance - 2) as f32 / 30.0).clamp(0.0, 1.0),
            ((self.settings.fov - 30.0) / 80.0).clamp(0.0, 1.0),
            self.settings.gamma.clamp(0.0, 1.0),
            self.settings.master_volume.clamp(0.0, 1.0),
        ]
    }

    fn apply_settings_slider_norm(&mut self, slider_idx: usize, norm: f32) {
        let t = norm.clamp(0.0, 1.0);
        match slider_idx {
            0 => {
                let raw = (2.0 + t * 30.0).round() as i32;
                let stepped = ((raw + 1) / 2) * 2;
                self.settings.render_distance = stepped.clamp(2, 32);
                self.shulkds.dashboard.set_status(format!(
                    "Render Distance: {} Chunks",
                    self.settings.render_distance
                ));
            }
            1 => {
                let raw = (30.0 + t * 80.0).round();
                let stepped = (raw / 5.0).round() * 5.0;
                self.settings.fov = stepped.clamp(30.0, 110.0);
                self.shulkds
                    .dashboard
                    .set_status(format!("FOV: {}", self.settings.fov as i32));
            }
            2 => {
                self.settings.gamma = (t * 10.0).round() / 10.0;
                if let Some(r) = self.renderer.as_mut() {
                    r.gamma = self.settings.gamma;
                }
                self.shulkds.dashboard.set_status(format!(
                    "Brightness: {}%",
                    (self.settings.gamma * 100.0).round() as i32
                ));
            }
            _ => {
                self.settings.master_volume = (t * 10.0).round() / 10.0;
                self.shulkds.dashboard.set_status(format!(
                    "Volume: {}%",
                    (self.settings.master_volume * 100.0).round() as i32
                ));
            }
        }
    }

    /// Tab 3: Clean Quick Options with uniform slider cards, right value pills, and bottom toggles.
    fn draw_3ds_settings_tab(&mut self, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
        let m = Self::settings_tab_metrics(x, y, w, h);
        if m.show_header {
            let icon_sz = 12.0;
            self.draw_3ds_options_icon(
                batch,
                m.row_x + 2.0,
                m.header_y + (m.header_h - icon_sz) * 0.5,
                icon_sz,
            );
            batch.text(
                &self.gui.atlas,
                "Quick Options",
                m.row_x + icon_sz + 6.0,
                m.header_y + (m.header_h - 8.0) * 0.5,
                TEXT_DARK,
                false,
            );
            let [ax, ay, aw, ah] = m.all_opts_rect;
            self.draw_3ds_button_scaled(batch, ax, ay, aw, ah, "All Options...", false, 0.85);
        }

        let labels = ["Render Dist", "FOV", "Brightness", "Volume"];
        let values = [
            format!("{} ch", self.settings.render_distance),
            format!("{}\u{00b0}", self.settings.fov as i32),
            format!("{}%", (self.settings.gamma * 100.0).round() as i32),
            format!("{}%", (self.settings.master_volume * 100.0).round() as i32),
        ];
        let norms = self.settings_slider_norms();

        let max_lw = labels
            .iter()
            .map(|l| HudBatch::text_width(&self.gui.atlas, l))
            .fold(1.0, f32::max);
        let shared_label_scale = ((m.label_w - 4.0).max(16.0) / max_lw)
            .min((m.row_h - 4.0).max(6.0) / 8.0)
            .clamp(0.55, 0.92);

        let btn_inset = 2.0;
        let btn_h = (m.row_h - btn_inset * 2.0).max(8.0);
        let groove_h = (btn_h * 0.42).clamp(4.0, 6.0);
        let knob_w = 6.0;
        let knob_h = (btn_h - 1.0).clamp(8.0, 14.0);

        for i in 0..4 {
            let ry = m.rows_y0 + i as f32 * m.row_step;
            self.draw_slate_card(batch, m.row_x, ry, m.row_w, m.row_h, None);

            // Uniform label
            let label_y = ry + (m.row_h - 8.0 * shared_label_scale) * 0.5;
            batch.text_scaled(
                &self.gui.atlas,
                labels[i],
                m.label_x,
                label_y,
                shared_label_scale,
                rgba(220, 226, 238, 255),
                false,
            );

            // Step - / + buttons
            let by = ry + btn_inset;
            self.draw_3ds_button_scaled(batch, m.minus_x, by, m.step_btn_w, btn_h, "-", false, 0.9);
            self.draw_3ds_button_scaled(batch, m.plus_x, by, m.step_btn_w, btn_h, "+", false, 0.9);

            // Slim modern slider groove + progress bar + thumb
            let gy = ry + (m.row_h - groove_h) * 0.5;
            let a = &self.gui.atlas;
            batch.fill(a, m.track_x, gy, m.track_w, groove_h, rgba(18, 18, 20, 255));
            batch.fill(
                a,
                m.track_x + 1.0,
                gy + 1.0,
                (m.track_w - 2.0).max(0.0),
                (groove_h - 2.0).max(0.0),
                rgba(28, 28, 30, 255),
            );
            let inner_tw = (m.track_w - 2.0).max(2.0);
            let fill_w = (inner_tw * norms[i]).round();
            if fill_w > 0.0 {
                batch.fill(
                    a,
                    m.track_x + 1.0,
                    gy + 1.0,
                    fill_w,
                    (groove_h - 2.0).max(0.0),
                    rgba(96, 186, 96, 255),
                );
            }
            let kx = m.track_x + ((m.track_w - knob_w).max(0.0) * norms[i]).round();
            let ky = ry + (m.row_h - knob_h) * 0.5;
            batch.fill(a, kx, ky, knob_w, knob_h, rgba(18, 18, 20, 255));
            batch.fill(
                a,
                kx + 1.0,
                ky + 1.0,
                knob_w - 2.0,
                (knob_h - 2.0).max(0.0),
                rgba(242, 244, 248, 255),
            );

            // Right-aligned dark value pill
            let vy = by;
            let vh = btn_h;
            batch.fill(a, m.val_x, vy, m.val_w, vh, rgba(68, 68, 72, 255));
            batch.fill(
                a,
                m.val_x + 1.0,
                vy + 1.0,
                (m.val_w - 2.0).max(0.0),
                (vh - 2.0).max(0.0),
                rgba(24, 24, 26, 255),
            );
            let vw_raw = HudBatch::text_width(a, &values[i]).max(1.0);
            let vs = ((m.val_w - 4.0).max(8.0) / vw_raw)
                .min((vh - 2.0).max(6.0) / 8.0)
                .clamp(0.55, 0.85);
            let vx = m.val_x + (m.val_w - vw_raw * vs) * 0.5;
            let vty = vy + (vh - 8.0 * vs) * 0.5;
            batch.text_scaled(a, &values[i], vx, vty, vs, rgba(240, 245, 255, 255), false);
        }

        // Performance toggles: FSR 1.0 & Frame Generation
        let fsr = self.settings.horizon.fsr;
        let fsr_lbl = format!("FSR: {}", fsr.short_label());
        self.draw_3ds_button_scaled(
            batch,
            m.reach_x,
            m.perf_y,
            m.reach_w,
            m.toggle_h,
            &fsr_lbl,
            fsr != FsrUpscaling::Off,
            0.88,
        );
        let fg_on = self.settings.horizon.frame_generation != FrameGeneration::Off;
        let fg_lbl = if fg_on {
            if cfg!(target_os = "android") {
                "Frame Gen: 2x"
            } else {
                "Frame Gen: FSR 3.1"
            }
        } else {
            "Frame Gen: OFF"
        };
        self.draw_3ds_button_scaled(
            batch, m.cam_x, m.perf_y, m.cam_w, m.toggle_h, fg_lbl, fg_on, 0.88,
        );

        // Bottom 2-column Toggles: Reach-Around & Camera
        let reach_lbl = if self.settings.controller.reach_around {
            "Reach-Around: ON"
        } else {
            "Reach-Around: OFF"
        };
        self.draw_3ds_button_scaled(
            batch,
            m.reach_x,
            m.toggle_y,
            m.reach_w,
            m.toggle_h,
            reach_lbl,
            self.settings.controller.reach_around,
            0.88,
        );
        let cam_lbl = match self.camera_type {
            super::perspective::CameraType::FirstPerson => "Camera: 1st",
            super::perspective::CameraType::ThirdPersonBack => "Camera: 3rd Back",
            super::perspective::CameraType::ThirdPersonFront => "Camera: 3rd Front",
        };
        self.draw_3ds_button_scaled(
            batch,
            m.cam_x,
            m.toggle_y,
            m.cam_w,
            m.toggle_h,
            cam_lbl,
            !self.camera_type.first_person(),
            0.88,
        );

        let hotbar_pos = self.settings.extras.shulkds_hotbar;
        let hotbar_lbl = format!("Hotbar: {}", hotbar_pos.compact_label());
        self.draw_3ds_button_scaled(
            batch,
            m.reach_x,
            m.hud_y,
            m.reach_w,
            m.toggle_h,
            &hotbar_lbl,
            hotbar_pos != HotbarPosition::BottomScreenBottom,
            0.88,
        );
        let lid_sleep = self.settings.extras.shulkds_lid_sleep;
        let lid_lbl = format!("Lid Sleep: {}", lid_sleep.compact_label());
        self.draw_3ds_button_scaled(
            batch,
            m.cam_x,
            m.hud_y,
            m.cam_w,
            m.toggle_h,
            &lid_lbl,
            lid_sleep != LidSleepMode::Off,
            0.88,
        );
    }

    /// Look up the rendered slot at `(lx, ly)` relative to the bottom viewport using populated `focus_rects`.
    fn find_slot_at(&self, lx: f32, ly: f32) -> Option<usize> {
        let rect = self.shulkds_bottom_gui_rect();
        let gx = rect.x + lx;
        let gy = rect.y + ly;
        for (rect_bounds, slot_opt) in self
            .shulkds
            .dashboard
            .focus_rects
            .iter()
            .zip(self.shulkds.dashboard.focus_slots.iter())
        {
            if let Some(slot_idx) = slot_opt {
                let [fx, fy, fw, fh] = *rect_bounds;
                if gx >= fx && gx <= fx + fw && gy >= fy && gy <= fy + fh {
                    return Some(*slot_idx);
                }
            }
        }
        None
    }

    /// Smart dual-screen cadence: 120 Hz on touch, instant on discrete UI/inventory changes,
    /// 15 Hz on active Map telemetry, and 2 Hz idle so Display 2 never starves Display 0.
    pub(super) fn should_present_shulkds_bottom(&mut self, sec_size: [u32; 2]) -> bool {
        fn mix_bytes(h: &mut u64, bytes: &[u8]) {
            for &b in bytes {
                *h ^= u64::from(b);
                *h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        fn mix_opt_stack(h: &mut u64, stack: Option<&horizon_client::items::ItemStack>) {
            if let Some(st) = stack {
                mix_bytes(h, &st.count.to_le_bytes());
                mix_bytes(h, &st.damage.to_le_bytes());
                mix_bytes(h, st.item.as_bytes());
            } else {
                mix_bytes(h, &[0xff]);
            }
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        mix_bytes(&mut h, &sec_size[0].to_le_bytes());
        mix_bytes(&mut h, &sec_size[1].to_le_bytes());
        mix_bytes(
            &mut h,
            &[
                u8::from(self.title.is_some()),
                u8::from(self.remote_overlay_active()),
                u8::from(self.is_shulkds_paused()),
                self.shulkds.dashboard.active_tab as u8,
                self.selected as u8,
                u8::from(self.carried.is_some()),
                u8::from(self.shulkds.dashboard.quick_move_mode),
                self.settings.horizon.fsr as u8,
                self.settings.horizon.frame_generation as u8,
                u8::from(self.settings.controller.reach_around),
                self.camera_type as u8,
                self.settings.extras.shulkds_hotbar as u8,
                self.settings.extras.shulkds_lid_sleep as u8,
                u8::from(self.settings.extras.shulkds_classic_crafting),
                u8::from(self.settings.extras.shulkds_classic_inventory),
            ],
        );
        mix_bytes(&mut h, self.dim.id().as_bytes());
        mix_bytes(&mut h, &self.shulkds_update_sig().to_le_bytes());
        mix_opt_stack(&mut h, self.carried.as_ref());
        mix_bytes(
            &mut h,
            &(self.shulkds.dashboard.active_slot.unwrap_or(usize::MAX) as u64).to_le_bytes(),
        );
        mix_bytes(
            &mut h,
            &self.shulkds.dashboard.map_zoom.to_bits().to_le_bytes(),
        );
        mix_bytes(&mut h, &self.settings.render_distance.to_le_bytes());
        mix_bytes(&mut h, &self.settings.fov.to_bits().to_le_bytes());
        mix_bytes(
            &mut h,
            &((self.settings.gamma * 100.0).round() as i32).to_le_bytes(),
        );
        mix_bytes(
            &mut h,
            &((self.settings.master_volume * 100.0).round() as i32).to_le_bytes(),
        );
        if let Some(status) = self.shulkds.dashboard.current_status() {
            mix_bytes(&mut h, status.as_bytes());
        }
        if self.title.is_some() {
            mix_bytes(&mut h, &self.shulkds_title.hud_signature().to_le_bytes());
        }
        for slot in self.inventory.iter().take(9) {
            mix_opt_stack(&mut h, slot.as_ref());
        }
        if self.shulkds.dashboard.active_tab == DashboardTab::Inventory {
            for slot in 9..36 {
                mix_opt_stack(&mut h, self.slot_view(slot).as_ref());
            }
            for slot in 500..=504 {
                mix_opt_stack(&mut h, self.slot_view(slot).as_ref());
            }
        }
        let mut telemetry_sig: u64 = 0xcbf2_9ce4_8422_2325;
        if self.shulkds.dashboard.active_tab == DashboardTab::Minimap {
            let biome_id = self.current_biome_id_and_name().0;
            mix_bytes(&mut h, &biome_id.to_le_bytes());
            let [px, py, pz] = self.player_position();
            mix_bytes(
                &mut telemetry_sig,
                &((px * 2.0).floor() as i32).to_le_bytes(),
            );
            mix_bytes(&mut telemetry_sig, &(py.floor() as i32).to_le_bytes());
            mix_bytes(
                &mut telemetry_sig,
                &((pz * 2.0).floor() as i32).to_le_bytes(),
            );
            mix_bytes(
                &mut telemetry_sig,
                &((self.yaw.rem_euclid(360.0) * 0.5).floor() as i32).to_le_bytes(),
            );
            mix_bytes(&mut telemetry_sig, &self.frame_reading().0.to_le_bytes());
            mix_bytes(
                &mut telemetry_sig,
                &(self.world.day_time / 16).to_le_bytes(),
            );
        }
        let now = Instant::now();
        let touch_active = self.shulkds.dashboard.pointer_down
            || self.shulkds_title.pointer_down
            || self.shulkds.dashboard.last_bottom_touch.is_some_and(|t| {
                now.saturating_duration_since(t) < std::time::Duration::from_millis(250)
            });
        let elapsed = self
            .shulkds
            .dashboard
            .last_bottom_present
            .map_or(std::time::Duration::MAX, |t| {
                now.saturating_duration_since(t)
            });
        let should_present = touch_active
            || h != self.shulkds.dashboard.last_bottom_sig
            || elapsed >= std::time::Duration::from_millis(33);
        if should_present {
            self.shulkds.dashboard.last_bottom_sig = h;
            self.shulkds.dashboard.last_bottom_telemetry_sig = telemetry_sig;
            self.shulkds.dashboard.last_bottom_present = Some(now);
        }
        should_present
    }

    /// Primary input handler for normalized touch/pointer events on the bottom screen.
    pub fn handle_bottom_screen_pointer(
        &mut self,
        norm_x: f32,
        norm_y: f32,
        action: BottomPointerAction,
    ) {
        if !matches!(action, BottomPointerAction::Move)
            || self.shulkds.dashboard.pointer_down
            || self.shulkds_title.pointer_down
        {
            let now = Instant::now();
            self.last_input = now;
            self.shulkds.dashboard.last_bottom_touch = Some(now);
        }
        if self.shulkds_title_active() {
            self.handle_shulkds_title_pointer(norm_x, norm_y, action);
            return;
        }
        if self.is_shulkds_paused() {
            if matches!(action, BottomPointerAction::Down { .. }) {
                self.dismiss_soft_keyboard();
            }
            self.shulkds.dashboard.pointer_down = false;
            self.shulkds.dashboard.active_slider = None;
            self.shulkds.dashboard.drag_slot = None;
            return;
        }
        if matches!(action, BottomPointerAction::Move) && !self.shulkds.dashboard.pointer_down {
            return;
        }
        let rect = self.shulkds_bottom_gui_rect();
        let nx = norm_x.clamp(0.0, 1.0);
        let ny = norm_y.clamp(0.0, 1.0);
        let pixel_x = nx * rect.width;
        let pixel_y = ny * rect.height;

        self.shulkds.dashboard.norm_pointer_x = nx;
        self.shulkds.dashboard.norm_pointer_y = ny;
        self.shulkds.dashboard.pointer_x = rect.x + pixel_x;
        self.shulkds.dashboard.pointer_y = rect.y + pixel_y;

        match action {
            BottomPointerAction::Down { button } => {
                self.dismiss_soft_keyboard();
                self.shulkds.dashboard.bottom_focus = false;
                self.shulkds.dashboard.pointer_down = true;
                let had_carried = self.carried.is_some();
                self.shulkds.dashboard.drag_slot = if had_carried {
                    None
                } else {
                    self.find_slot_at(pixel_x, pixel_y)
                };
                self.handle_bottom_screen_tap(pixel_x, pixel_y, button == PointerButton::Secondary);
            }
            BottomPointerAction::Move => {
                if self.shulkds.dashboard.pointer_down
                    && let Some(active) = self.shulkds.dashboard.active_slider.clone()
                    && let Some(idx) = active
                        .strip_prefix("slider_")
                        .and_then(|s| s.parse::<usize>().ok())
                {
                    let lay = self.bottom_3ds_layout(0.0, 0.0, rect.width, rect.height);
                    let (ix, iy, iw, ih) = lay.inner_rect();
                    let m = Self::settings_tab_metrics(ix, iy, iw, ih);
                    let norm = (pixel_x - m.track_x) / m.track_w.max(1.0);
                    self.apply_settings_slider_norm(idx, norm);
                }
            }
            BottomPointerAction::Up { button } => {
                self.shulkds.dashboard.pointer_down = false;
                self.shulkds.dashboard.active_slider = None;
                let drag_src = self.shulkds.dashboard.drag_slot.take();
                // Finger drag-and-drop: releasing over a different slot places/swaps held stack
                if matches!(button, PointerButton::Primary | PointerButton::Secondary)
                    && self.carried.is_some()
                    && let Some(dest) = self.find_slot_at(pixel_x, pixel_y)
                    && Some(dest) != drag_src
                {
                    self.shulkds.dashboard.last_tap_slot = None;
                    self.handle_slot_click_interact(
                        dest,
                        button == PointerButton::Secondary,
                        false,
                    );
                }
            }
            BottomPointerAction::Scroll { delta_y } => {
                if delta_y.abs() < 0.01 {
                    return;
                }
                if self.shulkds.dashboard.active_tab == DashboardTab::Minimap {
                    self.shulkds.dashboard.map_zoom = if delta_y > 0.0 {
                        (self.shulkds.dashboard.map_zoom * 2.0).min(4.0)
                    } else {
                        (self.shulkds.dashboard.map_zoom * 0.5).max(0.5)
                    };
                    self.shulkds
                        .dashboard
                        .set_status(format!("Map Zoom: {}x", self.shulkds.dashboard.map_zoom));
                }
            }
        }
    }

    /// Process a tap / click at `(lx, ly)` relative to the bottom screen viewport.
    fn handle_bottom_screen_tap(&mut self, lx: f32, ly: f32, right_click: bool) {
        let rect = self.shulkds_bottom_gui_rect();
        let lay = self.bottom_3ds_layout(0.0, 0.0, rect.width, rect.height);

        // 1. Check Top Tab Bar taps
        if ly >= lay.tab_y && ly <= (lay.tab_y + lay.tab_h) {
            if let Some(idx) = (0..DashboardTab::ALL.len()).find(|&i| {
                let (tx, tw) = lay.tab_rect(i);
                lx >= tx && lx <= tx + tw
            }) {
                let next_tab = DashboardTab::ALL[idx];
                if next_tab != self.shulkds.dashboard.active_tab {
                    self.return_shulkds_carried_to_inventory();
                    self.shulkds.dashboard.active_tab = next_tab;
                }
                self.audio.play_button_click();
                return;
            }
        }

        // 2. Check Persistent Bottom Hotbar taps (when placed on bottom screen)
        if lay.show_hotbar
            && ly >= lay.hotbar_y
            && ly <= (lay.hotbar_y + lay.hotbar_h)
            && lx >= lay.hotbar_x
            && lx <= (lay.hotbar_x + lay.hotbar_w)
        {
            let col = ((lx - lay.hotbar_x) / (20.0 * lay.hotbar_scale)).floor() as isize;
            if (0..9).contains(&col) {
                let slot_idx = col as usize;
                self.shulkds.dashboard.hotbar_tap_slot = Some(slot_idx);
                if self.shulkds.dashboard.active_tab == DashboardTab::Inventory {
                    self.handle_slot_click_interact(slot_idx, right_click, false);
                } else {
                    self.return_shulkds_carried_to_inventory();
                    self.select_dashboard_slot(slot_idx);
                    self.audio.play_button_click();
                }
                return;
            }
        }

        // 3. Dispatch tap into active tab's inner area
        let (ix, iy, iw, ih) = lay.inner_rect();

        match self.shulkds.dashboard.active_tab {
            DashboardTab::Minimap => self.handle_map_tab_tap(lx, ly, ix, iy, iw, ih),
            DashboardTab::Inventory => self.handle_survival_inventory_tap(
                lx,
                ly,
                ix,
                iy,
                iw,
                ih,
                right_click,
                self.creative,
            ),
            DashboardTab::Settings => self.handle_settings_tab_tap(lx, ly, ix, iy, iw, ih),
        }
    }

    fn cycle_map_zoom_up(&mut self) {
        let cur = self.shulkds.dashboard.map_zoom;
        let next = if cur < 1.0 {
            1.0
        } else if cur < 2.0 {
            2.0
        } else if cur < 4.0 {
            4.0
        } else {
            0.5
        };
        self.shulkds.dashboard.map_zoom = next;
        self.shulkds
            .dashboard
            .set_status(format!("Map Zoom: {}x", next));
    }

    fn handle_map_tab_tap(&mut self, lx: f32, ly: f32, x: f32, y: f32, w: f32, h: f32) {
        let (map_x, map_y, map_size, info_panel_x, info_panel_w) =
            Self::map_tab_metrics(x, y, w, h);

        // Tapping directly on the parchment map cycles zoom levels
        if lx >= map_x && lx <= map_x + map_size && ly >= map_y && ly <= map_y + map_size {
            self.cycle_map_zoom_up();
            self.audio.play_button_click();
            return;
        }

        let rm = Self::map_right_layout(info_panel_x, map_y - 2.0, info_panel_w, map_size + 4.0);
        if ly >= rm.zoom_y && ly <= rm.zoom_y + rm.zoom_h {
            if lx >= rm.minus_x && lx <= rm.minus_x + rm.step_w {
                self.shulkds.dashboard.map_zoom = (self.shulkds.dashboard.map_zoom * 0.5).max(0.5);
                self.shulkds
                    .dashboard
                    .set_status(format!("Map Zoom: {}x", self.shulkds.dashboard.map_zoom));
                self.audio.play_button_click();
                return;
            }
            if lx >= rm.readout_x && lx <= rm.readout_x + rm.readout_w {
                self.cycle_map_zoom_up();
                self.audio.play_button_click();
                return;
            }
            if lx >= rm.plus_x && lx <= rm.plus_x + rm.step_w {
                self.shulkds.dashboard.map_zoom = (self.shulkds.dashboard.map_zoom * 2.0).min(4.0);
                self.shulkds
                    .dashboard
                    .set_status(format!("Map Zoom: {}x", self.shulkds.dashboard.map_zoom));
                self.audio.play_button_click();
                return;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_survival_inventory_tap(
        &mut self,
        lx: f32,
        ly: f32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        right_click: bool,
        is_creative_surv: bool,
    ) {
        let m = self.survival_inv_layout(x, y, w, h);

        // 0. Recessed Bottom Action Bar inside the Inventory panel
        if ly >= m.act_y && ly <= m.act_y + m.act_h {
            for (bx, bw, id, _, _) in self.surv_action_buttons(m.act_x, m.act_w, is_creative_surv) {
                if lx >= bx && lx <= bx + bw {
                    match id {
                        "quick" => {
                            self.shulkds.dashboard.quick_move_mode =
                                !self.shulkds.dashboard.quick_move_mode;
                            let state = if self.shulkds.dashboard.quick_move_mode {
                                "ON"
                            } else {
                                "OFF"
                            };
                            self.shulkds
                                .dashboard
                                .set_status(format!("Quick-Move Tap: {state}"));
                        }
                        "split" => self.execute_touch_split(),
                        "drop" => self.execute_touch_drop(),
                        "sort" => self.execute_quick_sort(),
                        "clear_bag" => self.execute_creative_clear_bag(),
                        _ => {}
                    }
                    self.audio.play_button_click();
                    return;
                }
            }
        }

        // 1. Top-Right Shelf: Offhand / Shield Slot (504)
        if lx >= m.offhand_x
            && lx <= m.offhand_x + m.offhand_sz
            && ly >= m.offhand_y
            && ly <= m.offhand_y + m.offhand_sz
        {
            self.handle_slot_click_interact(504, right_click, false);
            return;
        }

        // 3. 4 Vertical Armor Slots (500..504)
        for i in 0..4 {
            let sx = m.armor_x;
            let sy = m.armor_y0 + i as f32 * m.slot_size;
            if lx >= sx && lx <= sx + m.slot_size && ly >= sy && ly <= sy + m.slot_size {
                self.handle_slot_click_interact(500 + i, right_click, false);
                return;
            }
        }

        // 4. Player Preview Box (tap character to auto-equip best armor)
        if lx >= m.player_x
            && lx <= m.player_x + m.player_w
            && ly >= m.player_y
            && ly <= m.player_y + m.player_h
        {
            self.execute_auto_equip();
            self.audio.play_button_click();
            return;
        }

        // 5. Contiguous 3x9 Backpack Grid
        if self.check_inventory_grid_tap(
            lx,
            ly,
            m.bag_x0,
            m.bag_y0,
            m.slot_size,
            m.slot_size,
            m.slot_size,
            m.slot_size,
            right_click,
        ) {
            return;
        }

        // 6. Hotbar row inside Inv tab when Classic Top Hotbar is active
        if m.include_hotbar_row && ly >= m.hotbar_row_y && ly <= m.hotbar_row_y + m.slot_size {
            for col in 0..9 {
                let sx = m.bag_x0 + col as f32 * m.slot_size;
                if lx >= sx && lx <= sx + m.slot_size {
                    self.handle_slot_click_interact(col, right_click, false);
                    return;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_inventory_grid_tap(
        &mut self,
        lx: f32,
        ly: f32,
        gx: f32,
        gy: f32,
        col_step: f32,
        row_step: f32,
        slot_w: f32,
        slot_h: f32,
        right_click: bool,
    ) -> bool {
        for row in 0..3 {
            for col in 0..9 {
                let sx = gx + col as f32 * col_step;
                let sy = gy + row as f32 * row_step;
                if lx >= sx && lx <= sx + slot_w && ly >= sy && ly <= sy + slot_h {
                    self.handle_slot_click_interact(9 + row * 9 + col, right_click, false);
                    return true;
                }
            }
        }
        false
    }

    /// Slot interaction: supports touch Quick-Move mode, double-tap Quick-Move, and server sync.
    fn handle_slot_click_interact(
        &mut self,
        slot_idx: usize,
        right_click: bool,
        mut quick_move: bool,
    ) {
        self.select_dashboard_slot(slot_idx);
        let now = Instant::now();
        let is_double_tap = !right_click
            && self
                .shulkds
                .dashboard
                .last_tap_slot
                .is_some_and(|(prev_slot, at)| {
                    prev_slot == slot_idx && at.elapsed().as_millis() < 380
                });
        if self.shulkds.dashboard.quick_move_mode && self.carried.is_none() && !right_click {
            quick_move = true;
        } else if is_double_tap && self.carried.is_some() {
            // Double-tap after picking up deposits / quick-moves on supported slots
            quick_move = false;
        } else if is_double_tap {
            quick_move = true;
        }
        self.shulkds.dashboard.last_tap_slot = if is_double_tap {
            None
        } else {
            Some((slot_idx, now))
        };

        let input = if quick_move || self.has_shift_down() {
            horizon_client::menu::Input::QuickMove
        } else {
            horizon_client::menu::Input::Pickup
        };
        let btn = i32::from(right_click);
        if self.remote.is_some() {
            if self.screen == super::Screen::Creative {
                self.menu_click(Some(slot_idx), btn, input);
            } else {
                let _ = self.remote_menu_ops(&[(Some(slot_idx), btn, input)]);
            }
            self.audio.play_button_click();
            return;
        }
        if let Some(mut carried) = self.carried.take() {
            let target = self.slot_mut(slot_idx);
            if let Some(existing) = target {
                if existing.item == carried.item {
                    if right_click {
                        existing.count += 1;
                        if carried.count > 1 {
                            carried.count -= 1;
                            self.carried = Some(carried);
                        }
                    } else {
                        existing.count += carried.count;
                    }
                } else {
                    std::mem::swap(existing, &mut carried);
                    self.carried = Some(carried);
                }
            } else if right_click && carried.count > 1 {
                let mut single = carried.clone();
                single.count = 1;
                carried.count -= 1;
                *target = Some(single);
                self.carried = Some(carried);
            } else {
                *target = Some(carried);
            }
            self.audio.play_button_click();
        } else {
            let target = self.slot_mut(slot_idx);
            if let Some(mut item) = target.take() {
                if right_click && item.count > 1 {
                    let take_count = (item.count + 1) / 2;
                    let mut pick = item.clone();
                    pick.count = take_count;
                    item.count -= take_count;
                    *self.slot_mut(slot_idx) = Some(item);
                    self.carried = Some(pick);
                } else {
                    self.carried = Some(item);
                }
                self.audio.play_button_click();
            }
        }
    }

    fn handle_settings_tab_tap(&mut self, lx: f32, ly: f32, x: f32, y: f32, w: f32, h: f32) {
        let m = Self::settings_tab_metrics(x, y, w, h);

        if m.show_header {
            let [ax, ay, aw, ah] = m.all_opts_rect;
            if lx >= ax && lx <= ax + aw && ly >= ay && ly <= ay + ah {
                self.audio.play_button_click();
                self.open_options(false);
                return;
            }
        }

        for i in 0..4 {
            let ry = m.rows_y0 + i as f32 * m.row_step;
            if ly >= ry && ly <= ry + m.row_h {
                if lx >= m.minus_x && lx <= m.minus_x + m.step_btn_w {
                    match i {
                        0 => {
                            self.settings.render_distance =
                                (self.settings.render_distance - 2).max(2)
                        }
                        1 => self.settings.fov = (self.settings.fov - 5.0).clamp(30.0, 110.0),
                        2 => self.settings.gamma = (self.settings.gamma - 0.1).max(0.0),
                        _ => {
                            self.settings.master_volume =
                                (self.settings.master_volume - 0.1).max(0.0)
                        }
                    }
                    self.apply_settings_slider_norm(i, self.settings_slider_norms()[i]);
                    self.audio.play_button_click();
                    return;
                }
                if lx >= m.track_x && lx <= m.track_x + m.track_w {
                    self.shulkds.dashboard.active_slider = Some(format!("slider_{i}"));
                    let norm = (lx - m.track_x) / m.track_w.max(1.0);
                    self.apply_settings_slider_norm(i, norm);
                    self.audio.play_button_click();
                    return;
                }
                if lx >= m.plus_x && lx <= m.plus_x + m.step_btn_w {
                    match i {
                        0 => {
                            self.settings.render_distance =
                                (self.settings.render_distance + 2).min(32)
                        }
                        1 => self.settings.fov = (self.settings.fov + 5.0).clamp(30.0, 110.0),
                        2 => self.settings.gamma = (self.settings.gamma + 0.1).min(1.0),
                        _ => {
                            self.settings.master_volume =
                                (self.settings.master_volume + 0.1).min(1.0)
                        }
                    }
                    self.apply_settings_slider_norm(i, self.settings_slider_norms()[i]);
                    self.audio.play_button_click();
                    return;
                }
            }
        }

        if ly >= m.perf_y && ly <= m.perf_y + m.toggle_h {
            if lx >= m.reach_x && lx <= m.reach_x + m.reach_w {
                let all = FsrUpscaling::ALL;
                let idx = all
                    .iter()
                    .position(|&f| f == self.settings.horizon.fsr)
                    .unwrap_or(0);
                self.settings.horizon.fsr = all[(idx + 1) % all.len()];
                if self.settings.horizon.fsr != FsrUpscaling::Off {
                    self.settings.horizon.taa = false;
                    self.settings.horizon.upscaling = Upscaling::Native;
                }
                self.settings.horizon.preset = Preset::Custom;
                self.apply_ui_settings();
                self.settings.save(&Self::options_path());
                let lbl = self.settings.horizon.fsr.label().to_owned();
                self.shulkds.dashboard.set_status(format!("FSR 1.0: {lbl}"));
                self.audio.play_button_click();
                return;
            }
            if lx >= m.cam_x && lx <= m.cam_x + m.cam_w {
                self.settings.horizon.frame_generation =
                    match self.settings.horizon.frame_generation {
                        FrameGeneration::Off => FrameGeneration::Fsr31,
                        FrameGeneration::Fsr31 => FrameGeneration::Off,
                    };
                self.settings.horizon.preset = Preset::Custom;
                self.apply_ui_settings();
                self.settings.save(&Self::options_path());
                let lbl = self.settings.horizon.frame_generation.label();
                self.shulkds
                    .dashboard
                    .set_status(format!("Frame Gen: {lbl}"));
                self.audio.play_button_click();
                return;
            }
        }

        if ly >= m.toggle_y && ly <= m.toggle_y + m.toggle_h {
            if lx >= m.reach_x && lx <= m.reach_x + m.reach_w {
                self.settings.controller.reach_around = !self.settings.controller.reach_around;
                let state = if self.settings.controller.reach_around {
                    "ON"
                } else {
                    "OFF"
                };
                self.shulkds
                    .dashboard
                    .set_status(format!("Reach-Around: {state}"));
                self.audio.play_button_click();
                return;
            }
            if lx >= m.cam_x && lx <= m.cam_x + m.cam_w {
                self.camera_type = self.camera_type.cycle();
                let cam = match self.camera_type {
                    super::perspective::CameraType::FirstPerson => "1st Person",
                    super::perspective::CameraType::ThirdPersonBack => "3rd Back",
                    super::perspective::CameraType::ThirdPersonFront => "3rd Front",
                };
                self.shulkds.dashboard.set_status(format!("Camera: {cam}"));
                self.audio.play_button_click();
                return;
            }
        }

        if ly >= m.hud_y && ly <= m.hud_y + m.toggle_h {
            if lx >= m.reach_x && lx <= m.reach_x + m.reach_w {
                let next = self.settings.extras.shulkds_hotbar.cycle(false);
                self.settings.extras.shulkds_hotbar = next;
                self.settings.save(&Self::options_path());
                self.shulkds
                    .dashboard
                    .set_status(format!("Hotbar: {}", next.short_label()));
                self.audio.play_button_click();
                return;
            }
            if lx >= m.cam_x && lx <= m.cam_x + m.cam_w {
                let next = self.settings.extras.shulkds_lid_sleep.cycle(false);
                self.settings.extras.shulkds_lid_sleep = next;
                self.settings.save(&Self::options_path());
                self.sync_platform_lid_sleep_pref();
                self.shulkds
                    .dashboard
                    .set_status(format!("Lid Sleep: {}", next.short_label()));
                self.audio.play_button_click();
                return;
            }
        }
    }

    /// Keep dual-screen mouse input permanently locked on the bottom touch screen.
    pub(super) fn handle_shulkds_input_toggle(&mut self) {
        self.shulkds.mouse_focused_on_top = false;
        self.capture(false);
        self.shulkds.dashboard.set_status("Touch Mode");
        self.audio.play_button_click();
    }

    /// Touch Split Action: splits carried stack in half or picks up half of the active/last tapped slot.
    fn execute_touch_split(&mut self) {
        if let Some(carried) = self.carried.clone() {
            if let Some(dest) = (9..36).chain(0..9).find(|&i| {
                self.inventory[i].is_none()
                    || self.inventory[i]
                        .as_ref()
                        .is_some_and(|s| s.item == carried.item && s.count < 64)
            }) {
                let to_place = (carried.count / 2).max(1);
                if self.remote.is_some() {
                    let ops: Vec<_> = (0..to_place)
                        .map(|_| (Some(dest), 1, horizon_client::menu::Input::Pickup))
                        .collect();
                    let _ = self.remote_menu_ops(&ops);
                } else {
                    for _ in 0..to_place {
                        if self.carried.is_none() {
                            break;
                        }
                        self.handle_slot_click_interact(dest, true, false);
                    }
                }
                self.shulkds
                    .dashboard
                    .set_status(format!("Split {to_place} items"));
            }
        } else if let Some(slot_idx) = self
            .shulkds
            .dashboard
            .active_slot
            .filter(|&s| s < 1000 && self.slot_view(s).is_some())
            .or_else(|| {
                self.shulkds
                    .dashboard
                    .last_tap_slot
                    .map(|(s, _)| s)
                    .filter(|&s| self.slot_view(s).is_some())
            })
        {
            self.shulkds.dashboard.last_tap_slot = None;
            self.handle_slot_click_interact(slot_idx, true, false);
            self.shulkds.dashboard.set_status("Picked up half");
        } else {
            self.shulkds.dashboard.set_status("Tap a slot first");
        }
    }

    /// Touch Drop Action: drops held carried stack (or active/selected/last tapped slot) into the world.
    fn execute_touch_drop(&mut self) {
        if self.carried.is_some() {
            if self.remote.is_some() {
                let _ = self.remote_menu_ops(&[(None, 0, horizon_client::menu::Input::Pickup)]);
            } else {
                self.carried = None;
            }
            self.shulkds.dashboard.set_status("Dropped held stack");
        } else {
            let slot_idx = self
                .shulkds
                .dashboard
                .active_slot
                .filter(|&s| s < 1000 && self.slot_view(s).is_some())
                .or_else(|| {
                    self.shulkds
                        .dashboard
                        .last_tap_slot
                        .map(|(s, _)| s)
                        .filter(|&s| self.slot_view(s).is_some())
                })
                .unwrap_or(self.selected);
            if self.slot_view(slot_idx).is_some() {
                if self.remote.is_some() {
                    let _ = self.remote_menu_ops(&[(
                        Some(slot_idx),
                        1,
                        horizon_client::menu::Input::Throw,
                    )]);
                } else {
                    *self.slot_mut(slot_idx) = None;
                }
                self.shulkds.dashboard.set_status("Dropped stack");
            }
        }
    }

    /// Creative Mode: clear backpack slots (9..36) while keeping the hotbar intact.
    fn execute_creative_clear_bag(&mut self) {
        for slot in &mut self.inventory[9..36] {
            *slot = None;
        }
        self.send_creative_slots();
        self.shulkds
            .dashboard
            .set_status("Cleared backpack (hotbar kept)");
    }

    /// Quick-Sort Action: sorts items in player inventory (slots 9..36).
    pub(super) fn execute_quick_sort(&mut self) {
        if self.remote.is_some() {
            let mut desired: Vec<(usize, std::sync::Arc<str>)> = (9..36)
                .filter_map(|i| self.inventory[i].as_ref().map(|s| (i, s.item.clone())))
                .collect();
            desired.sort_by(|a, b| a.1.cmp(&b.1));
            for target_idx in 9..36 {
                let Some((_, want_item)) = desired.get(target_idx - 9) else {
                    break;
                };
                if self.inventory[target_idx]
                    .as_ref()
                    .is_some_and(|s| s.item == *want_item)
                {
                    continue;
                }
                let Some(src_idx) = ((target_idx + 1)..36).find(|&j| {
                    self.inventory[j]
                        .as_ref()
                        .is_some_and(|s| s.item == *want_item)
                }) else {
                    continue;
                };
                let mut ops = vec![
                    (Some(src_idx), 0, horizon_client::menu::Input::Pickup),
                    (Some(target_idx), 0, horizon_client::menu::Input::Pickup),
                ];
                if self.inventory[target_idx].is_some() {
                    ops.push((Some(src_idx), 0, horizon_client::menu::Input::Pickup));
                }
                let _ = self.remote_menu_ops(&ops);
            }
            self.shulkds.dashboard.set_status("Inventory Sorted");
            return;
        }
        let mut items = Vec::new();
        for i in 9..36 {
            if let Some(stack) = self.inventory[i].take() {
                items.push(stack);
            }
        }

        let mut consolidated: Vec<ItemStack> = Vec::new();
        for item in items {
            let max_stack = 64;
            if let Some(existing) = consolidated
                .iter_mut()
                .find(|s| s.item == item.item && s.count < max_stack)
            {
                let space = max_stack - existing.count;
                let add = space.min(item.count);
                existing.count += add;
                let remainder = item.count - add;
                if remainder > 0 {
                    consolidated.push(ItemStack {
                        count: remainder,
                        ..item
                    });
                }
            } else {
                consolidated.push(item);
            }
        }

        consolidated.sort_by(|a, b| a.item.cmp(&b.item));

        for (i, stack) in consolidated.into_iter().enumerate() {
            if 9 + i < 36 {
                self.inventory[9 + i] = Some(stack);
            }
        }

        self.shulkds.dashboard.set_status("Inventory Sorted");
    }

    /// Auto-Equip Action: automatically equips armor from inventory into armor slots.
    pub(super) fn execute_auto_equip(&mut self) {
        let mut equipped_count = 0;
        let armor_types = [
            ("helmet", 500),
            ("chestplate", 501),
            ("leggings", 502),
            ("boots", 503),
        ];

        for (suffix, slot) in armor_types {
            if self.armor[slot - 500].is_some() {
                continue;
            }
            let mut best_inv_idx = None;
            for i in (9..36).chain(0..9) {
                if let Some(item) = &self.inventory[i] {
                    if item.item.ends_with(suffix) {
                        best_inv_idx = Some(i);
                        break;
                    }
                }
            }

            if let Some(inv_idx) = best_inv_idx {
                if self.remote.is_some() {
                    if self.creative {
                        let current_armor = self.armor[slot - 500].take();
                        let new_armor = self.inventory[inv_idx].take();
                        self.armor[slot - 500] = new_armor;
                        self.inventory[inv_idx] = current_armor;
                        self.send_creative_slots();
                        equipped_count += 1;
                    } else {
                        let mut ops = vec![
                            (Some(inv_idx), 0, horizon_client::menu::Input::Pickup),
                            (Some(slot), 0, horizon_client::menu::Input::Pickup),
                        ];
                        if self.armor[slot - 500].is_some() {
                            ops.push((Some(inv_idx), 0, horizon_client::menu::Input::Pickup));
                        }
                        if self.remote_menu_ops(&ops) {
                            equipped_count += 1;
                        }
                    }
                } else {
                    let current_armor = self.armor[slot - 500].take();
                    let new_armor = self.inventory[inv_idx].take();
                    self.armor[slot - 500] = new_armor;
                    self.inventory[inv_idx] = current_armor;
                    equipped_count += 1;
                }
            }
        }
        // Also equip a shield into offhand (504) if offhand is currently empty
        if self.offhand.is_none()
            && let Some(sh_idx) = (9..36).chain(0..9).find(|&i| {
                self.inventory[i]
                    .as_ref()
                    .is_some_and(|s| &*s.item == "minecraft:shield")
            })
        {
            if self.remote.is_some() {
                if self.creative {
                    self.offhand = self.inventory[sh_idx].take();
                    self.send_creative_slots();
                    equipped_count += 1;
                } else if self.remote_menu_ops(&[
                    (Some(sh_idx), 0, horizon_client::menu::Input::Pickup),
                    (Some(504), 0, horizon_client::menu::Input::Pickup),
                ]) {
                    equipped_count += 1;
                }
            } else {
                self.offhand = self.inventory[sh_idx].take();
                equipped_count += 1;
            }
        }

        self.shulkds
            .dashboard
            .set_status(format!("Auto-Equipped {equipped_count} pieces"));
    }

    /// One-tap Craft 1 directly into inventory (or held cursor if full).
    pub(super) fn craft_one_from_grid(&mut self) {
        if let Some(result) = self.slot_view(200) {
            let dest = (0..36)
                .find(|&i| {
                    self.inventory[i]
                        .as_ref()
                        .is_some_and(|s| s.item == result.item && s.count + result.count <= 64)
                })
                .or_else(|| (0..36).find(|&i| self.inventory[i].is_none()));
            if self.remote.is_some() {
                if let Some(d) = dest
                    && self.carried.is_none()
                {
                    let _ = self.remote_menu_ops(&[
                        (Some(200), 0, horizon_client::menu::Input::Pickup),
                        (Some(d), 0, horizon_client::menu::Input::Pickup),
                    ]);
                } else {
                    let _ = self.remote_menu_ops(&[(
                        Some(200),
                        0,
                        horizon_client::menu::Input::Pickup,
                    )]);
                }
                self.shulkds
                    .dashboard
                    .set_status(format!("Crafted x{}", result.count));
                return;
            }
            if let Some(d) = dest {
                if let Some(existing) = &mut self.inventory[d] {
                    existing.count += result.count;
                } else {
                    self.inventory[d] = Some(result);
                }
            } else if let Some(carried) = &mut self.carried {
                if carried.item == result.item {
                    carried.count += result.count;
                }
            } else {
                self.carried = Some(result);
            }
            for i in 100..109 {
                let slot = self.slot_mut(i);
                if let Some(ing) = slot.as_mut() {
                    if ing.count > 1 {
                        ing.count -= 1;
                    } else {
                        *slot = None;
                    }
                }
            }
        }
    }

    pub(super) fn craft_all_from_grid(&mut self) {
        if self.remote.is_some() {
            let _ = self.remote_menu_ops(&[(Some(200), 0, horizon_client::menu::Input::QuickMove)]);
            self.shulkds.dashboard.set_status("Crafted Max");
            return;
        }
        for _ in 0..64 {
            if self.craft_result().is_some() {
                self.craft_one_from_grid();
            } else {
                break;
            }
        }
        self.shulkds.dashboard.set_status("Crafted Max");
    }

    fn format_biome_slug(raw: &str) -> String {
        raw.split('_')
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(first) => {
                        format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn current_biome_id_and_name(&self) -> (u16, String) {
        let [px, py, pz] = self.player_position();
        let (bx, by, bz) = (px.floor() as i32, py.floor() as i32, pz.floor() as i32);
        if let Some(chunk) = self.world.chunks.get(&(bx >> 4, bz >> 4)) {
            let biome_id = chunk.biome(bx, by, bz);
            let biomes = world_biomes!(self);
            if let Some(entry) = biomes.list.get(usize::from(biome_id)) {
                let full = entry.name.as_str();
                let (ns, slug) = full.split_once(':').unwrap_or(("minecraft", full));
                let key = format!("biome.{ns}.{slug}");
                let title = self
                    .pack
                    .names
                    .display_get(&key)
                    .filter(|s| !s.is_empty() && s != &key)
                    .unwrap_or_else(|| Self::format_biome_slug(slug));
                return (biome_id, title);
            }
        }
        let fallback = match self.dim.id() {
            "minecraft:the_nether" => "Nether Wastes",
            "minecraft:the_end" => "The End",
            _ => "Plains",
        };
        (u16::MAX, fallback.to_owned())
    }

    fn draw_dimension_thumbnail(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        size: f32,
        dim_id: &str,
        dim_col: u32,
    ) {
        let a = &self.gui.atlas;
        batch.fill(
            a,
            x - 1.0,
            y - 1.0,
            size + 2.0,
            size + 2.0,
            rgba(16, 16, 18, 255),
        );
        batch.fill(a, x, y + size, size + 1.0, 1.0, rgba(68, 70, 76, 255));
        batch.fill(a, x + size, y, 1.0, size, rgba(68, 70, 76, 255));

        let sprite = match dim_id {
            "minecraft:the_nether" => "shulkds/dim_nether",
            "minecraft:the_end" => "shulkds/dim_end",
            _ => "shulkds/dim_overworld",
        };
        if a.sprites.contains_key(sprite) {
            batch.sprite_scaled(a, sprite, x, y, size, size, u32::MAX);
        } else {
            let sky_col = match dim_id {
                "minecraft:the_nether" => rgba(56, 18, 20, 255),
                "minecraft:the_end" => rgba(22, 14, 38, 255),
                _ => rgba(0x72, 0xA4, 0xFE, 255),
            };
            let ground_h = (size * 0.28).floor().max(2.0);
            batch.fill(a, x, y, size, size - ground_h, sky_col);
            batch.fill(a, x, y + size - ground_h, size, ground_h, dim_col);
        }
    }

    fn current_dimension_info(&self) -> (&'static str, u32) {
        match self.dim.id() {
            "minecraft:the_nether" => ("The Nether", rgba(214, 96, 86, 255)),
            "minecraft:the_end" => ("The End", rgba(176, 128, 226, 255)),
            _ => ("Overworld", rgba(88, 196, 120, 255)),
        }
    }

    pub(super) fn shulkds_emulated(&self) -> bool {
        self.shulkds.config.enabled
            && self.shulkds.config.mode == horizon_client::dual_screen::DualScreenMode::Emulated
    }

    pub(super) fn shulkds_uses_top_menu_batch(&self) -> bool {
        self.shulkds_emulated() || (self.shulkds.config.enabled && self.title.is_none())
    }

    fn shulkds_auto_base_scale(size: [u32; 2]) -> u32 {
        let mut scale = 1u32;
        while size[0] / (scale + 1) >= 320 && size[1] / (scale + 1) >= 240 {
            scale += 1;
        }
        scale
    }

    pub(super) fn shulkds_ingame_hud_scale(&self, size: [u32; 2]) -> f32 {
        let base = HudBatch::new(size).scale;
        let auto_base = Self::shulkds_auto_base_scale(size);
        let auto_or_max = self.settings.gui_scale == 0 || self.settings.gui_scale >= auto_base;
        if self.shulkds.config.enabled
            && self.title.is_none()
            && !self.shulkds_emulated()
            && auto_or_max
        {
            let next = base + 2.0;
            if size[0] as f32 / next >= 300.0 && size[1] as f32 / next >= 170.0 {
                return next;
            }
            let fallback = base + 1.0;
            if size[0] as f32 / fallback >= 300.0 && size[1] as f32 / fallback >= 185.0 {
                return fallback;
            }
        }
        base
    }

    pub(super) fn shulkds_gui_scale(&self) -> f32 {
        self.renderer.as_ref().map_or(1.0, |r| {
            HudBatch::new([r.config.width, r.config.height]).scale
        })
    }

    pub(super) fn shulkds_gui_layout(&self) -> horizon_client::dual_screen::DualScreenLayout {
        let scale = self.shulkds_gui_scale();
        self.shulkds.layout.to_gui(scale)
    }

    /// Logical GUI bounds of the bottom touch dashboard: uses the secondary compositor's own
    /// swapchain size and `HudBatch` GUI scale in `DualWindow` mode (e.g. `1280x960` 4:3 on Pocket Duo),
    /// or `shulkds_gui_layout().bottom_viewport` in `Emulated` mode.
    pub(super) fn shulkds_bottom_gui_rect(&self) -> ViewportRect {
        if self.shulkds.config.mode == horizon_client::dual_screen::DualScreenMode::DualWindow
            && let Some(comp) = &self.secondary_compositor
        {
            let sec_size = comp.size();
            let sec_hud =
                HudBatch::new_shulkds_bottom(sec_size, self.shulkds.config.touch_gui_scale);
            return ViewportRect::new(
                0.0,
                0.0,
                sec_size[0] as f32 / sec_hud.scale,
                sec_size[1] as f32 / sec_hud.scale,
            );
        }
        self.shulkds_gui_layout().bottom_viewport
    }

    /// Top-screen menu metrics `(top_x, top_y, menu_w, menu_h, menu_scale)` in
    /// ShulkDS mode: fits top-screen Legacy4J/vanilla UIs comfortably across both
    /// `Emulated` viewports and dedicated `DualWindow` top displays.
    pub(super) fn shulkds_top_menu_metrics(&self) -> (f32, f32, f32, f32, f32) {
        let (top_x, top_y, tw, th) = if self.shulkds_emulated() {
            let top = &self.shulkds.layout.top_viewport;
            (top.x, top.y, top.width.max(1.0), top.height.max(1.0))
        } else {
            self.renderer
                .as_ref()
                .map_or((0.0, 0.0, 1280.0, 720.0), |r| {
                    (
                        0.0,
                        0.0,
                        r.config.width.max(1) as f32,
                        r.config.height.max(1) as f32,
                    )
                })
        };
        let (req_w, req_h) = if self.title.is_some() {
            (340.0, 224.0)
        } else if self.screen == super::Screen::Creative && self.use_legacy_creative() {
            (324.0, 218.0)
        } else if self.legacy_crafting_open() {
            (348.0, 220.0)
        } else if self.screen == super::Screen::Inventory
            && let Some((lw, lh)) = self.legacy_screen_dimensions()
        {
            (lw + 10.0, lh + 8.0)
        } else if self.screen == super::Screen::Options {
            (320.0, 240.0)
        } else if self.screen == super::Screen::Paused {
            (240.0, 196.0)
        } else if self.screen == super::Screen::Chat {
            (320.0, 180.0)
        } else {
            (280.0, 224.0)
        };
        let limit = horizon_client::hud::GUI_SCALE.load(std::sync::atomic::Ordering::Relaxed);
        let fit = (tw / req_w).min(th / req_h);
        let menu_scale = if self.shulkds_emulated() {
            let cap = if limit > 0 { limit as f32 } else { 6.0 };
            if fit >= 1.0 {
                fit.floor().clamp(1.0, cap)
            } else {
                ((fit * 4.0).floor() / 4.0).clamp(0.25, 1.0)
            }
        } else {
            let base = HudBatch::new([tw.round() as u32, th.round() as u32]).scale;
            base.min(fit.max(1.0)).max(1.0)
        };
        (
            top_x,
            top_y,
            (tw / menu_scale).max(1.0),
            (th / menu_scale).max(1.0),
            menu_scale,
        )
    }

    pub(super) fn shulkds_top_menu_batch(&self) -> HudBatch {
        let (_, _, menu_w, menu_h, menu_scale) = self.shulkds_top_menu_metrics();
        let mut batch = HudBatch::new([1, 1]);
        batch.width = menu_w;
        batch.height = menu_h;
        batch.scale = menu_scale;
        batch
    }

    pub(super) fn shulkds_top_cursor_from_physical(&self, px: f64, py: f64) -> (f32, f32) {
        if self.shulkds_uses_top_menu_batch() {
            let (top_x, top_y, _, _, menu_scale) = self.shulkds_top_menu_metrics();
            (
                (px as f32 - top_x) / menu_scale,
                (py as f32 - top_y) / menu_scale,
            )
        } else {
            let base_scale = self.shulkds_gui_scale();
            (px as f32 / base_scale, py as f32 / base_scale)
        }
    }

    pub(super) fn shulkds_top_physical_from_cursor(&self, at: (f32, f32)) -> (f64, f64) {
        if self.shulkds_uses_top_menu_batch() {
            let (top_x, top_y, _, _, menu_scale) = self.shulkds_top_menu_metrics();
            (
                f64::from(top_x + at.0 * menu_scale),
                f64::from(top_y + at.1 * menu_scale),
            )
        } else {
            let base_scale = self.shulkds_gui_scale();
            (f64::from(at.0 * base_scale), f64::from(at.1 * base_scale))
        }
    }

    pub(super) fn begin_shulkds_top_menu_batch(
        &self,
        batch: &mut HudBatch,
    ) -> Option<ShulkdsTopMenuBatchState> {
        if !self.shulkds_uses_top_menu_batch() {
            return None;
        }
        let (top_x, top_y, menu_w, menu_h, menu_scale) = self.shulkds_top_menu_metrics();
        let state = ShulkdsTopMenuBatchState {
            start_vertex: batch.vertices.len(),
            prev_width: batch.width,
            prev_height: batch.height,
            prev_scale: batch.scale,
            top_x,
            top_y,
            menu_scale,
        };
        batch.width = menu_w;
        batch.height = menu_h;
        batch.scale = menu_scale;
        Some(state)
    }

    pub(super) fn end_shulkds_top_menu_batch(
        &self,
        batch: &mut HudBatch,
        state: Option<ShulkdsTopMenuBatchState>,
    ) {
        let Some(state) = state else {
            return;
        };
        let inv_parent_scale = 1.0 / state.prev_scale.max(0.25);
        for v in &mut batch.vertices[state.start_vertex..] {
            v.pos[0] = (state.top_x + v.pos[0] * state.menu_scale) * inv_parent_scale;
            v.pos[1] = (state.top_y + v.pos[1] * state.menu_scale) * inv_parent_scale;
        }
        let off_x = state.top_x.max(0.0).round() as u32;
        let off_y = state.top_y.max(0.0).round() as u32;
        let start_u32 = state.start_vertex as u32;
        let end_u32 = batch.vertices.len() as u32;
        for (range, blend) in &mut batch.blends {
            if range.start >= start_u32
                && range.end <= end_u32
                && let horizon_client::hud::Blend::Depth(rect) = blend
            {
                rect[0] = rect[0].saturating_add(off_x);
                rect[1] = rect[1].saturating_add(off_y);
            }
        }
        batch.width = state.prev_width;
        batch.height = state.prev_height;
        batch.scale = state.prev_scale;
    }

    pub(super) fn shulkds_top_gui_rect(&self, batch_scale: f32) -> (f32, f32, f32, f32) {
        if self.shulkds.config.enabled
            && self.shulkds.config.mode == horizon_client::dual_screen::DualScreenMode::Emulated
        {
            let top = self.shulkds.layout.top_viewport.to_gui(batch_scale);
            (top.x, top.y, top.width, top.height)
        } else {
            let size = self
                .renderer
                .as_ref()
                .map_or([1280, 720], |r| [r.config.width, r.config.height]);
            let s = batch_scale.max(1.0);
            (
                0.0,
                0.0,
                (size[0] as f32 / s).ceil(),
                (size[1] as f32 / s).ceil(),
            )
        }
    }

    pub(super) fn shulkds_mouse_captured_by_shell(&self, pressed: bool) -> bool {
        if !self.shulkds_emulated() || self.pad_synthetic_input() {
            return false;
        }
        if !pressed && self.shulkds.dashboard.pointer_down {
            return true;
        }
        let Some((rx, ry)) = self.raw_cursor else {
            return false;
        };
        let (px, py) = (rx as f32, ry as f32);
        let bottom = self.shulkds.layout.bottom_viewport;
        let top = self.shulkds.layout.top_viewport;
        let in_bottom = bottom.width > 0.0 && bottom.height > 0.0 && bottom.contains(px, py);
        let outside_top = top.width > 0.0 && top.height > 0.0 && !top.contains(px, py);
        pressed && (in_bottom || outside_top)
    }

    pub(super) fn shulkds_handle_shell_mouse(&mut self, button: PointerButton, pressed: bool) {
        let bottom = self.shulkds.layout.bottom_viewport;
        let (in_bottom, norm_x, norm_y) = if let Some((rx, ry)) = self.raw_cursor
            && bottom.width > 0.0
            && bottom.height > 0.0
        {
            let (px, py) = (rx as f32, ry as f32);
            (
                bottom.contains(px, py),
                ((px - bottom.x) / bottom.width).clamp(0.0, 1.0),
                ((py - bottom.y) / bottom.height).clamp(0.0, 1.0),
            )
        } else {
            (false, 0.0, 0.0)
        };
        if in_bottom || (!pressed && self.shulkds.dashboard.pointer_down) {
            let action = if pressed {
                BottomPointerAction::Down { button }
            } else {
                BottomPointerAction::Up { button }
            };
            self.handle_bottom_screen_pointer(norm_x, norm_y, action);
        }
    }
}

pub(super) struct ShulkdsTopMenuBatchState {
    start_vertex: usize,
    prev_width: f32,
    prev_height: f32,
    prev_scale: f32,
    top_x: f32,
    top_y: f32,
    menu_scale: f32,
}

#[derive(Clone, Copy)]
struct CachedDeviceHud {
    battery: Option<(u8, bool)>,
    use_24h: bool,
    hour: u8,
    minute: u8,
}

fn local_chrono_hm() -> (u8, u8) {
    let now = chrono::Local::now();
    let secs = now.timestamp() + i64::from(now.offset().local_minus_utc());
    let s = secs.rem_euclid(86_400);
    ((s / 3600) as u8, ((s / 60) % 60) as u8)
}

fn cached_device_hud_status() -> CachedDeviceHud {
    static CACHE: std::sync::Mutex<Option<(Instant, CachedDeviceHud)>> =
        std::sync::Mutex::new(None);
    let mut guard = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, cached)) = *guard
        && at.elapsed() < std::time::Duration::from_secs(3)
    {
        return cached;
    }

    #[cfg(target_os = "android")]
    let fresh = {
        if let Some(raw) = crate::android::device_hud_status() {
            let decoded = horizon_client::dual_screen_android::decode_device_hud_status(raw);
            let (hour, minute) = decoded.local_hm.unwrap_or_else(local_chrono_hm);
            CachedDeviceHud {
                battery: decoded.battery_percent.map(|pct| (pct, decoded.charging)),
                use_24h: decoded.use_24h,
                hour,
                minute,
            }
        } else {
            let (hour, minute) = local_chrono_hm();
            CachedDeviceHud {
                battery: None,
                use_24h: false,
                hour,
                minute,
            }
        }
    };

    #[cfg(not(target_os = "android"))]
    let fresh = {
        let (hour, minute) = local_chrono_hm();
        let battery =
            horizon_client::dual_screen_android::read_linux_sysfs_battery().unwrap_or((100, false));
        CachedDeviceHud {
            battery: Some(battery),
            use_24h: false,
            hour,
            minute,
        }
    };

    *guard = Some((Instant::now(), fresh));
    fresh
}

impl Game<'_> {
    /// Draw the slightly transparent local time (bottom-left) and pixelated battery icon
    /// (bottom-right) on the bottom touch screen while in-game.
    fn draw_shulkds_bottom_device_hud(
        &self,
        batch: &mut HudBatch,
        rect: &ViewportRect,
        lay: &Bottom3dsLayout,
    ) {
        let status = cached_device_hud_status();

        const HUD_ALPHA: u8 = 172;
        const CAVITY_ALPHA: u8 = 112;
        const SHADOW_ALPHA: u8 = 108;

        // Left corner: phone local time in Minecraft font.
        let time_str = horizon_client::dual_screen_android::format_local_clock(
            status.hour,
            status.minute,
            status.use_24h,
        );
        let a = &self.gui.atlas;
        let left_limit = if lay.show_hotbar {
            lay.hotbar_x - lay.hotbar_scale - 3.0
        } else {
            rect.x + rect.width * 0.45
        };
        let tx = (rect.x + 4.0).floor();
        let avail_w = (left_limit - tx).max(18.0);
        let raw_tw = HudBatch::text_width(a, &time_str).max(1.0);
        let text_scale = (avail_w / raw_tw).clamp(0.60, 1.0);
        let top_aligned = lay.hotbar_pos == HotbarPosition::BottomScreenTop;
        let ty = if top_aligned {
            (lay.hotbar_y + (lay.hotbar_h - 8.0 * text_scale) * 0.5)
                .floor()
                .max(rect.y + 2.0)
        } else {
            (rect.y + rect.height - 4.0 - 8.0 * text_scale).floor()
        };
        batch.text_scaled(
            a,
            &time_str,
            tx,
            ty,
            text_scale,
            rgba(255, 255, 255, HUD_ALPHA),
            true,
        );

        // Right corner: crisp pixelated battery icon.
        let Some((pct, charging)) = status.battery else {
            return;
        };
        let bx = (rect.x + rect.width - 4.0 - horizon_client::dual_screen_android::BATTERY_SHELL_W)
            .floor();
        let by = if top_aligned {
            (lay.hotbar_y
                + (lay.hotbar_h - horizon_client::dual_screen_android::BATTERY_SHELL_H) * 0.5)
                .floor()
                .max(rect.y + 2.0)
        } else {
            (rect.y + rect.height - 4.0 - horizon_client::dual_screen_android::BATTERY_SHELL_H)
                .floor()
        };
        let grid = horizon_client::dual_screen_android::battery_pixel_grid(pct, charging);
        let (fill_hi, fill_body) = if charging {
            (
                rgba(136, 246, 168, HUD_ALPHA),
                rgba(74, 214, 118, HUD_ALPHA),
            )
        } else if pct > 50 {
            (rgba(144, 244, 148, HUD_ALPHA), rgba(84, 212, 92, HUD_ALPHA))
        } else if pct > 20 {
            (
                rgba(255, 228, 124, HUD_ALPHA),
                rgba(242, 192, 64, HUD_ALPHA),
            )
        } else {
            (rgba(255, 128, 128, HUD_ALPHA), rgba(232, 72, 72, HUD_ALPHA))
        };
        let color_for = |px| match px {
            horizon_client::dual_screen_android::BatteryPixel::Empty => None,
            horizon_client::dual_screen_android::BatteryPixel::Shadow => {
                Some(rgba(14, 16, 20, SHADOW_ALPHA))
            }
            horizon_client::dual_screen_android::BatteryPixel::Shell => {
                Some(rgba(240, 244, 248, HUD_ALPHA))
            }
            horizon_client::dual_screen_android::BatteryPixel::Cavity => {
                Some(rgba(16, 18, 22, CAVITY_ALPHA))
            }
            horizon_client::dual_screen_android::BatteryPixel::FillHi => Some(fill_hi),
            horizon_client::dual_screen_android::BatteryPixel::FillBody => Some(fill_body),
            horizon_client::dual_screen_android::BatteryPixel::Bolt => {
                Some(rgba(255, 244, 118, 200))
            }
        };
        for (ry, row) in grid.iter().enumerate() {
            let mut cx = 0usize;
            while cx < row.len() {
                let Some(col) = color_for(row[cx]) else {
                    cx += 1;
                    continue;
                };
                let mut end = cx + 1;
                while end < row.len() && row[end] == row[cx] {
                    end += 1;
                }
                batch.fill(
                    a,
                    bx + cx as f32,
                    by + ry as f32,
                    (end - cx) as f32,
                    1.0,
                    col,
                );
                cx = end;
            }
        }
    }

    fn draw_shulkds_bottom_backdrop(&self, batch: &mut HudBatch, rect: &ViewportRect) {
        let (bx, by, bw, bh) = (rect.x, rect.y, rect.width, rect.height);
        let screen_w = batch.width;
        if bx > 0.0 || (bx + bw) < screen_w {
            let bezel_color = rgba(10, 10, 14, 255);
            let border_color = rgba(28, 28, 34, 255);
            if bx > 0.0 {
                batch.fill(&self.gui.atlas, 0.0, by, bx, bh, bezel_color);
                batch.fill(&self.gui.atlas, bx - 1.0, by, 1.0, bh, border_color);
            }
            let right_x = bx + bw;
            if right_x < screen_w {
                batch.fill(
                    &self.gui.atlas,
                    right_x,
                    by,
                    screen_w - right_x,
                    bh,
                    bezel_color,
                );
                batch.fill(&self.gui.atlas, right_x, by, 1.0, bh, border_color);
            }
        }
        if self.gui.atlas.sprites.contains_key("block/dirt") {
            ui::tiled(
                batch,
                &self.gui.atlas,
                "block/dirt",
                bx,
                by,
                bw,
                bh,
                32.0,
                rgba(72, 72, 72, 255),
            );
        } else {
            ui::menu_background(batch, &self.gui.atlas, bx, by, bw, bh, false);
        }
    }

    /// Draw the bottom screen during loading/saving screens: plain darkened dirt backdrop only,
    /// keeping all loading/saving text and progress bars exclusively on the top screen.
    pub(super) fn draw_shulkds_loading_bottom(&self, batch: &mut HudBatch, rect: &ViewportRect) {
        self.draw_shulkds_bottom_backdrop(batch, rect);
    }

    /// Present the plain darkened dirt bottom screen onto the secondary display in `DualWindow`
    /// mode, redrawing immediately when entering loading and idling at 2 Hz afterward.
    pub(super) fn present_shulkds_loading_bottom(&mut self) {
        if !self.shulkds.config.enabled {
            return;
        }
        self.sync_secondary_display();
        if self.shulkds.config.mode != horizon_client::dual_screen::DualScreenMode::DualWindow {
            return;
        }
        if let Some(mut comp) = self.secondary_compositor.take() {
            let sec_size = comp.size();
            let loading_sig =
                0x5348_554c_4b5f_4c44_u64 ^ (u64::from(sec_size[0]) << 32) ^ u64::from(sec_size[1]);
            let now = Instant::now();
            let elapsed = self
                .shulkds
                .dashboard
                .last_bottom_present
                .map_or(std::time::Duration::MAX, |t| {
                    now.saturating_duration_since(t)
                });
            if self.shulkds.dashboard.last_bottom_sig != loading_sig
                || elapsed >= std::time::Duration::from_millis(500)
            {
                let mut sec_hud =
                    HudBatch::new_shulkds_bottom(sec_size, self.shulkds.config.touch_gui_scale);
                let bottom_rect = ViewportRect::new(
                    0.0,
                    0.0,
                    sec_size[0] as f32 / sec_hud.scale,
                    sec_size[1] as f32 / sec_hud.scale,
                );
                self.draw_shulkds_loading_bottom(&mut sec_hud, &bottom_rect);
                if let Some(r) = self.renderer.as_mut() {
                    comp.present_hud_surface(|target, depth, size| {
                        r.render_hud_to_view_with_depth(target, depth, &sec_hud, size);
                    });
                }
                self.shulkds.dashboard.last_bottom_sig = loading_sig;
                self.shulkds.dashboard.last_bottom_present = Some(now);
            }
            self.secondary_compositor = Some(comp);
        }
    }
}

#[cfg(test)]
mod classic_carried_tests {
    use super::*;

    #[test]
    fn paused_top_screen_keeps_carried_item_regardless_of_bottom_tab() {
        for tab in [
            DashboardTab::Minimap,
            DashboardTab::Inventory,
            DashboardTab::Settings,
        ] {
            assert!(!Game::should_auto_return_bottom_carried(true, true, tab));
            assert!(!Game::should_auto_return_bottom_carried(true, false, tab));
        }
    }

    #[test]
    fn playing_top_screen_only_returns_carried_item_off_inventory_tab() {
        assert!(Game::should_auto_return_bottom_carried(
            false,
            true,
            DashboardTab::Minimap
        ));
        assert!(Game::should_auto_return_bottom_carried(
            false,
            true,
            DashboardTab::Settings
        ));
        assert!(!Game::should_auto_return_bottom_carried(
            false,
            true,
            DashboardTab::Inventory
        ));
        assert!(!Game::should_auto_return_bottom_carried(
            false,
            false,
            DashboardTab::Minimap
        ));
    }

    #[test]
    fn map_right_layout_fits_non_overlapping_across_presets() {
        let cases = [
            (320.0_f32, 240.0_f32, HotbarPosition::BottomScreenBottom),
            (310.0_f32, 270.0_f32, HotbarPosition::BottomScreenBottom),
            (320.0_f32, 240.0_f32, HotbarPosition::TopScreenClassic),
            (240.0_f32, 160.0_f32, HotbarPosition::BottomScreenBottom),
        ];
        for (bw, bh, hotbar) in cases {
            let lay = Bottom3dsLayout::compute(0.0, 0.0, bw, bh, hotbar);
            let (ix, iy, iw, ih) = lay.inner_rect();
            let (_, map_y, map_size, info_x, info_w) = Game::map_tab_metrics(ix, iy, iw, ih);
            let panel_y = map_y - 2.0;
            let panel_h = map_size + 4.0;
            let rm = Game::map_right_layout(info_x, panel_y, info_w, panel_h);

            assert!(rm.rx >= info_x);
            assert!(rm.rx + rm.rw <= info_x + info_w);
            assert!(rm.clock_y >= panel_y);
            assert!(rm.clock_y + rm.clock_h <= rm.heading_y);
            assert!(rm.heading_y + rm.heading_h <= rm.coord_ys[0]);
            assert!(rm.coord_ys[0] + rm.coord_h <= rm.coord_ys[1]);
            assert!(rm.coord_ys[1] + rm.coord_h <= rm.coord_ys[2]);
            assert!(rm.coord_ys[2] + rm.coord_h <= rm.biome_y);
            assert!(rm.biome_y + rm.biome_h <= rm.fps_box_y);
            assert!(rm.fps_box_y + rm.fps_box_h <= rm.zoom_y);
            assert!(rm.zoom_y + rm.zoom_h <= panel_y + panel_h);
            assert!(rm.biome_h >= 20.0);
        }
    }

    #[test]
    fn clock_frame_index_matches_vanilla_solar_cycle() {
        assert_eq!(Game::clock_frame_index(6_000), 0);
        assert_eq!(Game::clock_frame_index(12_000), 16);
        assert_eq!(Game::clock_frame_index(18_000), 32);
        assert_eq!(Game::clock_frame_index(0), 48);
        assert_eq!(Game::clock_frame_index(30_000), 0);
    }

    #[test]
    fn compass_heading_info_matches_cardinals_and_intercardinals() {
        assert_eq!(Game::compass_heading_info(0.0), (0, "South"));
        assert_eq!(Game::compass_heading_info(45.0), (4, "Southwest"));
        assert_eq!(Game::compass_heading_info(90.0), (8, "West"));
        assert_eq!(Game::compass_heading_info(180.0), (16, "North"));
        assert_eq!(Game::compass_heading_info(225.0), (20, "Northeast"));
        assert_eq!(Game::compass_heading_info(270.0), (24, "East"));
        assert_eq!(Game::compass_heading_info(-90.0), (24, "East"));
    }

    #[test]
    fn biome_card_layout_never_overlaps_across_all_biomes_and_presets() {
        fn mc_width(s: &str) -> f32 {
            s.chars()
                .map(|c| match c {
                    'i' | 'l' | '!' | '.' | ',' | ':' | ';' | '\'' | '|' => 2.0,
                    'I' | 't' | ' ' => 4.0,
                    'f' | 'k' | '(' | ')' | '{' | '}' | '[' | ']' => 5.0,
                    _ => 6.0,
                })
                .sum()
        }
        fn fixed6_width(s: &str) -> f32 {
            (s.chars().count() as f32) * 6.0
        }

        let biomes = [
            "Plains",
            "Sunflower Plains",
            "Snowy Plains",
            "Ice Spikes",
            "Desert",
            "Swamp",
            "Mangrove Swamp",
            "Forest",
            "Flower Forest",
            "Birch Forest",
            "Dark Forest",
            "Pale Garden",
            "Old Growth Birch Forest",
            "Old Growth Pine Taiga",
            "Old Growth Spruce Taiga",
            "Taiga",
            "Snowy Taiga",
            "Savanna",
            "Savanna Plateau",
            "Windswept Hills",
            "Windswept Gravelly Hills",
            "Windswept Forest",
            "Windswept Savanna",
            "Jungle",
            "Sparse Jungle",
            "Bamboo Jungle",
            "Badlands",
            "Eroded Badlands",
            "Wooded Badlands",
            "Meadow",
            "Cherry Grove",
            "Grove",
            "Snowy Slopes",
            "Frozen Peaks",
            "Jagged Peaks",
            "Stony Peaks",
            "River",
            "Frozen River",
            "Beach",
            "Snowy Beach",
            "Stony Shore",
            "Warm Ocean",
            "Lukewarm Ocean",
            "Deep Lukewarm Ocean",
            "Ocean",
            "Deep Ocean",
            "Cold Ocean",
            "Deep Cold Ocean",
            "Frozen Ocean",
            "Deep Frozen Ocean",
            "Mushroom Fields",
            "Dripstone Caves",
            "Lush Caves",
            "Deep Dark",
            "Nether Wastes",
            "Warped Forest",
            "Crimson Forest",
            "Soul Sand Valley",
            "Basalt Deltas",
            "The End",
            "End Highlands",
            "End Midlands",
            "Small End Islands",
            "End Barrens",
            "The Void",
            "Dappled Forest",
            "Sparse Cherry Grove",
            // Extreme modded single-word and multi-word stress cases
            "Supercalifragilisticexpialidocious",
            "Ancient Enchanted Crystal Caverns Valley",
        ];
        let dims = ["Overworld", "The Nether", "The End"];
        let presets = [
            (320.0_f32, 240.0_f32, HotbarPosition::BottomScreenBottom),
            (310.0_f32, 270.0_f32, HotbarPosition::BottomScreenBottom),
            (320.0_f32, 240.0_f32, HotbarPosition::TopScreenClassic),
            (240.0_f32, 160.0_f32, HotbarPosition::BottomScreenBottom),
        ];
        for (bw, bh, hotbar) in presets {
            let lay = Bottom3dsLayout::compute(0.0, 0.0, bw, bh, hotbar);
            let (ix, iy, iw, ih) = lay.inner_rect();
            let (_, map_y, map_size, info_x, info_w) = Game::map_tab_metrics(ix, iy, iw, ih);
            let rm = Game::map_right_layout(info_x, map_y - 2.0, info_w, map_size + 4.0);
            for biome in biomes {
                for dim in dims {
                    for measure in [mc_width as fn(&str) -> f32, fixed6_width as fn(&str) -> f32] {
                        let card = BiomeCardLayout::compute(
                            rm.rx, rm.biome_y, rm.rw, rm.biome_h, biome, dim, measure,
                        );
                        if let Some([tx, ty, tw, th]) = card.icon_rect {
                            assert!(tx - 1.0 >= rm.rx + 2.0);
                            assert!(ty - 1.0 >= rm.biome_y + 2.0);
                            assert!(ty + th + 1.0 <= rm.biome_y + rm.biome_h - 2.0);
                            assert!(tx + tw + 1.0 <= card.text_x - 3.0);
                        }
                        let mut lines: Vec<(String, f32, f32)> = card.biome_lines.clone();
                        if let Some(d) = card.dim_line.clone() {
                            lines.push(d);
                        }
                        assert!(!lines.is_empty());
                        for (txt, ly, ls) in &lines {
                            let rw = measure(txt) * ls;
                            assert!(rw <= card.avail_w + 0.05, "{biome}/{dim} {txt} w={rw}");
                            assert!(
                                card.text_x + rw <= rm.rx + rm.rw - 4.5,
                                "{biome}/{dim} right clip"
                            );
                            assert!(*ly >= rm.biome_y + 2.0, "{biome}/{dim} top clip ly={ly}");
                            assert!(
                                ly + 8.0 * ls <= rm.biome_y + rm.biome_h - 2.0,
                                "{biome}/{dim} bottom clip"
                            );
                        }
                        for pair in lines.windows(2) {
                            let (_, ya, sa) = &pair[0];
                            let (_, yb, _) = &pair[1];
                            assert!(
                                *yb >= *ya + 8.5 * *sa + 1.2,
                                "{biome}/{dim} descender overlap ya={ya} sa={sa} yb={yb}"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(
            Game::format_biome_slug("windswept_gravelly_hills"),
            "Windswept Gravelly Hills"
        );
    }
}
