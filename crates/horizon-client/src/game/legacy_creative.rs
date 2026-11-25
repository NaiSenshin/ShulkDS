//! 1:1 Legacy4J `CreativeModeScreen` (`321x212`, 8 top tabs per page, 10x5 `27x27` item grid,
//! `LegacyScroller`, 9 `27x27` hotbar slots, and vanilla `creative_tab_listing.json` ordering).

use super::{Game, Screen};
use horizon_client::hud::{HudBatch, rgba};
use horizon_client::items::ItemStack;
use horizon_client::ui::{EditBox, blit_sprite};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::OnceLock;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

const TABS_PER_PAGE: usize = 8;

struct LegacyCreativeTab {
    id: String,
    title_key: String,
    title_fallback: String,
    icon_sprite: String,
    items: Vec<String>,
}

pub(super) struct LegacyCreativeState {
    pub(super) tab: usize,
    pub(super) scroll_rows: [usize; 16],
    pub(super) dragging_scroll: bool,
    pub(super) search: EditBox,
}

impl Default for LegacyCreativeState {
    fn default() -> Self {
        let mut search = EditBox::new(0.0, 0.0, 200.0, 20.0);
        search.max_length = 50;
        search.focused = true;
        search.bordered = true;
        search.text_color = u32::MAX;
        Self {
            tab: 0,
            scroll_rows: [0; 16],
            dragging_scroll: false,
            search,
        }
    }
}

fn fallback_tab_title(id: &str) -> &'static str {
    match id {
        "legacy:building_blocks" => "Building Blocks",
        "legacy:decoration" => "Decorations",
        "legacy:redstone_and_transport" => "Redstone & Transport",
        "legacy:materials" => "Materials",
        "legacy:food" => "Food",
        "legacy:tools_and_armour" => "Tools & Armour",
        "legacy:brewing" => "Brewing",
        "legacy:misc" => "Miscellaneous",
        "legacy:search" => "Search Items",
        _ => "Creative",
    }
}

fn creative_tabs() -> &'static [LegacyCreativeTab] {
    static TABS: OnceLock<Vec<LegacyCreativeTab>> = OnceLock::new();
    TABS.get_or_init(|| {
        let raw = include_str!("../../assets/legacy/creative_tab_listing.json");
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
            let mut items = Vec::new();
            let mut seen = HashSet::new();
            if let Some(listing) = entry["listing"].as_array() {
                for item in listing {
                    let item_id = if let Some(s) = item.as_str() {
                        Some(s)
                    } else if let Some(s) = item["common_item"].as_str() {
                        Some(s)
                    } else {
                        item["item"].as_str()
                    };
                    if let Some(id_str) = item_id
                        && seen.insert(id_str.to_owned())
                    {
                        items.push(id_str.to_owned());
                    }
                }
            }
            out.push(LegacyCreativeTab {
                title_fallback: fallback_tab_title(&id).to_owned(),
                id,
                title_key,
                icon_sprite,
                items,
            });
        }
        out
    })
}

