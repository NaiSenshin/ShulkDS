//! 1:1 Legacy4J (`wily.legacy`) Console Edition GUIs, primitives, and slot geometry.

use super::{
    Game, Screen,
    mount_menu::{ARMOR as MOUNT_ARMOR, CHEST as MOUNT_CHEST, SADDLE as MOUNT_SADDLE},
    stations::{INPUT as STATION_INPUT, Kind as StationKind, RESULT as STATION_RESULT},
};
use horizon_client::hud::{HudBatch, rgba};
use horizon_client::items::ItemStack;
use horizon_client::ui::{blit_sprite, nine_slice_scaled};

impl Game<'_> {
    /// Whether top-screen container, station, creative, pause, and death GUIs use Legacy4J 1:1 layouts.
    #[inline]
    pub(super) fn use_legacy_gui(&self) -> bool {
        self.shulkds.config.enabled
    }

    #[inline]
    pub(super) fn use_classic_crafting(&self) -> bool {
        self.settings.extras.shulkds_classic_crafting
    }

    #[inline]
    pub(super) fn use_classic_inventory(&self) -> bool {
        self.settings.extras.shulkds_classic_inventory
    }

    #[inline]
    pub(super) fn use_legacy_creative(&self) -> bool {
        self.use_legacy_gui() && !self.use_classic_inventory()
    }

    #[inline]
    pub(super) fn use_legacy_inventory_layout(&self) -> bool {
        if !self.use_legacy_gui() {
            return false;
        }
        if self.legacy_crafting_open() {
            return true;
        }
        if self.screen == Screen::Creative {
            return !self.use_classic_inventory();
        }
        if self.screen == Screen::Inventory
            && self.container.is_none()
            && self.station.is_none()
            && !self.mount_active()
        {
            if self.craft_size == 3 {
                return !self.use_classic_crafting();
            }
            return !self.use_classic_inventory() && !self.use_classic_crafting();
        }
        true
    }

    #[inline]
    pub(super) fn is_virtual_mouse_gui_open(
        shulkds_enabled: bool,
        has_overlay_or_title: bool,
        screen: Screen,
        legacy_strip_active: bool,
    ) -> bool {
        shulkds_enabled
            && !has_overlay_or_title
            && matches!(screen, Screen::Inventory | Screen::Creative)
            && !legacy_strip_active
    }

    /// True when a slot-based inventory, container, workstation, classic crafting, or creative GUI is open in ShulkDS mode.
    #[inline]
    pub(super) fn shulkds_virtual_mouse_gui_open(&self) -> bool {
        Self::is_virtual_mouse_gui_open(
            self.shulkds.config.enabled,
            self.title.is_some() || self.sign_edit.is_some() || self.book.is_some(),
            self.screen,
            self.pad_legacy_crafting_active(),
        )
    }

    /// True when the open screen should render `LegacyCraftingScreen` (2x2 pocket or 3x3 Crafting Table).
    #[inline]
    pub(super) fn legacy_crafting_open(&self) -> bool {
        self.use_legacy_gui()
            && !self.use_classic_crafting()
            && self.screen == Screen::Inventory
            && self.container.is_none()
            && self.station.is_none()
            && !self.mount_active()
            && (self.craft_size == 3 || self.legacy_crafting.pocket_crafting)
    }

    /// True when the open screen should render `LegacyStonecutterScreen`.
    #[inline]
    pub(super) fn legacy_stonecutter_open(&self) -> bool {
        self.use_legacy_gui()
            && self.screen == Screen::Inventory
            && self
                .station
                .as_ref()
                .is_some_and(|s| s.kind == StationKind::Stonecutter)
    }

    /// True when the open screen should render `LegacyMerchantScreen`.
    #[inline]
    pub(super) fn legacy_merchant_open(&self) -> bool {
        self.use_legacy_gui()
            && self.screen == Screen::Inventory
            && self
                .station
                .as_ref()
                .is_some_and(|s| s.kind == StationKind::Merchant)
    }

    pub(super) fn legacy_panel(&self, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
        nine_slice_scaled(
            batch,
            &self.gui.atlas,
            "legacy/tiles/small_panel",
            48,
            48,
            [16; 4],
            x,
            y,
            w,
            h,
            u32::MAX,
        );
    }

    pub(super) fn legacy_creative_panel(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        nine_slice_scaled(
            batch,
            &self.gui.atlas,
            "legacy/tiles/creative_panel",
            48,
            48,
            [16; 4],
            x,
            y,
            w,
            h,
            u32::MAX,
        );
    }

    pub(super) fn legacy_thumb_panel(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
    ) {
        nine_slice_scaled(
            batch,
            &self.gui.atlas,
            "legacy/tiles/panel",
            24,
            24,
            [8; 4],
            x,
            y,
            w,
            h,
            color,
        );
    }

    pub(super) fn legacy_recess_color(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
    ) {
        nine_slice_scaled(
            batch,
            &self.gui.atlas,
            "legacy/tiles/square_recessed_panel",
            24,
            24,
            [8; 4],
            x,
            y,
            w,
            h,
            color,
        );
    }

    pub(super) fn legacy_recess(&self, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
        self.legacy_recess_color(batch, x, y, w, h, u32::MAX);
    }

    pub(super) fn legacy_entity_panel(&self, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
        nine_slice_scaled(
            batch,
            &self.gui.atlas,
            "legacy/tiles/entity_panel",
            24,
            24,
            [8; 4],
            x,
            y,
            w,
            h,
            u32::MAX,
        );
        batch.fill(
            &self.gui.atlas,
            x + 2.0,
            y + 2.0,
            (w - 4.0).max(1.0),
            (h - 4.0).max(1.0),
            rgba(0, 0, 0, 255),
        );
    }

    pub(super) fn legacy_square_entity_panel(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        nine_slice_scaled(
            batch,
            &self.gui.atlas,
            "legacy/tiles/square_entity_panel",
            24,
            24,
            [8; 4],
            x,
            y,
            w,
            h,
            u32::MAX,
        );
        batch.fill(
            &self.gui.atlas,
            x + 2.0,
            y + 2.0,
            (w - 4.0).max(1.0),
            (h - 4.0).max(1.0),
            rgba(0, 0, 0, 255),
        );
    }

    pub(super) fn legacy_tab_bg(
        &self,
        batch: &mut HudBatch,
        sprite: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        nine_slice_scaled(
            batch,
            &self.gui.atlas,
            sprite,
            49,
            49,
            [16; 4],
            x,
            y,
            w,
            h,
            u32::MAX,
        );
    }

    /// Draw a Legacy4J `icon_holder` (or `sizeable_icon_holder` when `< 18px`) at top-left `(x, y)` with size `sz x sz`.
    pub(super) fn legacy_slot_box(&self, batch: &mut HudBatch, x: f32, y: f32, sz: f32) {
        self.legacy_icon_holder(batch, x, y, sz, false, false);
    }

    /// Draw a Legacy4J `LegacyIconHolder` background (`icon_holder`, `red_icon_holder`, `gray_icon_holder`, or `sizeable_icon_holder`).
    pub(super) fn legacy_icon_holder(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        sz: f32,
        warning: bool,
        gray: bool,
    ) {
        if warning {
            blit_sprite(
                batch,
                &self.gui.atlas,
                "legacy/container/red_icon_holder",
                x,
                y,
                sz,
                sz,
                u32::MAX,
            );
        } else if gray {
            blit_sprite(
                batch,
                &self.gui.atlas,
                "legacy/container/gray_icon_holder",
                x,
                y,
                sz,
                sz,
                u32::MAX,
            );
        } else if sz < 18.0 {
            nine_slice_scaled(
                batch,
                &self.gui.atlas,
                "legacy/container/sizeable_icon_holder",
                16,
                16,
                [1; 4],
                x,
                y,
                sz,
                sz,
                u32::MAX,
            );
        } else {
            blit_sprite(
                batch,
                &self.gui.atlas,
                "legacy/container/icon_holder",
                x,
                y,
                sz,
                sz,
                u32::MAX,
            );
        }
    }

    /// Draw Legacy4J warning badge (`icon_warning`) at the top-left of an icon holder.
    pub(super) fn legacy_warning_icon(&self, batch: &mut HudBatch, x: f32, y: f32) {
        blit_sprite(
            batch,
            &self.gui.atlas,
            "legacy/container/icon_warning",
            x,
            y,
            8.0,
            8.0,
            u32::MAX,
        );
    }

    /// Draw a sprite directly inside `(x, y, w, h)` with white tint.
    pub(super) fn legacy_slot_with(
        &self,
        batch: &mut HudBatch,
        sprite: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        blit_sprite(batch, &self.gui.atlas, sprite, x, y, w, h, u32::MAX);
    }

    /// Draw a `cols x rows` grid of `sz x sz` Legacy4J `icon_holder` boxes whose top-left box corner is `(ox + gx, oy + gy)`.
    pub(super) fn legacy_slot_grid_sz(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
        gx: f32,
        gy: f32,
        cols: usize,
        rows: usize,
        sz: f32,
    ) {
        for r in 0..rows {
            for c in 0..cols {
                self.legacy_slot_box(batch, ox + gx + c as f32 * sz, oy + gy + r as f32 * sz, sz);
            }
        }
    }

    /// Draw the 3x9 main player inventory + 1x9 hotbar using exact `21x21` Legacy4J HD coordinates.
    pub(super) fn legacy_player_inv_21(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
        inv_x: f32,
        main_y: f32,
        hotbar_y: f32,
    ) {
        self.legacy_slot_grid_sz(batch, ox, oy, inv_x, main_y, 9, 3, 21.0);
        self.legacy_slot_grid_sz(batch, ox, oy, inv_x, hotbar_y, 9, 1, 21.0);
    }

    /// Draw the 3x9 main player inventory + 1x9 hotbar using `16x16` `sizeable_icon_holder` boxes (`RecipesScreen`).
    pub(super) fn legacy_player_inv_16(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
        inv_x: f32,
        main_y: f32,
        hotbar_y: f32,
    ) {
        self.legacy_slot_grid_sz(batch, ox, oy, inv_x, main_y, 9, 3, 16.0);
        self.legacy_slot_grid_sz(batch, ox, oy, inv_x, hotbar_y, 9, 1, 16.0);
    }

    /// Draw `LegacyScroller` (`w x h` recessed track, `16x16` thumb, `13x7` up/down arrows).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn legacy_scroller(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        scroll_idx: usize,
        scroll_max: usize,
    ) {
        let color = if scroll_max > 0 {
            u32::MAX
        } else {
            rgba(255, 255, 255, 128)
        };
        if scroll_max > 0 {
            let ax = x + (w - 13.0) * 0.5;
            if scroll_idx > 0 {
                blit_sprite(
                    batch,
                    &self.gui.atlas,
                    "legacy/widget/scroll_up",
                    ax,
                    y - 11.0,
                    13.0,
                    7.0,
                    u32::MAX,
                );
            }
            if scroll_idx < scroll_max {
                blit_sprite(
                    batch,
                    &self.gui.atlas,
                    "legacy/widget/scroll_down",
                    ax,
                    y + h + 4.0,
                    13.0,
                    7.0,
                    u32::MAX,
                );
            }
        }
        self.legacy_recess_color(batch, x, y, w, h, color);
        let frac = if scroll_max > 0 {
            (scroll_idx.min(scroll_max) as f32) / (scroll_max as f32)
        } else {
            0.0
        };
        let thumb_y = y - 1.0 + frac * (h - 13.5).max(0.0);
        self.legacy_thumb_panel(batch, x - 2.0, thumb_y, 16.0, 16.0, color);
    }

    /// Draw Legacy4J large crafting/smelting arrow (`arrow` + optional `full_arrow` fill `0.0..=1.0`).
    pub(super) fn legacy_arrow(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        progress: Option<f32>,
    ) {
        blit_sprite(
            batch,
            &self.gui.atlas,
            "legacy/container/arrow",
            x,
            y,
            w,
            h,
            u32::MAX,
        );
        if let Some(p) = progress
            && p > 0.0
            && let Some(mut r) = self
                .gui
                .atlas
                .sprites
                .get("legacy/container/full_arrow")
                .copied()
        {
            let frac = p.clamp(0.0, 1.0);
            r.u1 = r.u0 + (r.u1 - r.u0) * frac;
            batch.rect_uv(x, y, w * frac, h, r, u32::MAX);
        }
    }

    /// Draw Legacy4J furnace flame (`lit` + optional bottom-up `lit_progress` fill `0.0..=1.0`).
    pub(super) fn legacy_flame(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        size: f32,
        progress: Option<f32>,
    ) {
        blit_sprite(
            batch,
            &self.gui.atlas,
            "legacy/container/lit",
            x,
            y,
            size,
            size,
            u32::MAX,
        );
        if let Some(p) = progress
            && p > 0.0
            && let Some(mut r) = self
                .gui
                .atlas
                .sprites
                .get("legacy/container/lit_progress")
                .copied()
        {
            let frac = p.clamp(0.0, 1.0);
            let h = (size * frac).ceil().min(size);
            let top_skip = (size - h) / size;
            r.v0 += (r.v1 - r.v0) * top_skip;
            batch.rect_uv(x, y + size - h, size, h, r, u32::MAX);
        }
    }

    /// Draw a Legacy4J-scaled `ItemStack` centered inside a `sz x sz` box at top-left `(x, y)` (`scale = sz / 18.0`),
    /// optionally dimming alpha (`alpha < 1.0`).
    pub(super) fn draw_legacy_item_in_box(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
        sz: f32,
        alpha: f32,
    ) {
        if stack.count == 0 {
            return;
        }
        let scale = sz / 18.0;
        let draw_sz = 16.0 * scale;
        let ix = x + (sz - draw_sz) * 0.5;
        let iy = y + (sz - draw_sz) * 0.5;
        let start = batch.vertices.len();
        self.draw_item_with_count_offset(batch, stack, ix, iy, (-1, -2));
        if (scale - 1.0).abs() > 0.001 {
            batch.scale_since(start, ix, iy, scale, scale);
        }
        if alpha < 0.99 {
            let a_mul = alpha.clamp(0.0, 1.0);
            for v in &mut batch.vertices[start..] {
                let a = ((v.color >> 24) & 0xFF) as f32;
                let na = (a * a_mul).round().clamp(0.0, 255.0) as u32;
                v.color = (v.color & 0x00FF_FFFF) | (na << 24);
            }
        }
    }

    /// Draw a menu slot's `ItemStack` scaled by `legacy_slot_size_for(slot) / 18.0` around its 16x16 logical origin `(ix, iy)`.
    pub(super) fn draw_legacy_slot_item(
        &self,
        batch: &mut HudBatch,
        drag: Option<&super::DragView>,
        slot: usize,
        stack: &Option<ItemStack>,
        ix: f32,
        iy: f32,
    ) {
        if !self.use_legacy_inventory_layout() {
            self.draw_slot_item(batch, drag, slot, stack, ix, iy);
            return;
        }
        let sz = self.legacy_slot_size_for(slot);
        let scale = sz / 18.0;
        if (scale - 1.0).abs() <= 0.001 {
            self.draw_slot_item(batch, drag, slot, stack, ix, iy);
            return;
        }
        // Shift so the scaled 16x16 item ends up centered around the exact same (ix + 8, iy + 8) slot center.
        let adj_x = ix + 8.0 - 8.0 * scale;
        let adj_y = iy + 8.0 - 8.0 * scale;
        let start = batch.vertices.len();
        self.draw_slot_item_at(batch, drag, slot, stack, adj_x, adj_y, (-1, -2));
        batch.scale_since(start, adj_x, adj_y, scale, scale);
    }

    /// Legacy4J HD slot box dimension (`16.0`, `21.0`, `23.0`, `27.0`, `30.0`, `32.0`) for slot `i`.
    pub(super) fn legacy_slot_size_for(&self, i: usize) -> f32 {
        if !self.use_legacy_inventory_layout() {
            return 18.0;
        }
        if self.screen == Screen::Creative {
            return 27.0;
        }
        if self.legacy_crafting_open()
            || self.legacy_stonecutter_open()
            || self.legacy_merchant_open()
        {
            return 16.0;
        }
        if let Some(s) = self.station.as_ref() {
            return match s.kind {
                StationKind::Anvil | StationKind::Grindstone if i >= STATION_INPUT => 30.0,
                StationKind::Smithing if i == STATION_RESULT => 30.0,
                StationKind::Cartography if i == STATION_RESULT => 27.0,
                StationKind::Cartography if i >= STATION_INPUT => 23.0,
                StationKind::Loom if i == STATION_RESULT => 32.0,
                StationKind::Loom if i >= STATION_INPUT => 23.0,
                _ => 21.0,
            };
        }
        if self.brewing_open() && (300..305).contains(&i) {
            return 27.0;
        }
        match self.container_kind() {
            Some(horizon_client::containers::BlockEntity::Furnace(_)) if i == 302 => 32.0,
            Some(horizon_client::containers::BlockEntity::Crafter(_)) if i == 309 => 32.0,
            _ => 21.0,
        }
    }

    /// Exact Legacy4J HD panel dimensions `(width, height)` for centering & bounds.
    pub(super) fn legacy_screen_dimensions(&self) -> Option<(f32, f32)> {
        if !self.use_legacy_inventory_layout() {
            return None;
        }
        if self.screen == Screen::Creative {
            return Some((321.0, 212.0));
        }
        if let Some(s) = self.station.as_ref() {
            return Some(match s.kind {
                StationKind::Anvil | StationKind::Grindstone | StationKind::Smithing => {
                    (207.0, 215.0)
                }
                StationKind::Cartography => (207.0, 254.0),
                StationKind::Beacon => (260.0, 255.0),
                StationKind::Stonecutter => (348.0, 215.0),
                StationKind::Merchant => (294.0, 181.0),
                StationKind::Loom | StationKind::Enchanting => (215.0, 217.0),
            });
        }
        if self.brewing_open() {
            return Some((213.0, 225.0));
        }
        if self.mount_active() {
            return Some((215.0, 203.0));
        }
        if let Some(rows) = self.chest_rows() {
            if self.machine_screen() == Some(super::machines::Screen::Hopper) {
                return Some((215.0, 165.0));
            }
            if self.machine_screen().is_some() {
                return Some((215.0, 207.0));
            }
            return Some((215.0, 207.0 + (rows as f32 - 3.0) * 21.0));
        }
        match self.container_kind() {
            Some(horizon_client::containers::BlockEntity::Furnace(_)) => Some((214.0, 215.0)),
            Some(horizon_client::containers::BlockEntity::Crafter(_)) => Some((215.0, 202.0)),
            Some(_) => Some((215.0, 217.0)),
            None if self.legacy_crafting_open() => Some((348.0, 215.0)),
            None => Some((215.0, 217.0)),
        }
    }

    /// Exact Legacy4J HD screen top-left origin `(ox, oy)` within `batch`.
    pub(super) fn legacy_screen_origin(&self, batch: &HudBatch) -> Option<(f32, f32)> {
        let (w, h) = self.legacy_screen_dimensions()?;
        if self.screen == Screen::Creative {
            let ox = ((batch.width - w) * 0.5).floor();
            let oy = ((batch.height - 179.0) * 0.5).floor().max(33.0);
            return Some((ox, oy));
        }
        let (mut ox, mut oy) = batch.centered_origin(w, h);
        if self.legacy_crafting_open() {
            oy += 18.0;
        }
        if self.recipe_book_covers_shift_enabled() {
            ox += self.recipe_book_shift(batch.width);
        }
        Some((ox, oy))
    }

    #[inline]
    fn recipe_book_covers_shift_enabled(&self) -> bool {
        false
    }

    /// Legacy4J 16x16 item draw top-left coordinate `(ix, iy)` relative to `(ox, oy)`.
    pub(super) fn legacy_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        if !self.use_legacy_inventory_layout() {
            return None;
        }
        let center_16 = |x: f32, y: f32, sz: f32| (x + (sz - 16.0) * 0.5, y + (sz - 16.0) * 0.5);

        if self.screen == Screen::Creative {
            return match i {
                0..9 => Some(center_16(34.0 + i as f32 * 27.0, 176.0, 27.0)),
                _ => None,
            };
        }

        if self.legacy_crafting_open() || self.legacy_stonecutter_open() {
            return match i {
                0..9 => Some((186.0 + i as f32 * 16.0, 186.0)),
                9..36 => Some((
                    186.0 + ((i - 9) % 9) as f32 * 16.0,
                    133.0 + ((i - 9) / 9) as f32 * 16.0,
                )),
                _ => None,
            };
        }

        if self.legacy_merchant_open() {
            return match i {
                0..9 => Some((133.0 + i as f32 * 16.0, 154.0)),
                9..36 => Some((
                    133.0 + ((i - 9) % 9) as f32 * 16.0,
                    98.0 + ((i - 9) / 9) as f32 * 16.0,
                )),
                _ => None,
            };
        }

        if let Some(s) = self.station.as_ref() {
            return match s.kind {
                StationKind::Anvil => match i {
                    0..9 => Some(center_16(10.0 + i as f32 * 21.0, 185.0, 21.0)),
                    9..36 => Some(center_16(
                        10.0 + ((i - 9) % 9) as f32 * 21.0,
                        116.0 + ((i - 9) / 9) as f32 * 21.0,
                        21.0,
                    )),
                    x if x == STATION_INPUT => Some(center_16(15.0, 56.0, 30.0)),
                    x if x == STATION_INPUT + 1 => Some(center_16(84.0, 56.0, 30.0)),
                    x if x == STATION_RESULT => Some(center_16(167.0, 56.0, 30.0)),
                    _ => None,
                },
                StationKind::Grindstone => match i {
                    0..9 => Some(center_16(10.0 + i as f32 * 21.0, 185.0, 21.0)),
                    9..36 => Some(center_16(
                        10.0 + ((i - 9) % 9) as f32 * 21.0,
                        116.0 + ((i - 9) / 9) as f32 * 21.0,
                        21.0,
                    )),
                    x if x == STATION_INPUT => Some(center_16(41.0, 30.0, 30.0)),
                    x if x == STATION_INPUT + 1 => Some(center_16(41.0, 65.0, 30.0)),
                    x if x == STATION_RESULT => Some(center_16(138.0, 46.0, 30.0)),
                    _ => None,
                },
                StationKind::Smithing => match i {
                    0..9 => Some(center_16(10.0 + i as f32 * 21.0, 185.0, 21.0)),
                    9..36 => Some(center_16(
                        10.0 + ((i - 9) % 9) as f32 * 21.0,
                        116.0 + ((i - 9) / 9) as f32 * 21.0,
                        21.0,
                    )),
                    x if x == STATION_INPUT => Some(center_16(10.0, 60.0, 21.0)),
                    x if x == STATION_INPUT + 1 => Some(center_16(31.0, 60.0, 21.0)),
                    x if x == STATION_INPUT + 2 => Some(center_16(52.0, 60.0, 21.0)),
                    x if x == STATION_RESULT => Some(center_16(127.0, 56.0, 30.0)),
                    _ => None,
                },
                StationKind::Enchanting => match i {
                    0..9 => Some(center_16(14.0 + i as f32 * 21.0, 185.0, 21.0)),
                    9..36 => Some(center_16(
                        14.0 + ((i - 9) % 9) as f32 * 21.0,
                        115.0 + ((i - 9) / 9) as f32 * 21.0,
                        21.0,
                    )),
                    x if x == STATION_INPUT => Some(center_16(19.0, 66.0, 21.0)),
                    x if x == STATION_INPUT + 1 => Some(center_16(50.0, 66.0, 21.0)),
                    _ => None,
                },
                StationKind::Cartography => match i {
                    0..9 => Some(center_16(10.0 + i as f32 * 21.0, 225.0, 21.0)),
                    9..36 => Some(center_16(
                        10.0 + ((i - 9) % 9) as f32 * 21.0,
                        156.0 + ((i - 9) / 9) as f32 * 21.0,
                        21.0,
                    )),
                    x if x == STATION_INPUT => Some(center_16(10.0, 62.0, 23.0)),
                    x if x == STATION_INPUT + 1 => Some(center_16(10.0, 105.0, 23.0)),
                    x if x == STATION_RESULT => Some(center_16(166.0, 82.0, 27.0)),
                    _ => None,
                },
                StationKind::Beacon => match i {
                    0..9 => Some(center_16(36.0 + i as f32 * 21.0, 223.0, 21.0)),
                    9..36 => Some(center_16(
                        36.0 + ((i - 9) % 9) as f32 * 21.0,
                        155.0 + ((i - 9) / 9) as f32 * 21.0,
                        21.0,
                    )),
                    x if x == STATION_INPUT => Some(center_16(141.0, 129.0, 21.0)),
                    _ => None,
                },
                StationKind::Loom => match i {
                    0..9 => Some(center_16(14.0 + i as f32 * 21.0, 185.0, 21.0)),
                    9..36 => Some(center_16(
                        14.0 + ((i - 9) % 9) as f32 * 21.0,
                        115.0 + ((i - 9) / 9) as f32 * 21.0,
                        21.0,
                    )),
                    x if x == STATION_INPUT => Some(center_16(19.0, 41.0, 23.0)),
                    x if x == STATION_INPUT + 1 => Some(center_16(45.0, 41.0, 23.0)),
                    x if x == STATION_INPUT + 2 => Some(center_16(32.0, 66.0, 23.0)),
                    x if x == STATION_RESULT => Some(center_16(166.0, 75.0, 32.0)),
                    _ => None,
                },
                StationKind::Stonecutter | StationKind::Merchant => None,
            };
        }

        if let Some(cols) = self
            .mount_menu
            .chest_columns()
            .filter(|_| self.mount_active())
        {
            return match i {
                0..9 => Some(center_16(14.0 + i as f32 * 21.0, 174.0, 21.0)),
                9..36 => Some(center_16(
                    14.0 + ((i - 9) % 9) as f32 * 21.0,
                    104.0 + ((i - 9) / 9) as f32 * 21.0,
                    21.0,
                )),
                x if x == MOUNT_SADDLE => Some(center_16(14.0, 21.0, 21.0)),
                x if x == MOUNT_ARMOR => Some(center_16(14.0, 42.0, 21.0)),
                x if x >= MOUNT_CHEST && x - MOUNT_CHEST < cols * 3 => {
                    let n = x - MOUNT_CHEST;
                    Some(center_16(
                        98.0 + (n % cols) as f32 * 21.0,
                        21.0 + (n / cols) as f32 * 21.0,
                        21.0,
                    ))
                }
                _ => None,
            };
        }

        if self.brewing_open() {
            return match i {
                0..9 => Some(center_16(13.0 + i as f32 * 21.0, 195.0, 21.0)),
                9..36 => Some(center_16(
                    13.0 + ((i - 9) % 9) as f32 * 21.0,
                    126.0 + ((i - 9) / 9) as f32 * 21.0,
                    21.0,
                )),
                300 => Some(center_16(60.0, 76.0, 27.0)),
                301 => Some(center_16(94.0, 87.0, 27.0)),
                302 => Some(center_16(129.0, 76.0, 27.0)),
                303 => Some(center_16(94.0, 25.0, 27.0)),
                304 => Some(center_16(19.0, 25.0, 27.0)),
                _ => None,
            };
        }

        if let Some(screen) = self.machine_screen() {
            let y_diff = if screen == super::machines::Screen::Hopper {
                -42.0
            } else {
                0.0
            };
            return match i {
                0..9 => Some(center_16(14.0 + i as f32 * 21.0, 177.0 + y_diff, 21.0)),
                9..36 => Some(center_16(
                    14.0 + ((i - 9) % 9) as f32 * 21.0,
                    107.0 + ((i - 9) / 9) as f32 * 21.0 + y_diff,
                    21.0,
                )),
                300..500 if screen == super::machines::Screen::Hopper => {
                    Some(center_16(55.0 + (i - 300) as f32 * 21.0, 26.0, 21.0))
                }
                300..500 => {
                    let idx = i - 300;
                    Some(center_16(
                        76.0 + (idx % 3) as f32 * 21.0,
                        26.0 + (idx / 3) as f32 * 21.0,
                        21.0,
                    ))
                }
                _ => None,
            };
        }

        if let Some(rows) = self.chest_rows() {
            let y_diff = (rows as f32 - 3.0) * 21.0;
            return match i {
                0..9 => Some(center_16(14.0 + i as f32 * 21.0, 177.0 + y_diff, 21.0)),
                9..36 => Some(center_16(
                    14.0 + ((i - 9) % 9) as f32 * 21.0,
                    107.0 + ((i - 9) / 9) as f32 * 21.0 + y_diff,
                    21.0,
                )),
                300..500 => {
                    let idx = i - 300;
                    Some(center_16(
                        14.0 + (idx % 9) as f32 * 21.0,
                        26.0 + (idx / 9) as f32 * 21.0,
                        21.0,
                    ))
                }
                _ => None,
            };
        }

        match self.container_kind() {
            Some(horizon_client::containers::BlockEntity::Furnace(_)) => match i {
                0..9 => Some(center_16(14.0 + i as f32 * 21.0, 180.0, 21.0)),
                9..36 => Some(center_16(
                    14.0 + ((i - 9) % 9) as f32 * 21.0,
                    111.0 + ((i - 9) / 9) as f32 * 21.0,
                    21.0,
                )),
                300 => Some(center_16(77.0, 25.0, 21.0)),
                301 => Some(center_16(77.0, 72.0, 21.0)),
                302 => Some(center_16(155.0, 44.0, 32.0)),
                _ => None,
            },
            Some(horizon_client::containers::BlockEntity::Crafter(_)) => match i {
                0..9 => Some(center_16(14.0 + i as f32 * 21.0, 171.0, 21.0)),
                9..36 => Some(center_16(
                    14.0 + ((i - 9) % 9) as f32 * 21.0,
                    102.0 + ((i - 9) / 9) as f32 * 21.0,
                    21.0,
                )),
                300..309 => {
                    let idx = i - 300;
                    Some(center_16(
                        34.0 + (idx % 3) as f32 * 21.0,
                        23.0 + (idx / 3) as f32 * 21.0,
                        21.0,
                    ))
                }
                309 => Some(center_16(150.0, 39.0, 32.0)),
                _ => None,
            },
            Some(_) => None,
            None => match i {
                // InventoryScreenMixin (hasClassicCrafting == false): centered armor + portrait + shield.
                0..9 => Some(center_16(14.0 + i as f32 * 21.0, 186.0, 21.0)),
                9..36 => Some(center_16(
                    14.0 + ((i - 9) % 9) as f32 * 21.0,
                    116.0 + ((i - 9) / 9) as f32 * 21.0,
                    21.0,
                )),
                500..504 => Some(center_16(64.0, 14.0 + (i - 500) as f32 * 21.0, 21.0)),
                504 => Some(center_16(161.0, 77.0, 21.0)),
                _ => None,
            },
        }
    }
}
