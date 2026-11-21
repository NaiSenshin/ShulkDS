//! 1:1 Legacy4J crafting-style screens (`348x215` `LegacyCraftingScreen`,
//! `348x215` `LegacyStonecutterScreen`, and `294x181` `LegacyMerchantScreen`).

use super::{
    Game, Screen,
    stations::{INPUT as STATION_INPUT, RESULT as STATION_RESULT},
};
use horizon_client::hud::{HudBatch, rgba};
use horizon_client::items::ItemStack;
use horizon_client::menu::Input as MenuInput;
use horizon_client::merchant::MerchantOffer;
use horizon_client::recipe_book::Display;
use horizon_client::text::Text;
use horizon_client::ui::blit_sprite;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

#[derive(Clone, Copy)]
enum TabFilter {
    Crafting,
    Stonecutter,
    Merchant,
}

struct RawCraftingGroup {
    tokens: Vec<String>,
}

struct RawCraftingTab {
    id: String,
    title_key: String,
    title_fallback: String,
    icon_sprite: String,
    groups: Vec<RawCraftingGroup>,
}

#[derive(Default)]
pub(super) struct LegacyCraftingState {
    pub(super) pocket_crafting: bool,
    pub(super) tab: usize,
    pub(super) strip_offset: usize,
    pub(super) selected_slot: usize,
    pub(super) variant_index: usize,
    pub(super) variant_offset: i32,
    pub(super) stone_offset: usize,
    pub(super) stone_selected: usize,
    pub(super) stone_variant: usize,
    pub(super) stone_variant_offset: i32,
    pub(super) merchant_offset: usize,
    pub(super) merchant_selected: usize,
}

fn crafting_tab_fallback(id: &str) -> &'static str {
    match id {
        "structures" => "Structures",
        "tools" => "Tools & Weapons",
        "food" => "Food",
        "armour" => "Armour",
        "mechanisms" => "Mechanisms",
        "transport" => "Transport",
        "decoration" => "Decorations",
        "materials" | "legacy:materials" => "Materials",
        _ => "Crafting",
    }
}

fn crafting_tab_metrics(count: usize) -> (f32, f32) {
    if count <= 1 {
        (51.0, 0.0)
    } else if count == 7 {
        (51.0, 49.5)
    } else {
        let w = (348.0 / count as f32 + 1.5).round();
        (w, (348.0 - w) / (count - 1) as f32)
    }
}

fn legacy_strip_scrollbar_hit(mx: f32, my: f32, ox: f32, oy: f32, total: usize) -> Option<usize> {
    let max_offset = total.saturating_sub(12);
    if max_offset == 0 {
        return None;
    }
    if mx >= ox + 12.0 && mx < ox + 336.0 && my >= oy + 70.0 && my < oy + 79.0 {
        let frac = ((mx - (ox + 12.0)) / 324.0).clamp(0.0, 0.9999);
        let next = (frac * (max_offset + 1) as f32).floor() as usize;
        Some(next.min(max_offset))
    } else {
        None
    }
}

fn normalize_recipe_token(raw: &str, item_only: bool) -> String {
    let clean = raw
        .strip_prefix("minecraft:")
        .or_else(|| raw.strip_prefix("legacy:"))
        .unwrap_or(raw);
    if item_only {
        format!("item:{clean}")
    } else {
        clean.to_owned()
    }
}

fn raw_crafting_tabs() -> &'static [RawCraftingTab] {
    static TABS: OnceLock<Vec<RawCraftingTab>> = OnceLock::new();
    TABS.get_or_init(|| {
        let raw = include_str!("../../assets/legacy/crafting_tab_listing.json");
        let parsed: Value = serde_json::from_str(raw).unwrap_or(Value::Array(Vec::new()));
        let mut out = Vec::new();
        let Some(arr) = parsed.as_array() else {
            return out;
        };
        for entry in arr {
            let id = entry["id"].as_str().unwrap_or_default().to_owned();
            let title_key = entry["name"]["translate"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let icon_raw = entry["icon"]["value"]["default"]
                .as_str()
                .unwrap_or("legacy:icon/structures");
            let icon_sprite = icon_raw.replacen("legacy:", "legacy/", 1);
            let mut groups = Vec::new();
            if let Some(listing) = entry["listing"].as_array() {
                for item in listing {
                    if let Some(s) = item.as_str() {
                        groups.push(RawCraftingGroup {
                            tokens: vec![normalize_recipe_token(s, false)],
                        });
                    } else if let Some(arr) = item["recipes"].as_array() {
                        let tokens: Vec<String> = arr
                            .iter()
                            .filter_map(|v| {
                                let raw = v
                                    .as_str()
                                    .or_else(|| v["value"].as_str())
                                    .or_else(|| v["id"].as_str())?;
                                let item_only = v["type"].as_str() == Some("item_id");
                                Some(normalize_recipe_token(raw, item_only))
                            })
                            .collect();
                        if !tokens.is_empty() {
                            groups.push(RawCraftingGroup { tokens });
                        }
                    } else if let Some(s) = item["value"].as_str().or_else(|| item["id"].as_str()) {
                        let item_only = item["type"].as_str() == Some("item_id");
                        groups.push(RawCraftingGroup {
                            tokens: vec![normalize_recipe_token(s, item_only)],
                        });
                    }
                }
            }
            out.push(RawCraftingTab {
                id: id.clone(),
                title_fallback: crafting_tab_fallback(&id).to_owned(),
                title_key,
                icon_sprite,
                groups,
            });
        }
        out
    })
}

fn stonecutting_bucket_order() -> &'static [(String, Vec<String>)] {
    static BUCKETS: OnceLock<Vec<(String, Vec<String>)>> = OnceLock::new();
    BUCKETS.get_or_init(|| {
        let raw = include_str!("../../assets/legacy/stonecutting_groups.json");
        let parsed: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
        let mut out = Vec::new();
        if let Some(obj) = parsed.as_object() {
            for (k, v) in obj {
                let vals = if let Some(arr) = v.as_array() {
                    arr.iter()
                        .filter_map(|e| e.as_str().map(ToOwned::to_owned))
                        .collect()
                } else if let Some(s) = v["value"].as_str() {
                    vec![s.to_owned()]
                } else {
                    Vec::new()
                };
                out.push((k.trim_end_matches(':').to_owned(), vals));
            }
        }
        out
    })
}

fn stonecutting_bucket_rank(item_id: &str) -> usize {
    let short = item_id.strip_prefix("minecraft:").unwrap_or(item_id);
    for (idx, (key, _)) in stonecutting_bucket_order().iter().enumerate() {
        match key.as_str() {
            "slabs" if short.ends_with("_slab") => return idx,
            "stairs" if short.ends_with("_stairs") => return idx,
            "walls" if short.ends_with("_wall") => return idx,
            _ => {}
        }
    }
    stonecutting_bucket_order().len()
}

fn villager_level_min_xp(level: i32) -> i32 {
    match level {
        1 => 0,
        2 => 10,
        3 => 70,
        4 => 150,
        5 => 250,
        _ => 250,
    }
}

fn villager_level_max_xp(level: i32) -> i32 {
    match level {
        1 => 10,
        2 => 70,
        3 => 150,
        4 => 250,
        _ => 250,
    }
}