impl Game<'_> {
    fn creative_tab_items(&self, tab_idx: usize) -> Vec<String> {
        let tabs = creative_tabs();
        let Some(tab) = tabs.get(tab_idx) else {
            return Vec::new();
        };
        let registry = self.items();
        if tab.id == "legacy:search" {
            let q = self
                .legacy_creative
                .search
                .value
                .trim()
                .to_ascii_lowercase();
            let mut all = Vec::new();
            let mut seen = HashSet::new();
            for t in tabs {
                for id in &t.items {
                    if registry.get(id.as_str()).is_some() && seen.insert(id.clone()) {
                        all.push(id.clone());
                    }
                }
            }
            for id_arc in &registry.order {
                let id = id_arc.to_string();
                if seen.insert(id.clone()) {
                    all.push(id);
                }
            }
            if !q.is_empty() {
                all.retain(|id| {
                    let name = self.pack.names.display_item(id).to_ascii_lowercase();
                    name.contains(&q) || id.to_ascii_lowercase().contains(&q)
                });
            }
            return all;
        }
        tab.items
            .iter()
            .filter(|id| registry.get(id.as_str()).is_some())
            .cloned()
            .collect()
    }

    pub(super) fn draw_legacy_creative(&mut self, batch: &mut HudBatch) {
        let tabs = creative_tabs();
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

        let tab_idx = self.legacy_creative.tab.min(tabs.len() - 1);
        self.legacy_creative.tab = tab_idx;
        let page = tab_idx / TABS_PER_PAGE;
        let page_start = page * TABS_PER_PAGE;
        let page_end = (page_start + TABS_PER_PAGE).min(tabs.len());
        let selected_local = tab_idx - page_start;
        let tab_count = page_end - page_start;

        let mut hovered_tab: Option<usize> = None;

        // 1. Unselected tabs behind main creative panel (`CreativeModeScreen.java:229-240`).
        for idx in 0..tab_count {
            let tx = ox + idx as f32 * 40.0;
            let ty = oy - 33.0 + 1.4;
            if self.cursor.0 >= tx
                && self.cursor.0 < tx + 40.0
                && self.cursor.1 >= oy - 33.0
                && self.cursor.1 < oy
            {
                hovered_tab = Some(page_start + idx);
            }
            if idx == selected_local {
                continue;
            }
            self.legacy_tab_bg(batch, "legacy/tiles/low_tab", tx, ty, 41.0, 39.0);
            blit_sprite(
                batch,
                &self.gui.atlas,
                &tabs[page_start + idx].icon_sprite,
                tx + (41.0 - 24.0) * 0.5,
                ty + 6.0,
                24.0,
                24.0,
                u32::MAX,
            );
        }

        // 2. Main 321x212 creative panel.
        self.legacy_creative_panel(batch, ox, oy, 321.0, 212.0);

        // 3. Selected tab over panel edge (`high_tab_left / middle / right`).
        {
            let sprite = if selected_local == 0 {
                "legacy/tiles/high_tab_left"
            } else if selected_local == TABS_PER_PAGE - 1 {
                "legacy/tiles/high_tab_right"
            } else {
                "legacy/tiles/high_tab_middle"
            };
            let tx = ox + selected_local as f32 * 40.0;
            let ty = oy - 33.0;
            self.legacy_tab_bg(batch, sprite, tx, ty, 41.0, 39.0);
            blit_sprite(
                batch,
                &self.gui.atlas,
                &tabs[tab_idx].icon_sprite,
                tx + (41.0 - 24.0) * 0.5,
                ty + 5.0,
                24.0,
                24.0,
                u32::MAX,
            );
        }

        // 4. Page navigation arrows (`tabs.len() > 8` -> Search tab sits on page 1).
        if tabs.len() > TABS_PER_PAGE {
            if page > 0 {
                blit_sprite(
                    batch,
                    &self.gui.atlas,
                    "legacy/widget/scroll_left",
                    ox - 10.0,
                    oy - 19.0,
                    7.0,
                    13.0,
                    u32::MAX,
                );
            }
            if page_end < tabs.len() {
                blit_sprite(
                    batch,
                    &self.gui.atlas,
                    "legacy/widget/scroll_right",
                    ox + 324.0,
                    oy - 19.0,
                    7.0,
                    13.0,
                    u32::MAX,
                );
            }
        }

        // 5. Tab title or Search box (`CreativeModeScreen.java:220-225, 289-292`).
        let is_search = tabs[tab_idx].id == "legacy:search";
        let title_str = self.translate(&tabs[tab_idx].title_key, &tabs[tab_idx].title_fallback);
        super::screen_info::begin_container("creative");
        super::screen_info::name_container(|| title_str.clone());
        if is_search {
            self.legacy_creative.search.x = ox + (321.0 - 200.0) * 0.5 - 6.0;
            self.legacy_creative.search.y = oy + 7.0;
            self.legacy_creative.search.width = 200.0;
            self.legacy_creative.search.height = 18.0;
            self.legacy_creative.search.focused = true;
            self.legacy_creative.search.bordered = true;
            self.legacy_creative
                .search
                .draw(batch, &self.gui.atlas, None);
        } else {
            let atlas = &self.gui.atlas;
            let tw = HudBatch::text_width(atlas, &title_str);
            batch.text(
                atlas,
                &title_str,
                ox + ((321.0 - tw) * 0.5).floor(),
                oy + 11.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }

        // 6. 10x5 (`27x27`) creative item grid + scroller.
        let items = self.creative_tab_items(tab_idx);
        let total_rows = items.len().div_ceil(10);
        let max_scroll = total_rows.saturating_sub(5);
        let slot_scroll = self.legacy_creative.scroll_rows[tab_idx].min(max_scroll);
        self.legacy_creative.scroll_rows[tab_idx] = slot_scroll;

        let mut hovered_stack: Option<ItemStack> = None;
        let mut hovered_box: Option<(f32, f32)> = None;

        for i in 0..50 {
            let sx = ox + 20.0 + (i % 10) as f32 * 27.0;
            let sy = oy + 29.0 + (i / 10) as f32 * 27.0;
            self.legacy_slot_box(batch, sx, sy, 27.0);
            if let Some(id) = items.get(slot_scroll * 10 + i) {
                let stack = ItemStack::new(id, 1);
                self.draw_legacy_item_in_box(batch, &stack, sx, sy, 27.0, 1.0);
                if self.cursor.0 >= sx
                    && self.cursor.0 < sx + 27.0
                    && self.cursor.1 >= sy
                    && self.cursor.1 < sy + 27.0
                {
                    hovered_stack = Some(stack);
                    hovered_box = Some((sx, sy));
                }
            } else if self.cursor.0 >= sx
                && self.cursor.0 < sx + 27.0
                && self.cursor.1 >= sy
                && self.cursor.1 < sy + 27.0
            {
                hovered_box = Some((sx, sy));
            }
        }

        self.legacy_scroller(
            batch,
            ox + 296.0,
            oy + 27.0,
            13.0,
            135.0,
            slot_scroll,
            max_scroll,
        );

        // 7. 9-slot `27x27` hotbar (`0..9`).
        for i in 0..9 {
            let sx = ox + 34.0 + i as f32 * 27.0;
            let sy = oy + 176.0;
            self.legacy_slot_box(batch, sx, sy, 27.0);
            if let Some(stack) = self.inventory.get(i).and_then(|s| s.clone()) {
                self.draw_legacy_item_in_box(batch, &stack, sx, sy, 27.0, 1.0);
                if self.cursor.0 >= sx
                    && self.cursor.0 < sx + 27.0
                    && self.cursor.1 >= sy
                    && self.cursor.1 < sy + 27.0
                {
                    hovered_stack = Some(stack);
                    hovered_box = Some((sx, sy));
                }
            } else if self.cursor.0 >= sx
                && self.cursor.0 < sx + 27.0
                && self.cursor.1 >= sy
                && self.cursor.1 < sy + 27.0
            {
                hovered_box = Some((sx, sy));
            }
        }

        if let Some((hx, hy)) = hovered_box {
            blit_sprite(
                batch,
                &self.gui.atlas,
                "legacy/container/slot_highlight",
                hx,
                hy,
                27.0,
                27.0,
                u32::MAX,
            );
        }

        // 8. Carried item + tooltip.
        self.draw_carried(batch);
        if self.carried.is_none() {
            if let Some(stack) = hovered_stack {
                self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
            } else if let Some(ht) = hovered_tab {
                let name = self.translate(&tabs[ht].title_key, &tabs[ht].title_fallback);
                let text = horizon_client::text::Text::literal(name);
                self.draw_tooltip_lines(batch, &[(text, u32::MAX)], self.cursor.0, self.cursor.1);
            }
        }
    }

    pub(super) fn click_legacy_creative(&mut self, batch: &HudBatch, right: bool) {
        let tabs = creative_tabs();
        if tabs.is_empty() {
            return;
        }
        let Some((ox, oy)) = self.legacy_screen_origin(batch) else {
            return;
        };
        let (mx, my) = self.cursor;
        let shift = self.has_shift_down();

        let tab_idx = self.legacy_creative.tab.min(tabs.len() - 1);
        let page = tab_idx / TABS_PER_PAGE;
        let page_start = page * TABS_PER_PAGE;
        let page_end = (page_start + TABS_PER_PAGE).min(tabs.len());
        let tab_count = page_end - page_start;

        // 1. Page arrows.
        if !right && tabs.len() > TABS_PER_PAGE {
            if page > 0 && mx >= ox - 14.0 && mx < ox && my >= oy - 33.0 && my < oy {
                self.dismiss_soft_keyboard();
                self.legacy_creative.tab = (page - 1) * TABS_PER_PAGE;
                self.audio.play_button_click();
                return;
            }
            if page_end < tabs.len()
                && mx >= ox + 321.0
                && mx < ox + 336.0
                && my >= oy - 33.0
                && my < oy
            {
                self.dismiss_soft_keyboard();
                self.legacy_creative.tab = ((page + 1) * TABS_PER_PAGE).min(tabs.len() - 1);
                self.audio.play_button_click();
                return;
            }
        }

        // 2. Top tabs.
        if !right && my >= oy - 33.0 && my < oy {
            for idx in 0..tab_count {
                let tx = ox + idx as f32 * 40.0;
                if mx >= tx && mx < tx + 40.0 {
                    let next_tab = page_start + idx;
                    self.legacy_creative.tab = next_tab;
                    if tabs[next_tab].id == "legacy:search" {
                        self.request_field_soft_keyboard();
                    } else {
                        self.dismiss_soft_keyboard();
                    }
                    self.audio.play_button_click();
                    return;
                }
            }
        }

        // 3. Search box when on Search tab.
        if !right && tabs[tab_idx].id == "legacy:search" {
            let sx = ox + (321.0 - 200.0) * 0.5 - 6.0;
            let sy = oy + 7.0;
            if mx >= sx && mx < sx + 200.0 && my >= sy && my < sy + 18.0 {
                self.legacy_creative.search.x = sx;
                self.legacy_creative.search.y = sy;
                self.legacy_creative.search.width = 200.0;
                self.legacy_creative
                    .search
                    .click(&self.gui.atlas, mx, shift, false);
                self.request_field_soft_keyboard();
                return;
            }
        }
        self.dismiss_soft_keyboard();

        let items = self.creative_tab_items(tab_idx);
        let max_scroll = items.len().div_ceil(10).saturating_sub(5);
        let slot_scroll = self.legacy_creative.scroll_rows[tab_idx].min(max_scroll);

        // 4. Scroller track press.
        if !right && mx >= ox + 294.0 && mx < ox + 312.0 && my >= oy + 16.0 && my < oy + 175.0 {
            self.legacy_creative.dragging_scroll = true;
            if max_scroll > 0 {
                let frac = ((my - (oy + 27.0) - 7.0) / (135.0 - 15.0)).clamp(0.0, 1.0);
                self.legacy_creative.scroll_rows[tab_idx] =
                    (frac * max_scroll as f32).round() as usize;
            }
            return;
        }

        // 5. 50-slot item grid (`20 + c*27, 29 + r*27, 27x27`).
        let carried_cap = self.carried.as_ref().map_or(64, |c| {
            self.items()
                .get(&c.item)
                .map_or(64, |it| it.max_stack)
                .max(1)
        });
        for i in 0..50 {
            let sx = ox + 20.0 + (i % 10) as f32 * 27.0;
            let sy = oy + 29.0 + (i / 10) as f32 * 27.0;
            if mx >= sx && mx < sx + 27.0 && my >= sy && my < sy + 27.0 {
                if let Some(carried) = &mut self.carried {
                    if let Some(id) = items.get(slot_scroll * 10 + i)
                        && carried.item.as_ref() == id.as_str()
                        && !right
                    {
                        if carried.count < carried_cap {
                            carried.count = if shift {
                                carried_cap
                            } else {
                                carried.count + 1
                            };
                        }
                    } else if right && carried.count > 1 {
                        carried.count -= 1;
                    } else {
                        self.carried = None;
                    }
                    return;
                }
                if let Some(id) = items.get(slot_scroll * 10 + i) {
                    let cap = self.items().get(id).map_or(64, |it| it.max_stack).max(1);
                    if shift {
                        let stack = ItemStack::new(id, cap);
                        if let Some(dest) = self.inventory[0..9]
                            .iter()
                            .position(|s| {
                                s.as_ref()
                                    .is_some_and(|e| e.item == stack.item && e.count < cap)
                            })
                            .or_else(|| self.inventory[0..9].iter().position(Option::is_none))
                        {
                            self.inventory[dest] = Some(stack);
                            self.send_creative_slots();
                        }
                    } else {
                        self.carried = Some(ItemStack::new(id, if right { cap } else { 1 }));
                    }
                }
                return;
            }
        }

        // 6. Hotbar slots (`0..9`).
        for i in 0..9 {
            let sx = ox + 34.0 + i as f32 * 27.0;
            let sy = oy + 176.0;
            if mx >= sx && mx < sx + 27.0 && my >= sy && my < sy + 27.0 {
                if shift {
                    self.inventory[i] = None;
                    self.send_creative_slots();
                    return;
                }
                match (self.carried.take(), self.inventory[i].take()) {
                    (None, None) => {}
                    (None, Some(mut slot_stack)) => {
                        if right && slot_stack.count > 1 {
                            let half = slot_stack.count.div_ceil(2);
                            let mut hand = slot_stack.clone();
                            hand.count = half;
                            slot_stack.count -= half;
                            self.inventory[i] = Some(slot_stack);
                            self.carried = Some(hand);
                        } else {
                            self.carried = Some(slot_stack);
                        }
                    }
                    (Some(mut hand), None) => {
                        if right && hand.count > 1 {
                            let mut one = hand.clone();
                            one.count = 1;
                            hand.count -= 1;
                            self.inventory[i] = Some(one);
                            self.carried = Some(hand);
                        } else {
                            self.inventory[i] = Some(hand);
                        }
                    }
                    (Some(mut hand), Some(mut slot_stack)) => {
                        let cap = self
                            .items()
                            .get(&slot_stack.item)
                            .map_or(64, |it| it.max_stack)
                            .max(1);
                        if hand.same_item_same_components(&slot_stack) {
                            let add = if right { 1 } else { hand.count }
                                .min(cap.saturating_sub(slot_stack.count));
                            slot_stack.count += add;
                            hand.count -= add;
                            self.inventory[i] = Some(slot_stack);
                            if hand.count > 0 {
                                self.carried = Some(hand);
                            }
                        } else {
                            self.inventory[i] = Some(hand);
                            self.carried = Some(slot_stack);
                        }
                    }
                }
                self.send_creative_slots();
                return;
            }
        }

        // 7. Click outside creative panel clears carried stack.
        if mx < ox || mx >= ox + 321.0 || my < oy - 33.0 || my >= oy + 212.0 {
            self.carried = None;
        }
    }

    pub(super) fn drag_legacy_creative(&mut self) {
        if !self.legacy_creative.dragging_scroll {
            return;
        }
        let Some(batch) = self.gui_scale() else {
            return;
        };
        let Some((_, oy)) = self.legacy_screen_origin(&batch) else {
            return;
        };
        let tab_idx = self.legacy_creative.tab;
        let items = self.creative_tab_items(tab_idx);
        let max_scroll = items.len().div_ceil(10).saturating_sub(5);
        if max_scroll > 0 {
            let frac = ((self.cursor.1 - (oy + 27.0) - 7.0) / (135.0 - 15.0)).clamp(0.0, 1.0);
            self.legacy_creative.scroll_rows[tab_idx] = (frac * max_scroll as f32).round() as usize;
        }
    }

    pub(super) fn release_legacy_creative(&mut self) {
        self.legacy_creative.dragging_scroll = false;
    }

    pub(super) fn scroll_legacy_creative(&mut self, d: f32) {
        let tabs = creative_tabs();
        if tabs.is_empty() || d == 0.0 {
            return;
        }
        let tab_idx = self.legacy_creative.tab.min(tabs.len() - 1);
        let max_scroll = self
            .creative_tab_items(tab_idx)
            .len()
            .div_ceil(10)
            .saturating_sub(5);
        let cur = self.legacy_creative.scroll_rows[tab_idx] as isize;
        let step = if d > 0.0 { -1 } else { 1 };
        self.legacy_creative.scroll_rows[tab_idx] =
            (cur + step).clamp(0, max_scroll as isize) as usize;
    }

    pub(super) fn key_legacy_creative(&mut self, event: &KeyEvent) -> bool {
        if self.screen != Screen::Creative
            || !self.use_legacy_creative()
            || event.state != ElementState::Pressed
        {
            return false;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return false;
        };
        if key == KeyCode::Escape {
            return false;
        }
        let tabs = creative_tabs();
        if tabs.is_empty() {
            return false;
        }
        let tab_idx = self.legacy_creative.tab.min(tabs.len() - 1);
        let on_search = tabs[tab_idx].id == "legacy:search";

        match key {
            KeyCode::PageUp | KeyCode::BracketLeft if !on_search => {
                self.legacy_creative.tab = (tab_idx + tabs.len() - 1) % tabs.len();
                self.audio.play_button_click();
                return true;
            }
            KeyCode::PageDown | KeyCode::BracketRight if !on_search => {
                self.legacy_creative.tab = (tab_idx + 1) % tabs.len();
                self.audio.play_button_click();
                return true;
            }
            KeyCode::KeyX if !on_search => {
                self.inventory[0..9].fill(None);
                self.send_creative_slots();
                return true;
            }
            _ => {}
        }

        if !on_search {
            return false;
        }

        if let Some(shortcut) = self.edit_shortcut(event) {
            if self
                .legacy_creative
                .search
                .shortcut(shortcut, &mut self.clipboard)
            {
                self.legacy_creative.scroll_rows[tab_idx] = 0;
            }
            return true;
        }
        let before = self.legacy_creative.search.value.clone();
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        if self.legacy_creative.search.key(key, ctrl, shift) {
            if self.legacy_creative.search.value != before {
                self.legacy_creative.scroll_rows[tab_idx] = 0;
            }
            return true;
        }
        if let Some(text) = event.text.as_deref()
            && !text.chars().all(char::is_control)
        {
            self.legacy_creative.search.insert_text(text);
            self.legacy_creative.scroll_rows[tab_idx] = 0;
            return true;
        }
        true
    }

    pub(super) fn legacy_creative_search_active(&self) -> bool {
        if self.screen != Screen::Creative || !self.use_legacy_creative() {
            return false;
        }
        let tabs = creative_tabs();
        !tabs.is_empty() && tabs[self.legacy_creative.tab.min(tabs.len() - 1)].id == "legacy:search"
    }

    pub(super) fn legacy_creative_search_rect(&self, batch: &HudBatch) -> Option<[f32; 4]> {
        if !self.legacy_creative_search_active() {
            return None;
        }
        let (ox, oy) = self.legacy_screen_origin(batch)?;
        Some([ox + (321.0 - 200.0) * 0.5 - 6.0, oy + 7.0, 200.0, 18.0])
    }

    pub(super) fn legacy_creative_cycle_tab(&mut self, next: bool) {
        let count = creative_tabs().len();
        if count == 0 {
            return;
        }
        self.dismiss_soft_keyboard();
        let cur = self.legacy_creative.tab.min(count - 1);
        self.legacy_creative.tab = if next {
            (cur + 1) % count
        } else {
            (cur + count - 1) % count
        };
        self.inv_drag = None;
        self.last_inv_click = None;
        self.audio.play_button_click();
    }

    pub(super) fn legacy_creative_first_slot_rect(&self, batch: &HudBatch) -> Option<[f32; 4]> {
        let (ox, oy) = self.legacy_screen_origin(batch)?;
        Some([ox + 20.0, oy + 29.0, 27.0, 27.0])
    }

    pub(super) fn legacy_creative_targets(
        &self,
        batch: &HudBatch,
    ) -> Vec<horizon_client::widgets::Target> {
        use horizon_client::widgets::{Kind, Target};
        let tabs = creative_tabs();
        if tabs.is_empty() {
            return Vec::new();
        }
        let Some((ox, oy)) = self.legacy_screen_origin(batch) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(68);
        for r in 0..5 {
            for c in 0..10 {
                out.push(Target {
                    rect: [
                        ox + 20.0 + c as f32 * 27.0,
                        oy + 29.0 + r as f32 * 27.0,
                        27.0,
                        27.0,
                    ],
                    kind: Kind::Slot,
                });
            }
        }
        for c in 0..9 {
            out.push(Target {
                rect: [ox + 34.0 + c as f32 * 27.0, oy + 176.0, 27.0, 27.0],
                kind: Kind::Slot,
            });
        }
        let tab_idx = self.legacy_creative.tab.min(tabs.len() - 1);
        if tabs[tab_idx].id == "legacy:search" {
            out.push(Target {
                rect: [ox + (321.0 - 200.0) * 0.5 - 6.0, oy + 7.0, 200.0, 18.0],
                kind: Kind::Field,
            });
        }
        let page = tab_idx / TABS_PER_PAGE;
        let page_start = page * TABS_PER_PAGE;
        let page_end = (page_start + TABS_PER_PAGE).min(tabs.len());
        let selected_local = tab_idx - page_start;
        for idx in 0..(page_end - page_start) {
            out.push(Target {
                rect: [ox + idx as f32 * 40.0, oy - 33.0, 40.0, 33.0],
                kind: Kind::Tab(idx == selected_local),
            });
        }
        out
    }

    pub(super) fn legacy_creative_pad_scroll_step(
        &mut self,
        button: horizon_client::gamepad::Button,
    ) -> bool {
        use horizon_client::gamepad::Button;
        if self.screen != Screen::Creative || !self.use_legacy_creative() {
            return false;
        }
        if !matches!(button, Button::DPadUp | Button::DPadDown) {
            return false;
        }
        let Some(batch) = self.gui_scale() else {
            return false;
        };
        let Some((ox, oy)) = self.legacy_screen_origin(&batch) else {
            return false;
        };
        let (on_top_row, on_bottom_row) = if let Some(focus) = self.pad.menu.focus {
            (
                (focus.rect[1] - (oy + 29.0)).abs() < 2.0,
                (focus.rect[1] - (oy + 29.0 + 4.0 * 27.0)).abs() < 2.0,
            )
        } else if self.pad.menu.pos.is_some() {
            let (cx, cy) = self.cursor;
            let in_grid_x = cx >= ox + 20.0 && cx < ox + 290.0;
            (
                in_grid_x && cy >= oy + 29.0 && cy < oy + 56.0,
                in_grid_x && cy >= oy + 29.0 + 4.0 * 27.0 && cy < oy + 29.0 + 5.0 * 27.0,
            )
        } else {
            return false;
        };
        let tabs = creative_tabs();
        if tabs.is_empty() {
            return false;
        }
        let tab_idx = self.legacy_creative.tab.min(tabs.len() - 1);
        let max_scroll = self
            .creative_tab_items(tab_idx)
            .len()
            .div_ceil(10)
            .saturating_sub(5);
        if max_scroll == 0 {
            return false;
        }
        let scroll = self.legacy_creative.scroll_rows[tab_idx].min(max_scroll);
        if button == Button::DPadUp && on_top_row && scroll > 0 {
            self.legacy_creative.scroll_rows[tab_idx] = scroll - 1;
            self.audio.play_button_click();
            return true;
        }
        if button == Button::DPadDown && on_bottom_row && scroll < max_scroll {
            self.legacy_creative.scroll_rows[tab_idx] = scroll + 1;
            self.audio.play_button_click();
            return true;
        }
        false
    }
}