impl Game<'_> {
    fn player_inv_counts(&self) -> HashMap<String, u32> {
        let mut counts = HashMap::new();
        for stack in self.inventory.iter().take(36).flatten() {
            *counts.entry(stack.item.to_string()).or_insert(0) += stack.count;
        }
        counts
    }

    fn crafting_available_counts(&self) -> HashMap<String, u32> {
        let mut counts = self.player_inv_counts();
        let grid_len = self.craft_size.min(3) * self.craft_size.min(3);
        for stack in self.craft_grid.iter().take(grid_len).flatten() {
            *counts.entry(stack.item.to_string()).or_insert(0) += stack.count;
        }
        counts
    }

    fn stonecutter_available_counts(&self) -> HashMap<String, u32> {
        let mut counts = self.player_inv_counts();
        if let Some(s) = self.station.as_ref()
            && let Some(inp) = &s.inputs[0]
        {
            *counts.entry(inp.item.to_string()).or_insert(0) += inp.count;
        }
        counts
    }

    fn merchant_available_counts(&self) -> HashMap<String, u32> {
        let mut counts = self.player_inv_counts();
        if let Some(s) = self.station.as_ref() {
            for inp in s.inputs.iter().take(2).flatten() {
                *counts.entry(inp.item.to_string()).or_insert(0) += inp.count;
            }
        }
        counts
    }

    fn can_fit_in_player_inv(&self, result: &ItemStack) -> bool {
        let max_stack = self
            .items()
            .get(&result.item)
            .map_or(64, |info| info.max_stack);
        let mut space = 0u32;
        for slot in self.inventory.iter().take(36) {
            match slot {
                None => return true,
                Some(existing)
                    if existing.same_item_same_components(result)
                        && existing.count < max_stack =>
                {
                    space += max_stack - existing.count;
                    if space >= result.count {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn will_free_craft_slot(&self, cells: &[(Option<ItemStack>, bool); 9]) -> bool {
        let mut needed: HashMap<&str, u32> = HashMap::new();
        for (stack, _) in cells {
            if let Some(st) = stack {
                *needed.entry(&*st.item).or_insert(0) += 1;
            }
        }
        self.inventory
            .iter()
            .take(36)
            .flatten()
            .any(|st| needed.get(&*st.item).copied().unwrap_or(0) >= st.count)
    }

    fn match_tab_groups(
        tab: &RawCraftingTab,
        id_map: &HashMap<&str, usize>,
        result_map: &HashMap<&str, Vec<usize>>,
        seen_global: &mut HashSet<usize>,
    ) -> Vec<Vec<usize>> {
        let mut groups = Vec::new();
        for raw_group in &tab.groups {
            let mut matched = Vec::new();
            let mut seen = HashSet::new();
            for tok in &raw_group.tokens {
                if let Some(item_name) = tok.strip_prefix("item:") {
                    if let Some(list) = result_map.get(item_name) {
                        for &idx in list {
                            if seen.insert(idx) {
                                matched.push(idx);
                                seen_global.insert(idx);
                            }
                        }
                    }
                } else if let Some(rec_name) = tok.strip_prefix("recipe:") {
                    if let Some(&idx) = id_map.get(rec_name)
                        && seen.insert(idx)
                    {
                        matched.push(idx);
                        seen_global.insert(idx);
                    }
                } else {
                    if let Some(&idx) = id_map.get(tok.as_str())
                        && seen.insert(idx)
                    {
                        matched.push(idx);
                        seen_global.insert(idx);
                    }
                    if let Some(list) = result_map.get(tok.as_str()) {
                        for &idx in list {
                            if seen.insert(idx) {
                                matched.push(idx);
                                seen_global.insert(idx);
                            }
                        }
                    }
                }
            }
            if !matched.is_empty() {
                groups.push(matched);
            }
        }
        groups
    }

    fn crafting_tab_groups(&self, tab_idx: usize) -> Vec<Vec<usize>> {
        let tabs = raw_crafting_tabs();
        let Some(tab) = tabs.get(tab_idx) else {
            return Vec::new();
        };
        let entries = self
            .recipe_ui
            .book
            .as_ref()
            .map_or(&[][..], |b| b.entries.as_slice());
        let grid_lim = if self.craft_size == 3 { 3 } else { 2 };
        let mut result_map: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut id_map: HashMap<&str, usize> = HashMap::new();
        for (idx, entry) in entries.iter().enumerate() {
            if matches!(entry.display, Display::Furnace(_)) || !entry.fits(grid_lim, grid_lim) {
                continue;
            }
            let res_short = entry
                .result
                .item
                .strip_prefix("minecraft:")
                .unwrap_or(&entry.result.item);
            result_map.entry(res_short).or_default().push(idx);
            let id_short = entry.id.strip_prefix("minecraft:").unwrap_or(&entry.id);
            id_map.entry(id_short).or_insert(idx);
        }

        let mut seen_global = HashSet::new();
        let mut groups = if tab.id == "materials" {
            let mut target_groups = Vec::new();
            for (idx, t) in tabs.iter().enumerate() {
                let matched = Self::match_tab_groups(t, &id_map, &result_map, &mut seen_global);
                if idx == tab_idx {
                    target_groups = matched;
                }
            }
            target_groups
        } else {
            Self::match_tab_groups(tab, &id_map, &result_map, &mut seen_global)
        };

        if tab.id == "materials" {
            let mut extra_order: Vec<&str> = Vec::new();
            let mut extra_groups: HashMap<&str, Vec<usize>> = HashMap::new();
            for (idx, entry) in entries.iter().enumerate() {
                if matches!(entry.display, Display::Furnace(_)) || !entry.fits(grid_lim, grid_lim)
                {
                    continue;
                }
                if seen_global.contains(&idx) {
                    continue;
                }
                let res_short = entry
                    .result
                    .item
                    .strip_prefix("minecraft:")
                    .unwrap_or(&entry.result.item);
                if !extra_groups.contains_key(res_short) {
                    extra_order.push(res_short);
                }
                extra_groups.entry(res_short).or_default().push(idx);
            }
            for key in extra_order {
                if let Some(group) = extra_groups.remove(key) {
                    groups.push(group);
                }
            }
        }
        groups
    }

    fn recipe_craftable_and_grid(
        &self,
        entry_idx: usize,
        counts: &HashMap<String, u32>,
    ) -> (bool, [(Option<ItemStack>, bool); 9], bool) {
        let mut cells: [(Option<ItemStack>, bool); 9] = Default::default();
        let Some(entry) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_idx))
        else {
            return (false, cells, true);
        };
        let mut rem = counts.clone();
        let mut can_craft = true;
        let mut any_warning = false;
        let tick_phase = (self.ticks / 16) as usize;

        let resolve_cell = |ing: &horizon_client::crafting::Ingredient,
                            rem: &mut HashMap<String, u32>|
         -> (Option<ItemStack>, bool) {
            for cand in &ing.order {
                if let Some(c) = rem.get_mut(cand.as_str())
                    && *c > 0
                {
                    *c -= 1;
                    return (Some(ItemStack::new(cand, 1)), false);
                }
            }
            let shown = if ing.order.is_empty() {
                None
            } else {
                Some(ItemStack::new(&ing.order[tick_phase % ing.order.len()], 1))
            };
            (shown, true)
        };

        match &entry.display {
            &Display::Shaped {
                width,
                height,
                cells: ref recipe_cells,
            } => {
                for r in 0..height.min(3) {
                    for c in 0..width.min(3) {
                        if let Some(Some(ing)) = recipe_cells.get(r * width + c) {
                            let (stack, warn) = resolve_cell(ing, &mut rem);
                            if warn {
                                can_craft = false;
                                any_warning = true;
                            }
                            cells[r * 3 + c] = (stack, warn);
                        }
                    }
                }
            }
            Display::Shapeless(list) => {
                let grid_cols = if self.craft_size == 3 { 3 } else { 2 };
                let max_cells = grid_cols * grid_cols;
                for (i, ing) in list.iter().take(max_cells).enumerate() {
                    let (stack, warn) = resolve_cell(ing, &mut rem);
                    if warn {
                        can_craft = false;
                        any_warning = true;
                    }
                    let slot_idx = (i / grid_cols) * 3 + (i % grid_cols);
                    cells[slot_idx] = (stack, warn);
                }
            }
            Display::Furnace(_) => {
                can_craft = false;
                any_warning = true;
            }
        }
        (can_craft, cells, any_warning)
    }

    fn focused_variant_pos(variants_len: usize, cur: usize, offset: i32) -> usize {
        if variants_len == 0 {
            return 0;
        }
        ((cur as isize + offset as isize).rem_euclid(variants_len as isize)) as usize
    }

    /// Draw `crafting_selection` (`36x105`) / `crafting_2_slots_selection` (`36x78`) + `select_icon_highlight` (`36x36`)
    /// around `(sx, sy)` and return hovered `(ItemStack, tooltip_rect)` if any popout neighbor is hovered.
    #[allow(clippy::too_many_arguments)]
    fn draw_variant_popout(
        &self,
        batch: &mut HudBatch,
        sx: f32,
        sy: f32,
        variants: &[(ItemStack, bool)],
        cur: usize,
        offset: i32,
    ) -> Option<ItemStack> {
        let len = variants.len();
        if len == 0 {
            return None;
        }
        let mut hovered = None;
        if len == 2 {
            let other = (cur + 1) % 2;
            blit_sprite(
                batch,
                &self.gui.atlas,
                "legacy/container/crafting_2_slots_selection",
                sx - 4.5,
                sy - 12.0,
                36.0,
                78.0,
                u32::MAX,
            );
            let (ref st, can) = variants[other];
            self.draw_legacy_item_in_box(batch, st, sx, sy + 27.0, 27.0, if can { 1.0 } else { 0.5 });
            if self.cursor.0 >= sx
                && self.cursor.0 < sx + 27.0
                && self.cursor.1 >= sy + 27.0
                && self.cursor.1 < sy + 54.0
            {
                hovered = Some(st.clone());
            }
        } else if len >= 3 {
            let prev = (cur + len - 1) % len;
            let next = (cur + 1) % len;
            blit_sprite(
                batch,
                &self.gui.atlas,
                "legacy/container/crafting_selection",
                sx - 4.5,
                sy - 39.0,
                36.0,
                105.0,
                u32::MAX,
            );
            let (ref st_prev, can_prev) = variants[prev];
            self.draw_legacy_item_in_box(
                batch,
                st_prev,
                sx,
                sy - 27.0,
                27.0,
                if can_prev { 1.0 } else { 0.5 },
            );
            if self.cursor.0 >= sx
                && self.cursor.0 < sx + 27.0
                && self.cursor.1 >= sy - 27.0
                && self.cursor.1 < sy
            {
                hovered = Some(st_prev.clone());
            }
            let (ref st_next, can_next) = variants[next];
            self.draw_legacy_item_in_box(
                batch,
                st_next,
                sx,
                sy + 27.0,
                27.0,
                if can_next { 1.0 } else { 0.5 },
            );
            if self.cursor.0 >= sx
                && self.cursor.0 < sx + 27.0
                && self.cursor.1 >= sy + 27.0
                && self.cursor.1 < sy + 54.0
            {
                hovered = Some(st_next.clone());
            }
        }
        let sel_y = sy + offset as f32 * 27.0;
        let (ref center_st, center_can) = variants[cur];
        self.draw_legacy_item_in_box(
            batch,
            center_st,
            sx,
            sy,
            27.0,
            if center_can { 1.0 } else { 0.5 },
        );
        if self.cursor.0 >= sx
            && self.cursor.0 < sx + 27.0
            && self.cursor.1 >= sy
            && self.cursor.1 < sy + 27.0
        {
            hovered = Some(center_st.clone());
        }
        blit_sprite(
            batch,
            &self.gui.atlas,
            "legacy/container/select_icon_highlight",
            sx - 4.5,
            sel_y - 4.5,
            36.0,
            36.0,
            u32::MAX,
        );
        hovered
    }

    fn draw_legacy_strip_scrollbar(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
        offset: usize,
        total: usize,
    ) {
        if total <= 12 {
            return;
        }
        let atlas = &self.gui.atlas;
        let tx = ox + 12.0;
        let ty = oy + 72.0;
        let tw = 324.0;
        // Sunken 5px LCE scrollbar groove (`#373737` top/left shadow, `#8B8B8B` track, `#FFFFFF` bottom/right).
        batch.fill(atlas, tx, ty, tw, 5.0, rgba(55, 55, 55, 255));
        batch.fill(atlas, tx + 1.0, ty + 1.0, tw - 1.0, 4.0, rgba(255, 255, 255, 255));
        batch.fill(atlas, tx + 1.0, ty + 1.0, tw - 2.0, 3.0, rgba(139, 139, 139, 255));
        let inner_w = tw - 2.0;
        let thumb_w = ((12.0 / total as f32) * inner_w)
            .round()
            .clamp(18.0, inner_w);
        let max_off = (total - 12) as f32;
        let thumb_x = tx
            + 1.0
            + ((offset.min(total - 12) as f32 / max_off) * (inner_w - thumb_w)).round();
        batch.fill(atlas, thumb_x, ty + 1.0, thumb_w, 1.0, rgba(245, 245, 245, 255));
        batch.fill(atlas, thumb_x, ty + 2.0, thumb_w, 1.0, rgba(198, 198, 198, 255));
        batch.fill(atlas, thumb_x, ty + 3.0, thumb_w, 1.0, rgba(85, 85, 85, 255));
    }

    fn draw_legacy_strip_chevrons(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
        offset: usize,
        total: usize,
    ) {
        if total <= 12 {
            return;
        }
        let left_tint = if offset > 0 {
            u32::MAX
        } else {
            rgba(130, 130, 130, 110)
        };
        blit_sprite(
            batch,
            &self.gui.atlas,
            "legacy/widget/scroll_left",
            ox + 3.0,
            oy + 46.0,
            6.0,
            11.0,
            left_tint,
        );
        let right_tint = if offset + 12 < total {
            u32::MAX
        } else {
            rgba(130, 130, 130, 110)
        };
        blit_sprite(
            batch,
            &self.gui.atlas,
            "legacy/widget/scroll_right",
            ox + 339.0,
            oy + 46.0,
            6.0,
            11.0,
            right_tint,
        );
    }

    pub(super) fn draw_legacy_crafting(&mut self, batch: &mut HudBatch) {
        self.ensure_book();
        let tabs = raw_crafting_tabs();
        if tabs.is_empty() {
            return;
        }
        let Some((ox, oy)) = self.legacy_screen_origin(batch) else {
            return;
        };
        {
            let atlas = &self.gui.atlas;
            batch.fill(
                atlas,
                0.0,
                0.0,
                batch.width,
                batch.height,
                rgba(16, 16, 16, 160),
            );
        }

        let tab_idx = self.legacy_crafting.tab.min(tabs.len() - 1);
        self.legacy_crafting.tab = tab_idx;

        let mut hovered_tab: Option<usize> = None;

        // 1. Top tabs (`LegacyCraftingScreen.java:638-644`, scaled across `348` width).
        let (tab_w, tab_step) = crafting_tab_metrics(tabs.len());
        for (idx, t) in tabs.iter().enumerate() {
            let tx = ox + idx as f32 * tab_step;
            let hit_w = if idx + 1 == tabs.len() {
                tab_w
            } else {
                tab_step
            };
            if self.cursor.0 >= tx
                && self.cursor.0 < tx + hit_w
                && self.cursor.1 >= oy - 37.0
                && self.cursor.1 < oy
            {
                hovered_tab = Some(idx);
            }
            if idx == tab_idx {
                continue;
            }
            let ty = oy - 37.0 + 4.4;
            self.legacy_tab_bg(batch, "legacy/tiles/low_tab", tx, ty, tab_w, 43.0);
            blit_sprite(
                batch,
                &self.gui.atlas,
                &t.icon_sprite,
                tx + (tab_w - 24.0) * 0.5,
                ty + 7.0,
                24.0,
                24.0,
                u32::MAX,
            );
        }

        // 2. Main 348x215 panel.
        self.legacy_panel(batch, ox, oy, 348.0, 215.0);

        // 3. Selected tab over panel top edge.
        {
            let sprite = if tab_idx == 0 {
                "legacy/tiles/high_tab_left"
            } else if tab_idx + 1 == tabs.len() {
                "legacy/tiles/high_tab_right"
            } else {
                "legacy/tiles/high_tab_middle"
            };
            let tx = ox + tab_idx as f32 * tab_step;
            let ty = oy - 37.0;
            self.legacy_tab_bg(batch, sprite, tx, ty, tab_w, 43.0);
            blit_sprite(
                batch,
                &self.gui.atlas,
                &tabs[tab_idx].icon_sprite,
                tx + (tab_w - 24.0) * 0.5,
                ty + 6.0,
                24.0,
                24.0,
                u32::MAX,
            );
        }

        // 4. Category title.
        let tab_title = self.translate(&tabs[tab_idx].title_key, &tabs[tab_idx].title_fallback);
        super::screen_info::name_container(|| tab_title.clone());
        {
            let atlas = &self.gui.atlas;
            let tw = HudBatch::text_width(atlas, &tab_title);
            batch.text(
                atlas,
                &tab_title,
                ox + ((348.0 - tw) * 0.5).floor(),
                oy + 17.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }

        // 5. Group strip (`12` slots centered at `ox + 12 + i*27, oy + 38`).
        let groups = self.crafting_tab_groups(tab_idx);
        let max_offset = groups.len().saturating_sub(12);
        let strip_offset = self.legacy_crafting.strip_offset.min(max_offset);
        self.legacy_crafting.strip_offset = strip_offset;
        let visible_groups = groups.len().saturating_sub(strip_offset).min(12);
        let sel_slot = if visible_groups > 0 {
            self.legacy_crafting.selected_slot.min(visible_groups - 1)
        } else {
            0
        };
        self.legacy_crafting.selected_slot = sel_slot;

        let counts = self.crafting_available_counts();
        let mut hovered_stack: Option<ItemStack> = None;

        for i in 0..12 {
            let sx = ox + 12.0 + i as f32 * 27.0;
            let sy = oy + 38.0;
            self.legacy_slot_box(batch, sx, sy, 27.0);
            let is_focused = i == sel_slot
                && groups
                    .get(strip_offset + i)
                    .is_some_and(|g| !g.is_empty());
            if !is_focused
                && let Some(group) = groups.get(strip_offset + i)
                && let Some(&first_idx) = group.first()
                && let Some(entry) = self
                    .recipe_ui
                    .book
                    .as_ref()
                    .and_then(|b| b.entries.get(first_idx))
            {
                let result = entry.result.clone();
                let (can, _, _) = self.recipe_craftable_and_grid(first_idx, &counts);
                self.draw_legacy_item_in_box(batch, &result, sx, sy, 27.0, if can { 1.0 } else { 0.5 });
                if self.cursor.0 >= sx
                    && self.cursor.0 < sx + 27.0
                    && self.cursor.1 >= sy
                    && self.cursor.1 < sy + 27.0
                {
                    hovered_stack = Some(result);
                }
            }
        }

        // Compute focused recipe entry from selected group & variant popout.
        let focused_group = groups.get(strip_offset + sel_slot).cloned().unwrap_or_default();
        let variant_count = focused_group.len();
        if variant_count > 0 {
            self.legacy_crafting.variant_index %= variant_count;
            let max_off = if variant_count >= 2 { 1 } else { 0 };
            let min_off = if variant_count >= 3 { -1 } else { 0 };
            self.legacy_crafting.variant_offset =
                self.legacy_crafting.variant_offset.clamp(min_off, max_off);
        } else {
            self.legacy_crafting.variant_index = 0;
            self.legacy_crafting.variant_offset = 0;
        }
        let cur_var = self.legacy_crafting.variant_index;
        let var_off = self.legacy_crafting.variant_offset;
        let focused_entry_idx = if variant_count > 0 {
            focused_group.get(Self::focused_variant_pos(variant_count, cur_var, var_off)).copied()
        } else {
            None
        };

        // 6. Bottom-left crafting blueprint recess (`9, 103, 163x105`).
        self.legacy_recess(batch, ox + 9.0, oy + 103.0, 163.0, 105.0);
        if let Some(entry_idx) = focused_entry_idx
            && let Some(result) = self
                .recipe_ui
                .book
                .as_ref()
                .and_then(|b| b.entries.get(entry_idx))
                .map(|e| e.result.clone())
        {
            let result_name = horizon_client::graphics_style::fit(
                &self.gui.atlas,
                &self.pack.names.display_item(&result.item),
                155.0,
            );
            let (_, cells, any_warning) = self.recipe_craftable_and_grid(entry_idx, &counts);
            {
                let atlas = &self.gui.atlas;
                let rw = HudBatch::text_width(atlas, &result_name);
                let tx = ox + 9.0 + ((163.0 - rw) * 0.5).floor();
                batch.text(atlas, &result_name, tx, oy + 114.0, rgba(64, 64, 64, 255), false);
            }
            let grid_n = if self.craft_size == 3 { 3 } else { 2 };
            let grid_x0 = if grid_n == 3 { ox + 21.0 } else { ox + 32.5 };
            let grid_y0 = if grid_n == 3 { oy + 133.0 } else { oy + 144.5 };
            for r in 0..grid_n {
                for c in 0..grid_n {
                    let gx = grid_x0 + c as f32 * 23.0;
                    let gy = grid_y0 + r as f32 * 23.0;
                    let (ref st_opt, warn) = cells[r * 3 + c];
                    let has_item = st_opt.is_some();
                    self.legacy_icon_holder(batch, gx, gy, 23.0, has_item && warn, false);
                    if let Some(st) = st_opt {
                        self.draw_legacy_item_in_box(batch, st, gx, gy, 23.0, 1.0);
                        if warn {
                            self.legacy_warning_icon(batch, gx, gy);
                        }
                        if self.cursor.0 >= gx
                            && self.cursor.0 < gx + 23.0
                            && self.cursor.1 >= gy
                            && self.cursor.1 < gy + 23.0
                        {
                            hovered_stack = Some(st.clone());
                        }
                    }
                }
            }
            self.legacy_slot_with(
                batch,
                "legacy/container/small_arrow",
                ox + 97.5,
                oy + 160.0,
                16.0,
                14.0,
            );
            let rx = ox + 124.5;
            let ry = oy + 151.0;
            self.legacy_icon_holder(batch, rx, ry, 36.0, any_warning, false);
            self.draw_legacy_item_in_box(batch, &result, rx, ry, 36.0, 1.0);
            if any_warning {
                self.legacy_warning_icon(batch, rx, ry);
            }
            if self.cursor.0 >= rx
                && self.cursor.0 < rx + 36.0
                && self.cursor.1 >= ry
                && self.cursor.1 < ry + 36.0
            {
                hovered_stack = Some(result);
            }
        } else {
            let grid_n = if self.craft_size == 3 { 3 } else { 2 };
            let grid_x0 = if grid_n == 3 { ox + 21.0 } else { ox + 32.5 };
            let grid_y0 = if grid_n == 3 { oy + 133.0 } else { oy + 144.5 };
            for r in 0..grid_n {
                for c in 0..grid_n {
                    self.legacy_slot_box(
                        batch,
                        grid_x0 + c as f32 * 23.0,
                        grid_y0 + r as f32 * 23.0,
                        23.0,
                    );
                }
            }
            self.legacy_slot_with(
                batch,
                "legacy/container/small_arrow",
                ox + 97.5,
                oy + 160.0,
                16.0,
                14.0,
            );
            self.legacy_slot_box(batch, ox + 124.5, oy + 151.0, 36.0);
        }

        // 7. Bottom-right `16x16` player inventory recess (`176, 103, 163x105`).
        self.legacy_recess(batch, ox + 176.0, oy + 103.0, 163.0, 105.0);
        {
            let atlas = &self.gui.atlas;
            let inv_lbl = self.translate("container.inventory", "Inventory");
            let iw = HudBatch::text_width(atlas, &inv_lbl);
            batch.text(
                atlas,
                &inv_lbl,
                ox + 176.0 + ((163.0 - iw) * 0.5).floor(),
                oy + 114.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }
        self.legacy_player_inv_16(batch, ox, oy, 186.0, 133.0, 186.0);
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        self.draw_inventory_slots(batch, ox, oy);
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        if hovered_stack.is_none()
            && let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1)
        {
            hovered_stack = self.slot_view(slot);
        }

        // 8. Horizontal overflow track, selected variant popout, and strip chevrons.
        self.draw_legacy_strip_scrollbar(batch, ox, oy, strip_offset, groups.len());
        if !focused_group.is_empty() {
            let mut variants = Vec::with_capacity(focused_group.len());
            for &idx in &focused_group {
                if let Some(result) = self
                    .recipe_ui
                    .book
                    .as_ref()
                    .and_then(|b| b.entries.get(idx))
                    .map(|e| e.result.clone())
                {
                    let (can, _, _) = self.recipe_craftable_and_grid(idx, &counts);
                    variants.push((result, can));
                }
            }
            let sx = ox + 12.0 + sel_slot as f32 * 27.0;
            let sy = oy + 38.0;
            if let Some(pop_tip) = self.draw_variant_popout(batch, sx, sy, &variants, cur_var, var_off)
            {
                hovered_stack = Some(pop_tip);
            }
        }
        self.draw_legacy_strip_chevrons(batch, ox, oy, strip_offset, groups.len());

        // 9. Carried item + tooltip.
        self.draw_carried(batch);
        if self.carried.is_none() {
            if let Some(stack) = hovered_stack {
                self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
            } else if let Some(ht) = hovered_tab {
                let name = self.translate(&tabs[ht].title_key, &tabs[ht].title_fallback);
                let text = Text::literal(name);
                self.draw_tooltip_lines(batch, &[(text, u32::MAX)], self.cursor.0, self.cursor.1);
            }
        }
    }

    fn execute_craft_entry(&mut self, entry_index: usize, craft_all: bool) {
        self.ensure_book();
        let Some(entry) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .cloned()
        else {
            return;
        };
        let counts = self.crafting_available_counts();
        let (can_craft, cells, _) = self.recipe_craftable_and_grid(entry_index, &counts);
        if !can_craft {
            return;
        }
        if !self.can_fit_in_player_inv(&entry.result) && !self.will_free_craft_slot(&cells) {
            return;
        }
        self.shulkds_place_recipe(entry_index, craft_all);
        if self.remote.is_some() {
            let _ = self.remote_menu_ops(&[(Some(200), 0, MenuInput::QuickMove)]);
        } else if self.slot_view(200).is_none() {
            return;
        } else if craft_all {
            self.craft_all_from_grid();
        } else {
            self.craft_one_from_grid();
        }
        self.audio
            .play("minecraft:entity.item.pickup", None, 0.25, 1.0);
    }

    pub(super) fn click_legacy_crafting(&mut self, right: bool) -> bool {
        if !self.legacy_crafting_open() {
            return false;
        }
        let Some(batch) = self.gui_scale() else {
            return true;
        };
        let Some((ox, oy)) = self.legacy_screen_origin(&batch) else {
            return true;
        };
        self.ensure_book();
        let (mx, my) = self.cursor;
        let tabs = raw_crafting_tabs();

        // 1. Top tabs.
        let (tab_w, tab_step) = crafting_tab_metrics(tabs.len());
        if !right && my >= oy - 37.0 && my < oy {
            for idx in 0..tabs.len() {
                let tx = ox + idx as f32 * tab_step;
                let hit_w = if idx + 1 == tabs.len() {
                    tab_w
                } else {
                    tab_step
                };
                if mx >= tx && mx < tx + hit_w {
                    self.legacy_crafting.tab = idx;
                    self.legacy_crafting.strip_offset = 0;
                    self.legacy_crafting.selected_slot = 0;
                    self.legacy_crafting.variant_index = 0;
                    self.legacy_crafting.variant_offset = 0;
                    self.audio.play_button_click();
                    return true;
                }
            }
        }

        let groups = self.crafting_tab_groups(self.legacy_crafting.tab);
        let max_offset = groups.len().saturating_sub(12);
        let strip_offset = self.legacy_crafting.strip_offset.min(max_offset);
        let visible_groups = groups.len().saturating_sub(strip_offset).min(12);
        let sel_slot = self
            .legacy_crafting
            .selected_slot
            .min(visible_groups.saturating_sub(1));

        // 2. Strip arrows.
        if !right && my >= oy + 38.0 && my < oy + 65.0 {
            if strip_offset > 0 && mx >= ox && mx < ox + 12.0 {
                self.legacy_crafting.strip_offset = strip_offset - 1;
                self.legacy_crafting.variant_index = 0;
                self.legacy_crafting.variant_offset = 0;
                self.audio.play_button_click();
                return true;
            }
            if strip_offset + 12 < groups.len() && mx >= ox + 336.0 && mx < ox + 348.0 {
                self.legacy_crafting.strip_offset = strip_offset + 1;
                self.legacy_crafting.variant_index = 0;
                self.legacy_crafting.variant_offset = 0;
                self.audio.play_button_click();
                return true;
            }
        }

        // 3. Variant popout top/bottom click on selected group.
        let sx = ox + 12.0 + sel_slot as f32 * 27.0;
        let sy = oy + 38.0;
        let focused_group = groups.get(strip_offset + sel_slot).cloned().unwrap_or_default();
        let len = focused_group.len();
        if !right && len >= 2 && mx >= sx && mx < sx + 27.0 {
            if len >= 3 && my >= sy - 27.0 && my < sy {
                if self.legacy_crafting.variant_offset == -1 {
                    let pos = Self::focused_variant_pos(len, self.legacy_crafting.variant_index, -1);
                    if let Some(&entry_idx) = focused_group.get(pos) {
                        self.execute_craft_entry(entry_idx, self.has_shift_down());
                    }
                } else {
                    self.legacy_crafting.variant_offset = -1;
                    self.audio.play_button_click();
                }
                return true;
            }
            if my >= sy + 27.0 && my < sy + 54.0 {
                if self.legacy_crafting.variant_offset == 1 {
                    let pos = Self::focused_variant_pos(len, self.legacy_crafting.variant_index, 1);
                    if let Some(&entry_idx) = focused_group.get(pos) {
                        self.execute_craft_entry(entry_idx, self.has_shift_down());
                    }
                } else {
                    self.legacy_crafting.variant_offset = 1;
                    self.audio.play_button_click();
                }
                return true;
            }
        }

        // 3b. Horizontal strip scrollbar click.
        let in_popout_frame = len >= 2 && mx >= sx - 4.5 && mx < sx + 31.5;
        if !right
            && !in_popout_frame
            && let Some(next_offset) = legacy_strip_scrollbar_hit(mx, my, ox, oy, groups.len())
        {
            if next_offset != strip_offset {
                self.legacy_crafting.strip_offset = next_offset;
                self.legacy_crafting.variant_index = 0;
                self.legacy_crafting.variant_offset = 0;
                self.audio.play_button_click();
            }
            return true;
        }

        // 4. 12 strip buttons.
        for i in 0..visible_groups {
            let gx = ox + 12.0 + i as f32 * 27.0;
            let gy = oy + 38.0;
            if mx >= gx && mx < gx + 27.0 && my >= gy && my < gy + 27.0 {
                if right && i == sel_slot && len > 1 {
                    self.legacy_crafting.variant_index = (self.legacy_crafting.variant_index + 1) % len;
                    self.legacy_crafting.variant_offset = 0;
                    self.audio.play_button_click();
                    return true;
                }
                if i != sel_slot {
                    self.legacy_crafting.selected_slot = i;
                    self.legacy_crafting.variant_index = 0;
                    self.legacy_crafting.variant_offset = 0;
                    self.audio.play_button_click();
                } else if !right {
                    if self.legacy_crafting.variant_offset != 0 {
                        self.legacy_crafting.variant_offset = 0;
                        self.audio.play_button_click();
                    } else if let Some(&entry_idx) = focused_group.get(self.legacy_crafting.variant_index % len.max(1)) {
                        self.execute_craft_entry(entry_idx, self.has_shift_down());
                    }
                }
                return true;
            }
        }

        // 5. Crafting result slot (`ox + 124.5, oy + 151.0, 36x36`).
        if !right
            && mx >= ox + 124.5
            && mx < ox + 160.5
            && my >= oy + 151.0
            && my < oy + 187.0
            && !focused_group.is_empty()
        {
            let pos = Self::focused_variant_pos(
                len,
                self.legacy_crafting.variant_index,
                self.legacy_crafting.variant_offset,
            );
            if let Some(&entry_idx) = focused_group.get(pos) {
                self.execute_craft_entry(entry_idx, self.has_shift_down());
            }
            return true;
        }

        // 6. Player inventory slots (`186..330, 133..202`) fall through to `menu_click` / `remote_click`.
        if self.slot_at(&batch, mx, my).is_some() {
            return false;
        }
        mx >= ox && mx < ox + 348.0 && my >= oy - 37.0 && my < oy + 215.0
    }

    pub(super) fn scroll_legacy_crafting(&mut self, d: f32) {
        if d == 0.0 {
            return;
        }
        self.ensure_book();
        let groups = self.crafting_tab_groups(self.legacy_crafting.tab);
        let max_offset = groups.len().saturating_sub(12);
        let cur = self.legacy_crafting.strip_offset as isize;
        let step = if d > 0.0 { -1 } else { 1 };
        let next = (cur + step).clamp(0, max_offset as isize) as usize;
        if next != self.legacy_crafting.strip_offset {
            self.legacy_crafting.strip_offset = next;
            self.legacy_crafting.variant_index = 0;
            self.legacy_crafting.variant_offset = 0;
        }
    }

    // =========================================================================
    // LegacyStonecutterScreen (`348x215`, 1:1 `LegacyStonecutterScreen.java`)
    // =========================================================================

    fn stonecutting_groups(&self, counts: &HashMap<String, u32>) -> Vec<(String, Vec<(usize, bool)>)> {
        let recipes = &self.stations().recipes.stonecutting;
        let mut by_input: HashMap<String, Vec<usize>> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for (idx, r) in recipes.iter().enumerate() {
            let key = r
                .input
                .order
                .first()
                .cloned()
                .unwrap_or_else(|| r.id.clone());
            if !by_input.contains_key(&key) {
                order.push(key.clone());
            }
            by_input.entry(key).or_default().push(idx);
        }
        let mut out = Vec::new();
        for key in order {
            let mut idxs = by_input.remove(&key).unwrap_or_default();
            idxs.sort_by_key(|&i| stonecutting_bucket_rank(&recipes[i].result.item));
            let group: Vec<(usize, bool)> = idxs
                .into_iter()
                .map(|i| {
                    let can = recipes[i]
                        .input
                        .order
                        .iter()
                        .any(|item| counts.get(item.as_str()).copied().unwrap_or(0) > 0);
                    (i, can)
                })
                .collect();
            if !group.is_empty() {
                out.push((key, group));
            }
        }
        out
    }

    pub(super) fn draw_legacy_stonecutter(&mut self, batch: &mut HudBatch) -> bool {
        if !self.legacy_stonecutter_open() {
            return false;
        }
        let Some((ox, oy)) = self.legacy_screen_origin(batch) else {
            return false;
        };
        {
            let atlas = &self.gui.atlas;
            batch.fill(
                atlas,
                0.0,
                0.0,
                batch.width,
                batch.height,
                rgba(16, 16, 16, 160),
            );
        }

        self.legacy_panel(batch, ox, oy, 348.0, 215.0);
        let title = self
            .remote_menu_title()
            .unwrap_or_else(|| Text::literal(self.translate("container.stonecutter", "Stonecutter")));
        super::screen_info::name_container(|| title.plain());
        {
            let atlas = &self.gui.atlas;
            let tw = atlas.font.width_text(&title);
            batch.text_styled(
                atlas,
                &title,
                ox + ((348.0 - tw) * 0.5).floor(),
                oy + 17.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }

        let counts = self.stonecutter_available_counts();

        let groups = self.stonecutting_groups(&counts);
        let max_offset = groups.len().saturating_sub(12);
        let strip_offset = self.legacy_crafting.stone_offset.min(max_offset);
        self.legacy_crafting.stone_offset = strip_offset;
        let visible_groups = groups.len().saturating_sub(strip_offset).min(12);
        let sel_slot = if visible_groups > 0 {
            self.legacy_crafting.stone_selected.min(visible_groups - 1)
        } else {
            0
        };
        self.legacy_crafting.stone_selected = sel_slot;

        let focused_group = groups
            .get(strip_offset + sel_slot)
            .map(|(_, g)| g.clone())
            .unwrap_or_default();
        let len = focused_group.len();
        if len > 0 {
            self.legacy_crafting.stone_variant %= len;
            let max_off = if len >= 2 { 1 } else { 0 };
            let min_off = if len >= 3 { -1 } else { 0 };
            self.legacy_crafting.stone_variant_offset =
                self.legacy_crafting.stone_variant_offset.clamp(min_off, max_off);
        } else {
            self.legacy_crafting.stone_variant = 0;
            self.legacy_crafting.stone_variant_offset = 0;
        }
        let cur_var = self.legacy_crafting.stone_variant;
        let var_off = self.legacy_crafting.stone_variant_offset;
        let focused_recipe = focused_group
            .get(Self::focused_variant_pos(len, cur_var, var_off))
            .copied();

        let recipes = &self.stations().recipes.stonecutting;
        let mut hovered_stack: Option<ItemStack> = None;

        for i in 0..12 {
            let sx = ox + 12.0 + i as f32 * 27.0;
            let sy = oy + 38.0;
            self.legacy_slot_box(batch, sx, sy, 27.0);
            let is_focused = i == sel_slot && !focused_group.is_empty();
            if !is_focused
                && let Some((_, group)) = groups.get(strip_offset + i)
                && let Some(&(r_idx, can)) = group.first()
            {
                let res = recipes[r_idx].result.clone();
                self.draw_legacy_item_in_box(batch, &res, sx, sy, 27.0, if can { 1.0 } else { 0.5 });
                if self.cursor.0 >= sx
                    && self.cursor.0 < sx + 27.0
                    && self.cursor.1 >= sy
                    && self.cursor.1 < sy + 27.0
                {
                    hovered_stack = Some(res);
                }
            }
        }

        // Bottom-left stonecutting recess (`9, 103, 163x105`).
        self.legacy_recess(batch, ox + 9.0, oy + 103.0, 163.0, 105.0);
        self.legacy_slot_with(
            batch,
            "legacy/container/arrow",
            ox + 79.0,
            oy + 158.0,
            22.0,
            15.0,
        );
        if let Some((r_idx, can)) = focused_recipe {
            let r = &recipes[r_idx];
            let rname = horizon_client::graphics_style::fit(
                &self.gui.atlas,
                &self.pack.names.display_item(&r.result.item),
                155.0,
            );
            {
                let atlas = &self.gui.atlas;
                let tw = HudBatch::text_width(atlas, &rname);
                batch.text(
                    atlas,
                    &rname,
                    ox + 9.0 + ((163.0 - tw) * 0.5).floor(),
                    oy + 114.0,
                    rgba(64, 64, 64, 255),
                    false,
                );
            }
            let inp_id = r
                .input
                .order
                .iter()
                .find(|id| counts.get(id.as_str()).copied().unwrap_or(0) > 0)
                .or_else(|| r.input.order.first())
                .cloned();
            let ix = ox + 38.0;
            let iy = oy + 146.0;
            self.legacy_icon_holder(batch, ix, iy, 36.0, inp_id.is_some() && !can, false);
            if let Some(id) = inp_id {
                let st = ItemStack::new(&id, 1);
                self.draw_legacy_item_in_box(batch, &st, ix, iy, 36.0, 1.0);
                if !can {
                    self.legacy_warning_icon(batch, ix, iy);
                }
                if self.cursor.0 >= ix
                    && self.cursor.0 < ix + 36.0
                    && self.cursor.1 >= iy
                    && self.cursor.1 < iy + 36.0
                {
                    hovered_stack = Some(st);
                }
            }
            let rx = ox + 110.0;
            let ry = oy + 146.0;
            self.legacy_icon_holder(batch, rx, ry, 36.0, !can, false);
            self.draw_legacy_item_in_box(batch, &r.result, rx, ry, 36.0, 1.0);
            if !can {
                self.legacy_warning_icon(batch, rx, ry);
            }
            if self.cursor.0 >= rx
                && self.cursor.0 < rx + 36.0
                && self.cursor.1 >= ry
                && self.cursor.1 < ry + 36.0
            {
                hovered_stack = Some(r.result.clone());
            }
        } else {
            self.legacy_slot_box(batch, ox + 38.0, oy + 146.0, 36.0);
            self.legacy_slot_box(batch, ox + 110.0, oy + 146.0, 36.0);
        }

        let variants: Vec<(ItemStack, bool)> = focused_group
            .iter()
            .map(|&(r_idx, can)| (self.stations().recipes.stonecutting[r_idx].result.clone(), can))
            .collect();

        // Bottom-right `16x16` player inventory recess (`176, 103, 163x105`).
        self.legacy_recess(batch, ox + 176.0, oy + 103.0, 163.0, 105.0);
        {
            let atlas = &self.gui.atlas;
            let inv_lbl = self.translate("container.inventory", "Inventory");
            let iw = HudBatch::text_width(atlas, &inv_lbl);
            batch.text(
                atlas,
                &inv_lbl,
                ox + 176.0 + ((163.0 - iw) * 0.5).floor(),
                oy + 114.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }
        self.legacy_player_inv_16(batch, ox, oy, 186.0, 133.0, 186.0);
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        let drag = self.drag_view();
        for i in 0..36 {
            let (sx, sy) = self.slot_position(i);
            let stack = self.slot_view(i);
            self.draw_legacy_slot_item(batch, drag.as_ref(), i, &stack, ox + sx, oy + sy);
        }
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        if hovered_stack.is_none()
            && let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1)
        {
            hovered_stack = self.slot_view(slot);
        }

        // Popout variants & overflow indicators.
        self.draw_legacy_strip_scrollbar(batch, ox, oy, strip_offset, groups.len());
        if !variants.is_empty() {
            let sx = ox + 12.0 + sel_slot as f32 * 27.0;
            let sy = oy + 38.0;
            if let Some(pop_tip) = self.draw_variant_popout(batch, sx, sy, &variants, cur_var, var_off)
            {
                hovered_stack = Some(pop_tip);
            }
        }
        self.draw_legacy_strip_chevrons(batch, ox, oy, strip_offset, groups.len());

        self.draw_carried(batch);
        if self.carried.is_none()
            && let Some(stack) = hovered_stack
        {
            self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
        }
        true
    }

    fn execute_stonecut(&mut self, recipe_idx: usize, craft_all: bool) {
        let Some(recipe) = self.stations().recipes.stonecutting.get(recipe_idx).cloned() else {
            return;
        };
        let input_in_slot = self
            .station
            .as_ref()
            .and_then(|s| s.inputs[0].clone())
            .filter(|st| recipe.input.items.contains(&*st.item));

        let chosen_item = if let Some(st) = input_in_slot {
            st.item.to_string()
        } else {
            let Some(inv_slot) = (0..36).find(|&i| {
                self.inventory[i]
                    .as_ref()
                    .is_some_and(|st| recipe.input.items.contains(&*st.item))
            }) else {
                return;
            };
            let item_name = self.inventory[inv_slot]
                .as_ref()
                .map(|s| s.item.to_string())
                .unwrap_or_default();
            if self.station.as_ref().is_some_and(|s| s.inputs[0].is_some()) {
                self.menu_click(Some(STATION_INPUT), 0, MenuInput::QuickMove);
            }
            self.menu_click(Some(inv_slot), 0, MenuInput::QuickMove);
            item_name
        };

        let btn_idx = self
            .stations()
            .recipes
            .stonecutting
            .iter()
            .take(recipe_idx)
            .filter(|r| r.input.items.contains(chosen_item.as_str()))
            .count();

        let mut taken = false;
        let flag = &mut taken;
        self.menu_session(|menu, env, _| {
            *flag = menu.button(env, btn_idx as i32);
            Some(*flag)
        });
        if !taken {
            return;
        }
        self.remote_station_button(btn_idx as i32);
        let reps = if craft_all { 64 } else { 1 };
        let mut crafted_any = false;
        for _ in 0..reps {
            let Some(res) = self.slot_view(STATION_RESULT) else {
                break;
            };
            if !self.can_fit_in_player_inv(&res) {
                break;
            }
            self.menu_click(Some(STATION_RESULT), 0, MenuInput::QuickMove);
            crafted_any = true;
        }
        if crafted_any {
            self.station_sound("minecraft:ui.stonecutter.take_result");
        }
    }

    pub(super) fn click_legacy_stonecutter(&mut self, right: bool) -> bool {
        if !self.legacy_stonecutter_open() {
            return false;
        }
        let Some(batch) = self.gui_scale() else {
            return true;
        };
        let Some((ox, oy)) = self.legacy_screen_origin(&batch) else {
            return true;
        };
        let (mx, my) = self.cursor;
        let counts = self.stonecutter_available_counts();
        let groups = self.stonecutting_groups(&counts);
        let max_offset = groups.len().saturating_sub(12);
        let strip_offset = self.legacy_crafting.stone_offset.min(max_offset);
        let visible_groups = groups.len().saturating_sub(strip_offset).min(12);
        let sel_slot = self
            .legacy_crafting
            .stone_selected
            .min(visible_groups.saturating_sub(1));

        // 1. Strip scroll arrows.
        if !right && my >= oy + 38.0 && my < oy + 65.0 {
            if strip_offset > 0 && mx >= ox && mx < ox + 12.0 {
                self.legacy_crafting.stone_offset = strip_offset - 1;
                self.legacy_crafting.stone_variant = 0;
                self.legacy_crafting.stone_variant_offset = 0;
                self.audio.play_button_click();
                return true;
            }
            if strip_offset + 12 < groups.len() && mx >= ox + 336.0 && mx < ox + 348.0 {
                self.legacy_crafting.stone_offset = strip_offset + 1;
                self.legacy_crafting.stone_variant = 0;
                self.legacy_crafting.stone_variant_offset = 0;
                self.audio.play_button_click();
                return true;
            }
        }

        let focused_group = groups
            .get(strip_offset + sel_slot)
            .map(|(_, g)| g.clone())
            .unwrap_or_default();
        let len = focused_group.len();
        let sx = ox + 12.0 + sel_slot as f32 * 27.0;
        let sy = oy + 38.0;

        // 2. Popout clicks.
        if !right && len >= 2 && mx >= sx && mx < sx + 27.0 {
            if len >= 3 && my >= sy - 27.0 && my < sy {
                if self.legacy_crafting.stone_variant_offset == -1 {
                    let pos = Self::focused_variant_pos(len, self.legacy_crafting.stone_variant, -1);
                    if let Some(&(r_idx, true)) = focused_group.get(pos) {
                        self.execute_stonecut(r_idx, self.has_shift_down());
                    }
                } else {
                    self.legacy_crafting.stone_variant_offset = -1;
                    self.audio.play_button_click();
                }
                return true;
            }
            if my >= sy + 27.0 && my < sy + 54.0 {
                if self.legacy_crafting.stone_variant_offset == 1 {
                    let pos = Self::focused_variant_pos(len, self.legacy_crafting.stone_variant, 1);
                    if let Some(&(r_idx, true)) = focused_group.get(pos) {
                        self.execute_stonecut(r_idx, self.has_shift_down());
                    }
                } else {
                    self.legacy_crafting.stone_variant_offset = 1;
                    self.audio.play_button_click();
                }
                return true;
            }
        }

        // 2b. Horizontal strip scrollbar click.
        let in_popout_frame = len >= 2 && mx >= sx - 4.5 && mx < sx + 31.5;
        if !right
            && !in_popout_frame
            && let Some(next_offset) = legacy_strip_scrollbar_hit(mx, my, ox, oy, groups.len())
        {
            if next_offset != strip_offset {
                self.legacy_crafting.stone_offset = next_offset;
                self.legacy_crafting.stone_variant = 0;
                self.legacy_crafting.stone_variant_offset = 0;
                self.audio.play_button_click();
            }
            return true;
        }

        // 3. 12 strip buttons.
        for i in 0..visible_groups {
            let gx = ox + 12.0 + i as f32 * 27.0;
            let gy = oy + 38.0;
            if mx >= gx && mx < gx + 27.0 && my >= gy && my < gy + 27.0 {
                if right && i == sel_slot && len > 1 {
                    self.legacy_crafting.stone_variant =
                        (self.legacy_crafting.stone_variant + 1) % len;
                    self.legacy_crafting.stone_variant_offset = 0;
                    self.audio.play_button_click();
                    return true;
                }
                if i != sel_slot {
                    self.legacy_crafting.stone_selected = i;
                    self.legacy_crafting.stone_variant = 0;
                    self.legacy_crafting.stone_variant_offset = 0;
                    self.audio.play_button_click();
                } else if !right {
                    if self.legacy_crafting.stone_variant_offset != 0 {
                        self.legacy_crafting.stone_variant_offset = 0;
                        self.audio.play_button_click();
                    } else if let Some(&(r_idx, true)) =
                        focused_group.get(self.legacy_crafting.stone_variant % len.max(1))
                    {
                        self.execute_stonecut(r_idx, self.has_shift_down());
                    }
                }
                return true;
            }
        }

        // 4. Result slot (`ox + 110, oy + 146, 36x36`).
        if !right
            && mx >= ox + 110.0
            && mx < ox + 146.0
            && my >= oy + 146.0
            && my < oy + 182.0
            && !focused_group.is_empty()
        {
            let pos = Self::focused_variant_pos(
                len,
                self.legacy_crafting.stone_variant,
                self.legacy_crafting.stone_variant_offset,
            );
            if let Some(&(r_idx, true)) = focused_group.get(pos) {
                self.execute_stonecut(r_idx, self.has_shift_down());
            }
            return true;
        }

        // 5. Inventory slots fall through.
        if self.slot_at(&batch, mx, my).is_some() {
            return false;
        }
        mx >= ox && mx < ox + 348.0 && my >= oy && my < oy + 215.0
    }

    pub(super) fn scroll_legacy_stonecutter(&mut self, d: f32) {
        if d == 0.0 {
            return;
        }
        let counts = self.stonecutter_available_counts();
        let groups = self.stonecutting_groups(&counts);
        let max_offset = groups.len().saturating_sub(12);
        let cur = self.legacy_crafting.stone_offset as isize;
        let step = if d > 0.0 { -1 } else { 1 };
        let next = (cur + step).clamp(0, max_offset as isize) as usize;
        if next != self.legacy_crafting.stone_offset {
            self.legacy_crafting.stone_offset = next;
            self.legacy_crafting.stone_variant = 0;
            self.legacy_crafting.stone_variant_offset = 0;
        }
    }

    // =========================================================================
    // LegacyMerchantScreen (`294x181`, 1:1 `LegacyMerchantScreen.java`)
    // =========================================================================

    fn merchant_offer_craftable(&self, offer: &MerchantOffer, counts: &HashMap<String, u32>) -> (bool, bool, bool) {
        let a_ok = counts
            .get(&*offer.cost_a(self.items()).item)
            .copied()
            .unwrap_or(0)
            >= offer.cost_a(self.items()).count;
        let b_ok = match offer.cost_b() {
            Some(b) => counts.get(&*b.item).copied().unwrap_or(0) >= b.count,
            None => true,
        };
        (!a_ok, !b_ok, !a_ok || !b_ok)
    }

    fn execute_merchant_trade(&mut self, offer_idx: usize, trade_all: bool) {
        let Some(offer) = self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .and_then(|m| m.session.offers.get(offer_idx))
            .cloned()
        else {
            return;
        };
        if offer.out_of_stock() {
            return;
        }
        let counts = self.merchant_available_counts();
        let (_, _, missing_cost) = self.merchant_offer_craftable(&offer, &counts);
        if missing_cost {
            return;
        }
        let cost_a = offer.cost_a(self.items());
        let cost_b = offer.cost_b();
        let has_station_input = self
            .station
            .as_ref()
            .is_some_and(|s| s.inputs[0].is_some() || s.inputs[1].is_some());
        let frees_inv_slot = !has_station_input
            && self.inventory.iter().take(36).flatten().any(|st| {
                (*st.item == *cost_a.item && st.count <= cost_a.count)
                    || cost_b
                        .as_ref()
                        .is_some_and(|b| *st.item == *b.item && st.count <= b.count)
            });
        if !self.can_fit_in_player_inv(&offer.result) && !frees_inv_slot {
            return;
        }
        let reps = if trade_all && self.remote.is_none() { 64 } else { 1 };
        let mut traded = false;
        for step in 0..reps {
            self.try_move_items(offer_idx);
            if self.remote.is_none() || step > 0 {
                let Some(res) = self.slot_view(STATION_RESULT) else {
                    break;
                };
                if !self.can_fit_in_player_inv(&res) {
                    break;
                }
            }
            self.menu_click(Some(STATION_RESULT), 0, MenuInput::QuickMove);
            traded = true;
        }
        if traded {
            self.audio.play_button_click();
        }
    }

    pub(super) fn draw_legacy_merchant(&mut self, batch: &mut HudBatch) -> bool {
        if !self.legacy_merchant_open() {
            return false;
        }
        let Some((ox, oy)) = self.legacy_screen_origin(batch) else {
            return false;
        };
        let Some(m) = self.station.as_ref().and_then(|s| s.merchant.as_ref()) else {
            return false;
        };
        let (progress, custom, level, xp, offers) = (
            m.progress(),
            m.name.clone(),
            m.session.level,
            m.session.xp,
            m.session.offers.clone(),
        );
        let profession = m
            .session
            .profession
            .trim_start_matches("minecraft:")
            .to_owned();

        {
            let atlas = &self.gui.atlas;
            batch.fill(
                atlas,
                0.0,
                0.0,
                batch.width,
                batch.height,
                rgba(16, 16, 16, 160),
            );
        }

        // Main 294x181 panel + bottom recesses.
        self.legacy_panel(batch, ox, oy, 294.0, 181.0);
        self.legacy_recess(batch, ox + 12.0, oy + 79.0, 110.0, 93.0);
        self.legacy_recess(batch, ox + 126.0, oy + 79.0, 157.0, 93.0);

        // Title (`y = 12`).
        let base_title = custom.unwrap_or_else(|| {
            if profession == "none" {
                self.translate("entity.minecraft.villager", "Villager")
            } else {
                self.translate(
                    &format!("entity.minecraft.villager.{profession}"),
                    &profession,
                )
            }
        });
        let title_str = if progress && level > 0 && level <= 5 {
            let lvl_name = self.translate(&format!("merchant.level.{level}"), &format!("Level {level}"));
            format!("{base_title} - {lvl_name}")
        } else {
            base_title
        };
        super::screen_info::name_container(|| title_str.clone());
        {
            let atlas = &self.gui.atlas;
            let tw = HudBatch::text_width(atlas, &title_str);
            batch.text(
                atlas,
                &title_str,
                ox + ((294.0 - tw) * 0.5).floor(),
                oy + 12.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }

        let counts = self.merchant_available_counts();
        let max_offset = offers.len().saturating_sub(10);
        let offset = self.legacy_crafting.merchant_offset.min(max_offset);
        self.legacy_crafting.merchant_offset = offset;
        let selected = self
            .legacy_crafting
            .merchant_selected
            .min(offers.len().saturating_sub(offset).min(10).saturating_sub(1));
        self.legacy_crafting.merchant_selected = selected;
        let sel_offer = offers.get(offset + selected);

        // Villager XP progress bar (`243x6` at `(25.5, 28)` when `level < 5`).
        if progress && level < 5 {
            let bx = ox + 25.5;
            let by = oy + 28.0;
            let atlas = &self.gui.atlas;
            blit_sprite(
                batch,
                atlas,
                "gui/sprites/container/villager/experience_bar_background",
                bx,
                by,
                243.0,
                6.0,
                u32::MAX,
            );
            let min_xp = villager_level_min_xp(level);
            let max_xp = villager_level_max_xp(level).max(min_xp + 1);
            if xp >= min_xp {
                let frac = 243.0 / (max_xp - min_xp) as f32;
                let filled = ((xp - min_xp) as f32 * frac).floor().clamp(0.0, 243.0);
                if filled > 0.0
                    && let Some(mut r) = atlas
                        .sprites
                        .get("gui/sprites/container/villager/experience_bar_current")
                        .copied()
                {
                    r.u1 = r.u0 + (r.u1 - r.u0) * (filled / 243.0);
                    batch.rect_uv(bx, by, filled, 6.0, r, u32::MAX);
                }
                let offer_xp = sel_offer.map_or(0, |o| o.xp);
                if offer_xp > 0
                    && let Some(mut r) = atlas
                        .sprites
                        .get("legacy/container/experience_bar_result")
                        .copied()
                {
                    let extra = (offer_xp as f32 * frac).floor().clamp(0.0, 243.0 - filled);
                    if extra > 0.0 {
                        r.u0 += (r.u1 - r.u0) * (filled / 243.0);
                        r.u1 = r.u0 + (r.u1 - r.u0) * (extra / (243.0 - filled).max(1.0));
                        batch.rect_uv(bx + filled, by, extra, 6.0, r, u32::MAX);
                    }
                }
            }
        }

        // Scroll arrows (`ox + 5, oy + 52` / `ox + 283, oy + 52`).
        if offset > 0 {
            blit_sprite(
                batch,
                &self.gui.atlas,
                "legacy/widget/scroll_left",
                ox + 5.0,
                oy + 52.0,
                7.0,
                13.0,
                u32::MAX,
            );
        }
        if offset + 10 < offers.len() {
            blit_sprite(
                batch,
                &self.gui.atlas,
                "legacy/widget/scroll_right",
                ox + 283.0,
                oy + 52.0,
                7.0,
                13.0,
                u32::MAX,
            );
        }

        let mut hovered_stack: Option<ItemStack> = None;

        // 10 offer buttons (`ox + 13 + i*27, oy + 44`, `27x27`).
        for i in 0..10 {
            let sx = ox + 13.0 + i as f32 * 27.0;
            let sy = oy + 44.0;
            if let Some(offer) = offers.get(offset + i) {
                let (_, _, warn_all) = self.merchant_offer_craftable(offer, &counts);
                let out_of_stock = offer.out_of_stock();
                self.legacy_icon_holder(batch, sx, sy, 27.0, warn_all && !out_of_stock, false);
                self.draw_legacy_item_in_box(
                    batch,
                    &offer.result,
                    sx,
                    sy,
                    27.0,
                    if out_of_stock { 0.5 } else { 1.0 },
                );
                if out_of_stock {
                    blit_sprite(
                        batch,
                        &self.gui.atlas,
                        "legacy/container/error_cross",
                        sx + 6.0,
                        sy + 6.0,
                        15.0,
                        15.0,
                        u32::MAX,
                    );
                } else if warn_all {
                    self.legacy_warning_icon(batch, sx, sy);
                }
                if i == selected {
                    blit_sprite(
                        batch,
                        &self.gui.atlas,
                        "legacy/container/select_icon_highlight",
                        sx - 4.5,
                        sy - 4.5,
                        36.0,
                        36.0,
                        u32::MAX,
                    );
                }
                if self.cursor.0 >= sx
                    && self.cursor.0 < sx + 27.0
                    && self.cursor.1 >= sy
                    && self.cursor.1 < sy + 27.0
                {
                    hovered_stack = Some(offer.result.clone());
                }
            } else {
                self.legacy_slot_box(batch, sx, sy, 27.0);
            }
        }

        // Left trade recess (`12, 79, 110x93`).
        self.legacy_slot_with(
            batch,
            "legacy/container/arrow",
            ox + 47.0,
            oy + 131.0,
            33.0,
            22.5,
        );
        if let Some(offer) = sel_offer {
            let (warn_a, warn_b, warn_res) = self.merchant_offer_craftable(offer, &counts);
            let cost_a = offer.cost_a(self.items());
            let cost_b = offer.cost_b();
            let ax = ox + 17.0;
            let ay = oy + 79.0 + if cost_b.is_some() { 35.0 } else { 51.0 };
            self.legacy_icon_holder(batch, ax, ay, 27.0, warn_a, false);
            self.draw_legacy_item_in_box(batch, &cost_a, ax, ay, 27.0, 1.0);
            if warn_a {
                self.legacy_warning_icon(batch, ax, ay);
            }
            if self.cursor.0 >= ax
                && self.cursor.0 < ax + 27.0
                && self.cursor.1 >= ay
                && self.cursor.1 < ay + 27.0
            {
                hovered_stack = Some(cost_a);
            }
            if let Some(cb) = cost_b {
                let bx = ox + 17.0;
                let by = oy + 79.0 + 65.0;
                self.legacy_icon_holder(batch, bx, by, 27.0, warn_b, false);
                self.draw_legacy_item_in_box(batch, &cb, bx, by, 27.0, 1.0);
                if warn_b {
                    self.legacy_warning_icon(batch, bx, by);
                }
                if self.cursor.0 >= bx
                    && self.cursor.0 < bx + 27.0
                    && self.cursor.1 >= by
                    && self.cursor.1 < by + 27.0
                {
                    hovered_stack = Some(cb);
                }
            }
            if offer.out_of_stock() {
                blit_sprite(
                    batch,
                    &self.gui.atlas,
                    "legacy/container/error_cross",
                    ox + 52.0,
                    oy + 131.0,
                    22.5,
                    22.5,
                    u32::MAX,
                );
            }
            let rx = ox + 86.0;
            let ry = oy + 130.0;
            self.legacy_icon_holder(batch, rx, ry, 27.0, warn_res, false);
            self.draw_legacy_item_in_box(batch, &offer.result, rx, ry, 27.0, 1.0);
            if warn_res {
                self.legacy_warning_icon(batch, rx, ry);
            }
            if self.cursor.0 >= rx
                && self.cursor.0 < rx + 27.0
                && self.cursor.1 >= ry
                && self.cursor.1 < ry + 27.0
            {
                hovered_stack = Some(offer.result.clone());
            }
        } else {
            self.legacy_slot_box(batch, ox + 17.0, oy + 130.0, 27.0);
            self.legacy_slot_box(batch, ox + 86.0, oy + 130.0, 27.0);
        }

        // Right inventory recess (`126, 79, 157x93`, `16x16` slots at `(133, 98)` & `(133, 154)`).
        {
            let atlas = &self.gui.atlas;
            let inv_lbl = self.translate("container.inventory", "Inventory");
            let iw = HudBatch::text_width(atlas, &inv_lbl);
            batch.text(
                atlas,
                &inv_lbl,
                ox + 126.0 + ((157.0 - iw) * 0.5).floor(),
                oy + 87.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }
        self.legacy_player_inv_16(batch, ox, oy, 133.0, 98.0, 154.0);
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        let drag = self.drag_view();
        for i in 0..36 {
            let (sx, sy) = self.slot_position(i);
            let stack = self.slot_view(i);
            self.draw_legacy_slot_item(batch, drag.as_ref(), i, &stack, ox + sx, oy + sy);
        }
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        if hovered_stack.is_none()
            && let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1)
        {
            hovered_stack = self.slot_view(slot);
        }

        self.draw_carried(batch);
        if self.carried.is_none()
            && let Some(stack) = hovered_stack
        {
            self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
        }
        true
    }

    pub(super) fn click_legacy_merchant(&mut self, right: bool) -> bool {
        if !self.legacy_merchant_open() {
            return false;
        }
        let Some(batch) = self.gui_scale() else {
            return true;
        };
        let Some((ox, oy)) = self.legacy_screen_origin(&batch) else {
            return true;
        };
        let Some(offers) = self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .map(|m| m.session.offers.clone())
        else {
            return false;
        };
        let (mx, my) = self.cursor;
        let max_offset = offers.len().saturating_sub(10);
        let offset = self.legacy_crafting.merchant_offset.min(max_offset);
        let selected = self
            .legacy_crafting
            .merchant_selected
            .min(offers.len().saturating_sub(offset).min(10).saturating_sub(1));

        // 1. Scroll arrows.
        if !right && my >= oy + 44.0 && my < oy + 71.0 {
            if offset > 0 && mx >= ox && mx < ox + 13.0 {
                self.legacy_crafting.merchant_offset = offset - 1;
                self.audio.play_button_click();
                return true;
            }
            if offset + 10 < offers.len() && mx >= ox + 281.0 && mx < ox + 294.0 {
                self.legacy_crafting.merchant_offset = offset + 1;
                self.audio.play_button_click();
                return true;
            }
        }

        // 2. 10 offer buttons.
        for i in 0..10 {
            let sx = ox + 13.0 + i as f32 * 27.0;
            let sy = oy + 44.0;
            if !right
                && mx >= sx
                && mx < sx + 27.0
                && my >= sy
                && my < sy + 27.0
                && offset + i < offers.len()
            {
                if i != selected {
                    self.legacy_crafting.merchant_selected = i;
                    self.try_move_items(offset + i);
                    self.audio.play_button_click();
                } else {
                    self.execute_merchant_trade(offset + i, self.has_shift_down());
                }
                return true;
            }
        }

        // 3. Result slot (`ox + 86, oy + 130, 27x27`).
        if !right
            && mx >= ox + 86.0
            && mx < ox + 113.0
            && my >= oy + 130.0
            && my < oy + 157.0
            && offset + selected < offers.len()
        {
            self.execute_merchant_trade(offset + selected, self.has_shift_down());
            return true;
        }

        // 4. Inventory slots (`133..277, 98..170`) fall through.
        if self.slot_at(&batch, mx, my).is_some() {
            return false;
        }
        mx >= ox && mx < ox + 294.0 && my >= oy && my < oy + 181.0
    }

    pub(super) fn scroll_legacy_merchant(&mut self, d: f32) {
        if d == 0.0 {
            return;
        }
        let len = self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .map_or(0, |m| m.session.offers.len());
        let max_offset = len.saturating_sub(10);
        let cur = self.legacy_crafting.merchant_offset as isize;
        let step = if d > 0.0 { -1 } else { 1 };
        self.legacy_crafting.merchant_offset = (cur + step).clamp(0, max_offset as isize) as usize;
    }

    pub(super) fn key_legacy_crafting(&mut self, event: &KeyEvent) -> bool {
        if self.screen != Screen::Inventory
            || !self.use_legacy_gui()
            || event.state != ElementState::Pressed
        {
            return false;
        }
        let mode = if self.legacy_crafting_open() {
            TabFilter::Crafting
        } else if self.legacy_stonecutter_open() {
            TabFilter::Stonecutter
        } else if self.legacy_merchant_open() {
            TabFilter::Merchant
        } else {
            return false;
        };
        let PhysicalKey::Code(key) = event.physical_key else {
            return false;
        };
        match mode {
            TabFilter::Crafting => {
                self.ensure_book();
                let tabs = raw_crafting_tabs();
                let groups = self.crafting_tab_groups(self.legacy_crafting.tab);
                let visible = groups.len().saturating_sub(self.legacy_crafting.strip_offset).min(12);
                match key {
                    KeyCode::PageUp | KeyCode::BracketLeft if !tabs.is_empty() => {
                        self.legacy_crafting.tab =
                            (self.legacy_crafting.tab + tabs.len() - 1) % tabs.len();
                        self.legacy_crafting.strip_offset = 0;
                        self.legacy_crafting.selected_slot = 0;
                        self.legacy_crafting.variant_index = 0;
                        self.legacy_crafting.variant_offset = 0;
                        self.audio.play_button_click();
                        true
                    }
                    KeyCode::PageDown | KeyCode::BracketRight if !tabs.is_empty() => {
                        self.legacy_crafting.tab = (self.legacy_crafting.tab + 1) % tabs.len();
                        self.legacy_crafting.strip_offset = 0;
                        self.legacy_crafting.selected_slot = 0;
                        self.legacy_crafting.variant_index = 0;
                        self.legacy_crafting.variant_offset = 0;
                        self.audio.play_button_click();
                        true
                    }
                    KeyCode::ArrowLeft if !groups.is_empty() => {
                        if self.legacy_crafting.selected_slot > 0 {
                            self.legacy_crafting.selected_slot -= 1;
                        } else if self.legacy_crafting.strip_offset > 0 {
                            self.legacy_crafting.strip_offset -= 1;
                        } else {
                            self.legacy_crafting.strip_offset = groups.len().saturating_sub(12);
                            self.legacy_crafting.selected_slot = groups
                                .len()
                                .saturating_sub(self.legacy_crafting.strip_offset)
                                .min(12)
                                .saturating_sub(1);
                        }
                        self.legacy_crafting.variant_index = 0;
                        self.legacy_crafting.variant_offset = 0;
                        self.audio.play_button_click();
                        true
                    }
                    KeyCode::ArrowRight if !groups.is_empty() => {
                        if self.legacy_crafting.selected_slot + 1 < visible {
                            self.legacy_crafting.selected_slot += 1;
                        } else if self.legacy_crafting.strip_offset + 12 < groups.len() {
                            self.legacy_crafting.strip_offset += 1;
                        } else {
                            self.legacy_crafting.strip_offset = 0;
                            self.legacy_crafting.selected_slot = 0;
                        }
                        self.legacy_crafting.variant_index = 0;
                        self.legacy_crafting.variant_offset = 0;
                        self.audio.play_button_click();
                        true
                    }
                    KeyCode::ArrowUp | KeyCode::ArrowDown => {
                        let focused_len = groups
                            .get(self.legacy_crafting.strip_offset + self.legacy_crafting.selected_slot)
                            .map_or(0, Vec::len);
                        if focused_len > 1 {
                            let step = if key == KeyCode::ArrowUp { focused_len - 1 } else { 1 };
                            self.legacy_crafting.variant_index =
                                (self.legacy_crafting.variant_index + step) % focused_len;
                            self.legacy_crafting.variant_offset = 0;
                            self.audio.play_button_click();
                        }
                        true
                    }
                    KeyCode::Enter | KeyCode::NumpadEnter => {
                        if let Some(group) = groups
                            .get(self.legacy_crafting.strip_offset + self.legacy_crafting.selected_slot)
                            && !group.is_empty()
                        {
                            let pos = Self::focused_variant_pos(
                                group.len(),
                                self.legacy_crafting.variant_index,
                                self.legacy_crafting.variant_offset,
                            );
                            if let Some(&entry_idx) = group.get(pos) {
                                self.execute_craft_entry(entry_idx, self.has_shift_down());
                            }
                        }
                        true
                    }
                    _ => false,
                }
            }
            TabFilter::Stonecutter => {
                let counts = self.stonecutter_available_counts();
                let groups = self.stonecutting_groups(&counts);
                let visible = groups.len().saturating_sub(self.legacy_crafting.stone_offset).min(12);
                match key {
                    KeyCode::ArrowLeft if !groups.is_empty() => {
                        if self.legacy_crafting.stone_selected > 0 {
                            self.legacy_crafting.stone_selected -= 1;
                        } else if self.legacy_crafting.stone_offset > 0 {
                            self.legacy_crafting.stone_offset -= 1;
                        } else {
                            self.legacy_crafting.stone_offset = groups.len().saturating_sub(12);
                            self.legacy_crafting.stone_selected = groups
                                .len()
                                .saturating_sub(self.legacy_crafting.stone_offset)
                                .min(12)
                                .saturating_sub(1);
                        }
                        self.legacy_crafting.stone_variant = 0;
                        self.legacy_crafting.stone_variant_offset = 0;
                        true
                    }
                    KeyCode::ArrowRight if !groups.is_empty() => {
                        if self.legacy_crafting.stone_selected + 1 < visible {
                            self.legacy_crafting.stone_selected += 1;
                        } else if self.legacy_crafting.stone_offset + 12 < groups.len() {
                            self.legacy_crafting.stone_offset += 1;
                        } else {
                            self.legacy_crafting.stone_offset = 0;
                            self.legacy_crafting.stone_selected = 0;
                        }
                        self.legacy_crafting.stone_variant = 0;
                        self.legacy_crafting.stone_variant_offset = 0;
                        true
                    }
                    KeyCode::ArrowUp | KeyCode::ArrowDown => {
                        let focused_len = groups
                            .get(self.legacy_crafting.stone_offset + self.legacy_crafting.stone_selected)
                            .map_or(0, |(_, g)| g.len());
                        if focused_len > 1 {
                            let step = if key == KeyCode::ArrowUp { focused_len - 1 } else { 1 };
                            self.legacy_crafting.stone_variant =
                                (self.legacy_crafting.stone_variant + step) % focused_len;
                            self.legacy_crafting.stone_variant_offset = 0;
                        }
                        true
                    }
                    KeyCode::Enter | KeyCode::NumpadEnter => {
                        if let Some((_, group)) = groups
                            .get(self.legacy_crafting.stone_offset + self.legacy_crafting.stone_selected)
                            && !group.is_empty()
                        {
                            let pos = Self::focused_variant_pos(
                                group.len(),
                                self.legacy_crafting.stone_variant,
                                self.legacy_crafting.stone_variant_offset,
                            );
                            if let Some(&(r_idx, true)) = group.get(pos) {
                                self.execute_stonecut(r_idx, self.has_shift_down());
                            }
                        }
                        true
                    }
                    _ => false,
                }
            }
            TabFilter::Merchant => {
                let len = self
                    .station
                    .as_ref()
                    .and_then(|s| s.merchant.as_ref())
                    .map_or(0, |m| m.session.offers.len());
                let visible = len.saturating_sub(self.legacy_crafting.merchant_offset).min(10);
                match key {
                    KeyCode::ArrowLeft if len > 0 => {
                        if self.legacy_crafting.merchant_selected > 0 {
                            self.legacy_crafting.merchant_selected -= 1;
                        } else if self.legacy_crafting.merchant_offset > 0 {
                            self.legacy_crafting.merchant_offset -= 1;
                        }
                        true
                    }
                    KeyCode::ArrowRight if len > 0 => {
                        if self.legacy_crafting.merchant_selected + 1 < visible {
                            self.legacy_crafting.merchant_selected += 1;
                        } else if self.legacy_crafting.merchant_offset + 10 < len {
                            self.legacy_crafting.merchant_offset += 1;
                        }
                        true
                    }
                    KeyCode::Enter | KeyCode::NumpadEnter if len > 0 => {
                        let idx = (self.legacy_crafting.merchant_offset + self.legacy_crafting.merchant_selected)
                            .min(len - 1);
                        self.execute_merchant_trade(idx, self.has_shift_down());
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    pub(super) fn pad_legacy_crafting_active(&self) -> bool {
        self.legacy_crafting_open() || self.legacy_stonecutter_open() || self.legacy_merchant_open()
    }

    fn step_paged_strip(
        selected: &mut usize,
        offset: &mut usize,
        len: usize,
        window: usize,
        back: bool,
        wrap: bool,
    ) -> bool {
        if len == 0 {
            return false;
        }
        let visible = len.saturating_sub(*offset).min(window);
        if back {
            if *selected > 0 {
                *selected -= 1;
            } else if *offset > 0 {
                *offset -= 1;
            } else if wrap {
                *offset = len.saturating_sub(window);
                *selected = len.saturating_sub(*offset).min(window).saturating_sub(1);
            } else {
                return false;
            }
        } else if *selected + 1 < visible {
            *selected += 1;
        } else if *offset + window < len {
            *offset += 1;
        } else if wrap {
            *offset = 0;
            *selected = 0;
        } else {
            return false;
        }
        true
    }

    pub(super) fn pad_legacy_crafting_button(
        &mut self,
        button: horizon_client::gamepad::Button,
    ) -> bool {
        use horizon_client::gamepad::Button;
        if self.screen != Screen::Inventory || !self.use_legacy_gui() {
            return false;
        }
        if self.legacy_crafting_open() {
            self.ensure_book();
            let tabs = raw_crafting_tabs();
            let groups = self.crafting_tab_groups(self.legacy_crafting.tab);
            match button {
                Button::LeftBumper if !tabs.is_empty() => {
                    self.legacy_crafting.tab =
                        (self.legacy_crafting.tab + tabs.len() - 1) % tabs.len();
                    self.legacy_crafting.strip_offset = 0;
                    self.legacy_crafting.selected_slot = 0;
                    self.legacy_crafting.variant_index = 0;
                    self.legacy_crafting.variant_offset = 0;
                    self.audio.play_button_click();
                    true
                }
                Button::RightBumper if !tabs.is_empty() => {
                    self.legacy_crafting.tab = (self.legacy_crafting.tab + 1) % tabs.len();
                    self.legacy_crafting.strip_offset = 0;
                    self.legacy_crafting.selected_slot = 0;
                    self.legacy_crafting.variant_index = 0;
                    self.legacy_crafting.variant_offset = 0;
                    self.audio.play_button_click();
                    true
                }
                Button::DPadLeft | Button::DPadRight
                    if Self::step_paged_strip(
                        &mut self.legacy_crafting.selected_slot,
                        &mut self.legacy_crafting.strip_offset,
                        groups.len(),
                        12,
                        button == Button::DPadLeft,
                        true,
                    ) =>
                {
                    self.legacy_crafting.variant_index = 0;
                    self.legacy_crafting.variant_offset = 0;
                    self.audio.play_button_click();
                    true
                }
                Button::DPadLeft | Button::DPadRight => true,
                Button::DPadUp | Button::DPadDown => {
                    let focused_len = groups
                        .get(self.legacy_crafting.strip_offset + self.legacy_crafting.selected_slot)
                        .map_or(0, Vec::len);
                    if focused_len > 1 {
                        let step = if button == Button::DPadUp {
                            focused_len - 1
                        } else {
                            1
                        };
                        self.legacy_crafting.variant_index =
                            (self.legacy_crafting.variant_index + step) % focused_len;
                        self.legacy_crafting.variant_offset = 0;
                        self.audio.play_button_click();
                    }
                    true
                }
                Button::South | Button::North | Button::West => {
                    if let Some(group) = groups
                        .get(self.legacy_crafting.strip_offset + self.legacy_crafting.selected_slot)
                        && !group.is_empty()
                    {
                        let pos = Self::focused_variant_pos(
                            group.len(),
                            self.legacy_crafting.variant_index,
                            self.legacy_crafting.variant_offset,
                        );
                        if let Some(&entry_idx) = group.get(pos) {
                            self.execute_craft_entry(entry_idx, !matches!(button, Button::South));
                        }
                    }
                    true
                }
                _ => false,
            }
        } else if self.legacy_stonecutter_open() {
            let counts = self.stonecutter_available_counts();
            let groups = self.stonecutting_groups(&counts);
            match button {
                Button::DPadLeft | Button::DPadRight
                    if Self::step_paged_strip(
                        &mut self.legacy_crafting.stone_selected,
                        &mut self.legacy_crafting.stone_offset,
                        groups.len(),
                        12,
                        button == Button::DPadLeft,
                        true,
                    ) =>
                {
                    self.legacy_crafting.stone_variant = 0;
                    self.legacy_crafting.stone_variant_offset = 0;
                    self.audio.play_button_click();
                    true
                }
                Button::DPadLeft | Button::DPadRight => true,
                Button::DPadUp | Button::DPadDown => {
                    let focused_len = groups
                        .get(self.legacy_crafting.stone_offset + self.legacy_crafting.stone_selected)
                        .map_or(0, |(_, g)| g.len());
                    if focused_len > 1 {
                        let step = if button == Button::DPadUp {
                            focused_len - 1
                        } else {
                            1
                        };
                        self.legacy_crafting.stone_variant =
                            (self.legacy_crafting.stone_variant + step) % focused_len;
                        self.legacy_crafting.stone_variant_offset = 0;
                        self.audio.play_button_click();
                    }
                    true
                }
                Button::South | Button::North | Button::West => {
                    if let Some((_, group)) = groups
                        .get(self.legacy_crafting.stone_offset + self.legacy_crafting.stone_selected)
                        && !group.is_empty()
                    {
                        let pos = Self::focused_variant_pos(
                            group.len(),
                            self.legacy_crafting.stone_variant,
                            self.legacy_crafting.stone_variant_offset,
                        );
                        if let Some(&(r_idx, true)) = group.get(pos) {
                            self.execute_stonecut(r_idx, !matches!(button, Button::South));
                        }
                    }
                    true
                }
                _ => false,
            }
        } else if self.legacy_merchant_open() {
            let len = self
                .station
                .as_ref()
                .and_then(|s| s.merchant.as_ref())
                .map_or(0, |m| m.session.offers.len());
            match button {
                Button::DPadLeft | Button::DPadUp | Button::DPadRight | Button::DPadDown => {
                    let back = matches!(button, Button::DPadLeft | Button::DPadUp);
                    if Self::step_paged_strip(
                        &mut self.legacy_crafting.merchant_selected,
                        &mut self.legacy_crafting.merchant_offset,
                        len,
                        10,
                        back,
                        false,
                    ) {
                        self.audio.play_button_click();
                    }
                    true
                }
                Button::South | Button::North | Button::West => {
                    if len > 0 {
                        let idx = (self.legacy_crafting.merchant_offset
                            + self.legacy_crafting.merchant_selected)
                            .min(len - 1);
                        self.execute_merchant_trade(idx, !matches!(button, Button::South));
                    }
                    true
                }
                _ => false,
            }
        } else {
            false
        }
    }
}
