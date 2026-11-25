//! ShulkDS: Dual-Screen 3DS-Inspired Modern Main Menu & Title Sub-Pages.
//!
//! Splits title navigation across both displays cleanly:
//! - **Top Screen:** Atmospheric panorama with crisp Minecraft + ShulkDS branding,
//!   live animated 3D player avatar, or rich Hero Showcase cards previewing the selected
//!   world, live world-creation blueprint, or live multiplayer server status.
//! - **Bottom Screen:** Touch-first modern dark-slate 3DS hub with large Hero cards,
//!   drag-scrollable World/Server lists, interactive Create World builder with Android IME
//!   text input, and an Options Category Quick-Jump companion.

use super::{Game, Screen, event_loop::EventLoopCtl};
use horizon_client::{
    Result, accounts,
    dual_screen::{BottomPointerAction, PointerButton, ViewportRect},
    gamepad::Button,
    hud::{HudBatch, rgba},
    lang,
    multiplayer::{Outcome as MpOutcome, Screen as MpScreen},
    options_ui::Page,
    save::WorldPreset,
    title::{Action, DIFFICULTIES, Mode, WorldEntry},
    ui,
};
use winit::keyboard::KeyCode;

const CARD_BG: u32 = u32::from_le_bytes([24, 24, 26, 224]);
const CARD_SEL: u32 = u32::from_le_bytes([52, 52, 56, 232]);
const BORDER_DARK: u32 = u32::from_le_bytes([10, 10, 12, 255]);
const BORDER_SOFT: u32 = u32::from_le_bytes([86, 86, 92, 255]);
const BORDER_GLOW: u32 = u32::from_le_bytes([255, 255, 255, 240]);
const TEXT_WHITE: u32 = u32::from_le_bytes([255, 255, 255, 255]);
const TEXT_MUTED: u32 = u32::from_le_bytes([188, 192, 204, 255]);
const ACCENT_GREEN: u32 = u32::from_le_bytes([74, 204, 120, 255]);
const ACCENT_BLUE: u32 = u32::from_le_bytes([88, 168, 250, 255]);
const ACCENT_AMBER: u32 = u32::from_le_bytes([242, 178, 74, 255]);
const ACCENT_SILVER: u32 = u32::from_le_bytes([154, 168, 192, 255]);
const ACCENT_RED: u32 = u32::from_le_bytes([224, 86, 86, 255]);
const ACCENT_VIOLET: u32 = u32::from_le_bytes([178, 124, 240, 255]);

pub(super) struct ShulkdsUpdateView {
    pub version: String,
    pub size_mb: String,
    pub highlights: Vec<String>,
    pub stage_title: String,
    pub body: String,
    pub progress: Option<f32>,
    pub buttons: Vec<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitAct {
    MainSingleplayer,
    MainMultiplayer,
    MainFriends,
    MainOptions,
    MainLanguage,
    MainAccess,
    MainCredits,
    MainQuit,
    MainProfile,
    BackToMain,
    SelectRow(usize),
    SelectPlay,
    SelectCreate,
    SelectEdit,
    SelectCopy,
    SelectDelete,
    CreateTab(usize),
    CreateFocusField(u8),
    CreateMode(bool, bool),
    CreateDifficulty(usize),
    CreateButton(u8),
    MpRow(usize),
    MpButton(u8),
    WarningCheck,
    WarningProceed,
    WarningBack,
    OptionsPage(Page),
    OptionsDone,
    ModalBack,
    UpdateAction(u8),
}

#[derive(Clone)]
struct HitItem {
    rect: [f32; 4],
    act: HitAct,
    enabled: bool,
    focusable: bool,
}

pub(super) struct ShulkdsTitleState {
    pub(super) pending_action: Option<Action>,
    world_scroll: f32,
    server_scroll: f32,
    max_scroll: f32,
    scroll_rect: Option<[f32; 4]>,
    drag_start_y: Option<f32>,
    drag_last_y: f32,
    drag_moved: bool,
    pointer_pos: Option<(f32, f32)>,
    pub(super) pointer_down: bool,
    focus_idx: usize,
    last_mode_tag: u8,
    pending_row_focus: Option<usize>,
    hits: Vec<HitItem>,
}

impl ShulkdsTitleState {
    pub(super) fn hud_signature(&self) -> u64 {
        let mut h: u64 = self.focus_idx as u64 ^ (u64::from(self.last_mode_tag) << 32);
        h = h.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ u64::from(self.world_scroll.to_bits());
        h = h.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ u64::from(self.server_scroll.to_bits());
        h ^ (u64::from(self.pointer_down) << 63)
    }
}

impl Default for ShulkdsTitleState {
    fn default() -> Self {
        Self {
            pending_action: None,
            world_scroll: 0.0,
            server_scroll: 0.0,
            max_scroll: 0.0,
            scroll_rect: None,
            drag_start_y: None,
            drag_last_y: 0.0,
            drag_moved: false,
            pointer_pos: None,
            pointer_down: false,
            focus_idx: 1,
            last_mode_tag: u8::MAX,
            pending_row_focus: None,
            hits: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomTitleKind {
    Main,
    Select,
    Create,
    MultiplayerList,
    MultiplayerWarning,
    Options,
    DetailModal,
}

impl Game<'_> {
    #[inline]
    pub(super) fn shulkds_title_active(&self) -> bool {
        self.shulkds.config.enabled && self.title.is_some()
    }

    /// True when both screens render custom 3DS-inspired dual-screen title UI.
    pub(super) fn shulkds_title_split_mode(&self) -> bool {
        if !self.shulkds_title_active() || matches!(self.screen, Screen::Options | Screen::Win) {
            return false;
        }
        let Some(title) = self.title.as_ref() else {
            return false;
        };
        match &title.mode {
            Mode::Main | Mode::Select { .. } | Mode::MultiplayerWarning { .. } => true,
            Mode::Create { .. } => !title.has_create_modal(),
            Mode::Multiplayer(MpScreen::List { .. }) => true,
            _ => false,
        }
    }

    fn current_title_kind(&self) -> Option<BottomTitleKind> {
        if !self.shulkds_title_active() {
            return None;
        }
        if self.screen == Screen::Options {
            return Some(BottomTitleKind::Options);
        }
        let title = self.title.as_ref()?;
        Some(match &title.mode {
            Mode::Main => BottomTitleKind::Main,
            Mode::Select { .. } => BottomTitleKind::Select,
            Mode::Create { .. } if !title.has_create_modal() => BottomTitleKind::Create,
            Mode::Multiplayer(MpScreen::List { .. }) => BottomTitleKind::MultiplayerList,
            Mode::MultiplayerWarning { .. } => BottomTitleKind::MultiplayerWarning,
            _ => BottomTitleKind::DetailModal,
        })
    }

    fn sync_title_mode_tag(&mut self, tag: u8) {
        if self.shulkds_title.last_mode_tag != tag {
            self.dismiss_soft_keyboard();
            self.shulkds_title.last_mode_tag = tag;
            self.shulkds_title.pending_row_focus = None;
            self.shulkds_title.drag_start_y = None;
            self.shulkds_title.drag_moved = false;
            self.shulkds_title.pointer_down = false;
            self.shulkds.dashboard.pointer_down = false;
            match tag {
                0 => {
                    self.shulkds_title.focus_idx = 1;
                }
                1 => {
                    if let Some(title) = self.title.as_mut()
                        && let Mode::Select { worlds, selected } = &mut title.mode
                        && !worlds.is_empty()
                    {
                        let row = selected.unwrap_or(0).min(worlds.len() - 1);
                        *selected = Some(row);
                        self.shulkds_title.world_scroll = if row == 0 {
                            0.0
                        } else {
                            ((row as f32) * 34.0 - 34.0).max(0.0)
                        };
                        self.shulkds_title.focus_idx = 2;
                        self.shulkds_title.pending_row_focus = Some(row);
                    } else {
                        self.shulkds_title.world_scroll = 0.0;
                        self.shulkds_title.focus_idx = 1;
                    }
                }
                3 => {
                    if let Some(title) = self.title.as_mut()
                        && let Mode::Multiplayer(screen) = &mut title.mode
                    {
                        let count = screen.list_summaries().len();
                        if count > 0 {
                            let row = screen.selected_row().unwrap_or(0).min(count - 1);
                            screen.set_selected_row(Some(row));
                            self.shulkds_title.server_scroll = if row == 0 {
                                0.0
                            } else {
                                ((row as f32) * 32.0 - 32.0).max(0.0)
                            };
                            self.shulkds_title.focus_idx = 3;
                            self.shulkds_title.pending_row_focus = Some(row);
                        } else {
                            self.shulkds_title.server_scroll = 0.0;
                            self.shulkds_title.focus_idx = 2;
                        }
                    } else {
                        self.shulkds_title.server_scroll = 0.0;
                        self.shulkds_title.focus_idx = 2;
                    }
                }
                4 => {
                    self.shulkds_title.focus_idx = 1;
                }
                _ => {
                    self.shulkds_title.focus_idx = 0;
                }
            }
        }
    }

    fn clipped_list_hit_rect(r: [f32; 4], bx: f32, by: f32, list_y: f32, list_h: f32) -> [f32; 4] {
        let hit_top = r[1].max(list_y);
        let hit_bot = (r[1] + r[3]).min(list_y + list_h);
        [r[0] - bx, hit_top - by, r[2], (hit_bot - hit_top).max(1.0)]
    }

    fn draw_title_card(
        &self,
        batch: &mut HudBatch,
        [x, y, w, h]: [f32; 4],
        bg: u32,
        border: u32,
        top_accent: Option<u32>,
    ) {
        let a = &self.gui.atlas;
        batch.fill(a, x, y, w, h, border);
        batch.fill(
            a,
            x + 1.0,
            y + 1.0,
            (w - 2.0).max(0.0),
            (h - 2.0).max(0.0),
            bg,
        );
        batch.fill(a, x + 1.0, y + 1.0, (w - 2.0).max(0.0), 1.0, BORDER_SOFT);
        if let Some(col) = top_accent {
            batch.fill(a, x + 1.0, y + 1.0, (w - 2.0).max(0.0), 2.5, col);
        }
    }

    pub(super) fn draw_stone_panel(&self, batch: &mut HudBatch, [x, y, w, h]: [f32; 4]) {
        let a = &self.gui.atlas;
        batch.fill(a, x, y, w, h, rgba(12, 12, 14, 255));
        let iw = (w - 2.0).max(0.0);
        let ih = (h - 2.0).max(0.0);
        batch.fill(a, x + 1.0, y + 1.0, iw, ih, rgba(28, 28, 30, 232));
        batch.fill(a, x + 1.0, y + 1.0, iw, 1.0, rgba(78, 78, 82, 255));
        batch.fill(a, x + 1.0, y + 1.0, 1.0, ih, rgba(78, 78, 82, 255));
        if h >= 3.0 {
            batch.fill(a, x + 1.0, y + h - 2.0, iw, 1.0, rgba(18, 18, 20, 255));
        }
        if w >= 3.0 {
            batch.fill(a, x + w - 2.0, y + 1.0, 1.0, ih, rgba(18, 18, 20, 255));
        }
    }

    fn draw_text_field_box(&self, batch: &mut HudBatch, [x, y, w, h]: [f32; 4], focused: bool) {
        let a = &self.gui.atlas;
        let sprite = if focused {
            "gui/sprites/widget/text_field_highlighted"
        } else {
            "gui/sprites/widget/text_field"
        };
        if a.sprites.contains_key(sprite) {
            ui::nine_slice(batch, a, sprite, x, y, w, h, u32::MAX);
        } else {
            let border = if focused {
                TEXT_WHITE
            } else {
                rgba(160, 160, 160, 255)
            };
            batch.fill(a, x, y, w, h, border);
            batch.fill(
                a,
                x + 1.0,
                y + 1.0,
                (w - 2.0).max(0.0),
                (h - 2.0).max(0.0),
                rgba(0, 0, 0, 255),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_official_button(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        enabled: bool,
        focused: bool,
        hovered: bool,
        active: bool,
        pressed: bool,
    ) {
        let sprite = if !enabled {
            "gui/sprites/widget/button_disabled"
        } else if focused || hovered || active {
            "gui/sprites/widget/button_highlighted"
        } else {
            "gui/sprites/widget/button"
        };
        let a = &self.gui.atlas;
        if a.sprites.contains_key(sprite) {
            let tint = if pressed {
                rgba(210, 210, 210, 255)
            } else {
                u32::MAX
            };
            ui::nine_slice(batch, a, sprite, rect[0], rect[1], rect[2], rect[3], tint);
            if focused && !hovered {
                batch.fill(a, rect[0], rect[1], rect[2], 1.0, BORDER_GLOW);
                batch.fill(
                    a,
                    rect[0],
                    rect[1] + rect[3] - 1.0,
                    rect[2],
                    1.0,
                    BORDER_GLOW,
                );
            }
        } else {
            let bg = if !enabled {
                rgba(22, 24, 30, 195)
            } else if pressed {
                rgba(48, 56, 70, 245)
            } else if active || focused || hovered {
                CARD_SEL
            } else {
                CARD_BG
            };
            let bd = if !enabled {
                BORDER_DARK
            } else if focused || hovered || active {
                BORDER_GLOW
            } else {
                BORDER_DARK
            };
            self.draw_title_card(batch, rect, bg, bd, None);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_title_pill_btn(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        rect: [f32; 4],
        label: &str,
        _accent: Option<u32>,
        enabled: bool,
        active: bool,
        act: HitAct,
    ) {
        let focus_ordinal = self
            .shulkds_title
            .hits
            .iter()
            .filter(|h| h.focusable)
            .count();
        let focused = self.shulkds_title.focus_idx == focus_ordinal && enabled;
        self.shulkds_title.hits.push(HitItem {
            rect: [rect[0] - bx, rect[1] - by, rect[2], rect[3]],
            act,
            enabled,
            focusable: true,
        });
        let hovered =
            enabled && self.hit_under_pointer([rect[0] - bx, rect[1] - by, rect[2], rect[3]]);
        let pressed = hovered && self.shulkds_title.pointer_down;
        self.draw_official_button(batch, rect, enabled, focused, hovered, active, pressed);
        let a = &self.gui.atlas;
        let tw = HudBatch::text_width(a, label).max(1.0);
        let scale = ((rect[2] - 8.0).max(8.0) / tw)
            .min((rect[3] - 4.0).max(6.0) / 8.0)
            .clamp(0.55, 1.0);
        let fg = if !enabled {
            rgba(160, 160, 160, 255)
        } else if active || focused || hovered {
            rgba(255, 255, 160, 255)
        } else {
            TEXT_WHITE
        };
        let tx = rect[0] + (rect[2] - tw * scale) * 0.5;
        let ty = rect[1] + (rect[3] - 8.0 * scale) * 0.5 + if pressed { 1.0 } else { 0.0 };
        batch.text_scaled(a, label, tx, ty, scale, fg, true);
    }

    fn hit_under_pointer(&self, local_rect: [f32; 4]) -> bool {
        let Some((px, py)) = self.shulkds_title.pointer_pos else {
            return false;
        };
        px >= local_rect[0]
            && px <= local_rect[0] + local_rect[2]
            && py >= local_rect[1]
            && py <= local_rect[1] + local_rect[3]
    }

    fn draw_title_top_vignette(&self, batch: &mut HudBatch, w: f32, h: f32, show_top_bar: bool) {
        let a = &self.gui.atlas;
        if show_top_bar {
            batch.fill(a, 0.0, 0.0, w, 15.0, rgba(10, 10, 12, 165));
        }
        batch.fill(a, 0.0, h - 14.0, w, 14.0, rgba(10, 10, 12, 165));
    }

    fn fit_text_width(&self, text: &str, max_w: f32, max_scale: f32) -> (String, f32) {
        let a = &self.gui.atlas;
        let raw = HudBatch::text_width(a, text).max(1.0);
        if raw * max_scale <= max_w {
            return (text.to_owned(), max_scale);
        }
        let min_scale = (max_scale * 0.72).max(0.60);
        if raw * min_scale <= max_w {
            return (text.to_owned(), max_w / raw);
        }
        let mut s = text.to_owned();
        while !s.is_empty() && HudBatch::text_width(a, &format!("{s}..")) * min_scale > max_w {
            s.pop();
        }
        let out = if s.is_empty() {
            String::new()
        } else {
            format!("{s}..")
        };
        (out, min_scale)
    }

    fn game_mode_badge(mode: i32, hardcore: bool) -> (&'static str, u32) {
        if hardcore {
            ("Hardcore", ACCENT_RED)
        } else {
            match mode {
                1 => ("Creative", ACCENT_AMBER),
                2 => ("Adventure", ACCENT_VIOLET),
                3 => ("Spectator", ACCENT_SILVER),
                _ => ("Survival", ACCENT_GREEN),
            }
        }
    }

    fn world_preset_label(preset: WorldPreset) -> String {
        let key = preset.label();
        let fallback = match preset {
            WorldPreset::Normal => "Default",
            WorldPreset::Flat => "Superflat",
            WorldPreset::LargeBiomes => "Large Biomes",
            WorldPreset::Amplified => "Amplified",
            WorldPreset::SingleBiomeSurface => "Single Biome",
            WorldPreset::Debug => "Debug",
        };
        lang::get_or(key, fallback)
    }

    // -------------------------------------------------------------------------
    // Top Screen Rendering (Clean Panorama + 3D Showcase)
    // -------------------------------------------------------------------------

    pub(super) fn draw_shulkds_title_top(&mut self, batch: &mut HudBatch) {
        let (w, h) = (batch.width, batch.height);
        let kind = self.current_title_kind();
        let show_top_bar = matches!(
            kind,
            Some(
                BottomTitleKind::Select
                    | BottomTitleKind::Create
                    | BottomTitleKind::MultiplayerList
            )
        );
        self.draw_title_top_vignette(batch, w, h, show_top_bar);

        if let Some(upd) = self.shulkds_update_modal_view() {
            batch.fill(&self.gui.atlas, 0.0, 0.0, w, h, rgba(8, 10, 14, 175));
            let card_w = (w - 40.0).clamp(180.0, 340.0);
            let card_h = (h - 32.0).clamp(112.0, 176.0);
            let cx = ((w - card_w) * 0.5).floor();
            let cy = ((h - card_h) * 0.5).floor();
            self.draw_stone_panel(batch, [cx, cy, card_w, card_h]);
            batch.fill(
                &self.gui.atlas,
                cx + 2.0,
                cy + 2.0,
                card_w - 4.0,
                2.0,
                ACCENT_GREEN,
            );

            let hdr = if upd.version.is_empty() {
                upd.stage_title.clone()
            } else {
                format!("ShulkDS v{} Available", upd.version)
            };
            let (hdr_fit, hs) = self.fit_text_width(&hdr, card_w - 24.0, 1.10);
            let hw = HudBatch::text_width(&self.gui.atlas, &hdr_fit) * hs;
            batch.text_scaled(
                &self.gui.atlas,
                &hdr_fit,
                cx + (card_w - hw) * 0.5,
                cy + 10.0,
                hs,
                ACCENT_GREEN,
                true,
            );

            let mut ly = cy + 25.0;
            if !upd.size_mb.is_empty() {
                let sz = format!("Download size: {} MB", upd.size_mb);
                let sw = HudBatch::text_width(&self.gui.atlas, &sz) * 0.80;
                batch.text_scaled(
                    &self.gui.atlas,
                    &sz,
                    cx + (card_w - sw) * 0.5,
                    ly,
                    0.80,
                    TEXT_MUTED,
                    true,
                );
                ly += 13.0;
            }

            let inner_w = card_w - 24.0;
            if !upd.highlights.is_empty() {
                batch.text_scaled(
                    &self.gui.atlas,
                    "Highlights:",
                    cx + 12.0,
                    ly,
                    0.85,
                    ACCENT_AMBER,
                    true,
                );
                ly += 11.0;
                let max_y = cy + card_h - 14.0;
                for item in &upd.highlights {
                    let bullet = format!("- {item}");
                    let txt = horizon_client::text::Text::literal(bullet);
                    let wrapped = self.gui.atlas.font.split(&txt, inner_w / 0.80);
                    for line in wrapped.into_iter().take(2) {
                        if ly + 9.0 > max_y {
                            break;
                        }
                        let s = line.plain();
                        batch.text_scaled(
                            &self.gui.atlas,
                            &s,
                            cx + 12.0,
                            ly,
                            0.80,
                            TEXT_WHITE,
                            true,
                        );
                        ly += 10.0;
                    }
                    ly += 2.0;
                    if ly + 9.0 > max_y {
                        break;
                    }
                }
            } else {
                let txt = horizon_client::text::Text::literal(upd.body.clone());
                let wrapped = self.gui.atlas.font.split(&txt, inner_w / 0.85);
                for line in wrapped.into_iter().take(6) {
                    let s = line.plain();
                    batch.text_scaled(&self.gui.atlas, &s, cx + 12.0, ly, 0.85, TEXT_WHITE, true);
                    ly += 11.0;
                }
            }
            return;
        }

        match kind {
            Some(BottomTitleKind::Main) => {
                let hide_splash = self.settings.file.bool("hideSplashTexts");
                let logo_cx = (w * 0.5).floor();
                let logo_scale = ((w * 0.68) / 280.0).clamp(0.70, 1.08);
                let logo_h = 280.0 * logo_scale * (215.0 / 1024.0);
                let logo_y = (h * 0.10).floor().max(16.0);
                if let Some(title) = self.title.as_ref() {
                    title.draw_logo_and_splash(
                        &self.gui.atlas,
                        batch,
                        logo_cx,
                        logo_y,
                        logo_scale,
                        hide_splash,
                    );
                }

                // Centered live animated 3D Player Avatar beneath the logo
                let av_top = (logo_y + logo_h + 26.0).floor();
                let av_bot = (h - 20.0).floor();
                if av_bot - av_top >= 52.0 {
                    let av_h = (av_bot - av_top).clamp(52.0, 118.0);
                    let av_w = (av_h * 0.85).clamp(52.0, 108.0);
                    let av_x0 = ((w - av_w) * 0.5).floor();
                    let p_rect = [av_x0, av_top, av_x0 + av_w, av_top + av_h];
                    let model_sz = (av_h * 0.40).clamp(22.0, 44.0);
                    self.draw_title_player_in(batch, p_rect, model_sz, true);
                }

                // Subtle bottom footer bar
                let ver = format!(
                    "Horizon/ShulkDS v{} ({})",
                    horizon_core::release::VERSION,
                    horizon_core::VANILLA_VERSION
                );
                batch.text_scaled(&self.gui.atlas, &ver, 6.0, h - 11.0, 0.85, TEXT_MUTED, true);
                let notice = lang::get_or(
                    "horizon.title.not_affiliated",
                    "Not affiliated with Mojang or Microsoft",
                );
                let nw = HudBatch::text_width(&self.gui.atlas, &notice) * 0.80;
                batch.text_scaled(
                    &self.gui.atlas,
                    &notice,
                    w - nw - 6.0,
                    h - 11.0,
                    0.80,
                    TEXT_MUTED,
                    true,
                );
            }
            Some(BottomTitleKind::Select) => {
                // Auto-select first world so the Hero Showcase card is immediately populated
                if let Some(title) = self.title.as_mut()
                    && let Mode::Select { worlds, selected } = &mut title.mode
                    && selected.is_none()
                    && !worlds.is_empty()
                {
                    *selected = Some(0);
                }
                let Some(title) = self.title.as_ref() else {
                    return;
                };
                let Mode::Select { worlds, selected } = &title.mode else {
                    return;
                };

                batch.text_scaled(
                    &self.gui.atlas,
                    "HOME  >  SINGLEPLAYER",
                    8.0,
                    5.0,
                    0.85,
                    TEXT_MUTED,
                    true,
                );
                let count_txt = format!("{} Worlds", worlds.len());
                let cw = HudBatch::text_width(&self.gui.atlas, &count_txt) * 0.85;
                batch.text_scaled(
                    &self.gui.atlas,
                    &count_txt,
                    w - cw - 8.0,
                    5.0,
                    0.85,
                    ACCENT_GREEN,
                    true,
                );

                let card_w = (w - 36.0).min(340.0);
                let card_h = (h - 68.0).clamp(92.0, 128.0);
                let cx = ((w - card_w) * 0.5).floor();
                let cy = ((h - card_h) * 0.5).floor();

                if let Some(idx) = *selected
                    && let Some(world) = worlds.get(idx)
                {
                    let (mode_lbl, mode_col) =
                        Self::game_mode_badge(world.game_mode, world.hardcore);
                    self.draw_stone_panel(batch, [cx, cy, card_w, card_h]);

                    let icon_sz = (card_h - 32.0).clamp(42.0, 58.0);
                    let ix = cx + 14.0;
                    let iy = cy + (card_h - icon_sz) * 0.5;
                    batch.fill(
                        &self.gui.atlas,
                        ix - 2.0,
                        iy - 2.0,
                        icon_sz + 4.0,
                        icon_sz + 4.0,
                        BORDER_DARK,
                    );
                    title.draw_world_icon_at(
                        &self.gui.atlas,
                        batch,
                        world,
                        idx % horizon_client::hud::WORLD_ICON_SLOTS,
                        ix,
                        iy,
                        icon_sz,
                    );

                    let tx = ix + icon_sz + 14.0;
                    let tw_avail = (cx + card_w - 12.0 - tx).max(60.0);
                    let (wname, wscale) = self.fit_text_width(&world.name, tw_avail, 1.35);
                    batch.text_scaled(
                        &self.gui.atlas,
                        &wname,
                        tx,
                        iy + 2.0,
                        wscale,
                        TEXT_WHITE,
                        true,
                    );

                    // Pills: Game mode + Commands
                    let pill_y = iy + 20.0;
                    let mpw = HudBatch::text_width(&self.gui.atlas, mode_lbl) * 0.85 + 10.0;
                    self.draw_title_card(
                        batch,
                        [tx, pill_y, mpw, 13.0],
                        rgba(36, 36, 38, 240),
                        mode_col,
                        None,
                    );
                    batch.text_scaled(
                        &self.gui.atlas,
                        mode_lbl,
                        tx + 5.0,
                        pill_y + 3.0,
                        0.85,
                        mode_col,
                        true,
                    );

                    if world.commands {
                        let cmd_x = tx + mpw + 5.0;
                        let cmd_lbl = "Commands";
                        let cmw = HudBatch::text_width(&self.gui.atlas, cmd_lbl) * 0.82 + 10.0;
                        if cmd_x + cmw < cx + card_w - 8.0 {
                            self.draw_title_card(
                                batch,
                                [cmd_x, pill_y, cmw, 13.0],
                                rgba(36, 36, 38, 240),
                                BORDER_SOFT,
                                None,
                            );
                            batch.text_scaled(
                                &self.gui.atlas,
                                cmd_lbl,
                                cmd_x + 5.0,
                                pill_y + 3.0,
                                0.82,
                                TEXT_MUTED,
                                true,
                            );
                        }
                    }

                    let date: chrono::DateTime<chrono::Local> = world.modified.into();
                    let meta = format!(
                        "Folder: {}  ·  {}",
                        world.folder,
                        date.format("%Y-%m-%d %H:%M")
                    );
                    let (meta_fit, ms) = self.fit_text_width(&meta, tw_avail, 0.82);
                    batch.text_scaled(
                        &self.gui.atlas,
                        &meta_fit,
                        tx,
                        pill_y + 19.0,
                        ms,
                        TEXT_MUTED,
                        true,
                    );

                    let status_line = if world.corrupted {
                        ("Corrupted level data (tap Play to recover)", ACCENT_RED)
                    } else if world.locked {
                        ("World is currently locked", ACCENT_RED)
                    } else if let Some(incomp) = &world.incompatible {
                        let _ = incomp;
                        ("Incompatible world version", ACCENT_AMBER)
                    } else if world.file_fixing || world.downgrade {
                        ("World needs upgrade/backup", ACCENT_AMBER)
                    } else {
                        (
                            &*format!(
                                "Minecraft {}",
                                if world.version.is_empty() {
                                    horizon_core::VANILLA_VERSION
                                } else {
                                    &world.version
                                }
                            ),
                            TEXT_MUTED,
                        )
                    };
                    let (st_fit, ss) = self.fit_text_width(status_line.0, tw_avail, 0.82);
                    batch.text_scaled(
                        &self.gui.atlas,
                        &st_fit,
                        tx,
                        pill_y + 31.0,
                        ss,
                        status_line.1,
                        true,
                    );
                } else {
                    self.draw_stone_panel(batch, [cx, cy, card_w, 72.0]);
                    let msg = "No worlds yet — tap + New below to begin";
                    let mw = HudBatch::text_width(&self.gui.atlas, msg) * 0.95;
                    batch.text_scaled(
                        &self.gui.atlas,
                        msg,
                        cx + (card_w - mw) * 0.5,
                        cy + 32.0,
                        0.95,
                        TEXT_WHITE,
                        true,
                    );
                }
                let hint = "Select a world below, or double-tap to launch";
                let hw = HudBatch::text_width(&self.gui.atlas, hint) * 0.82;
                batch.text_scaled(
                    &self.gui.atlas,
                    hint,
                    (w - hw) * 0.5,
                    h - 11.0,
                    0.82,
                    TEXT_MUTED,
                    true,
                );
            }
            Some(BottomTitleKind::Create) => {
                let Some(title) = self.title.as_ref() else {
                    return;
                };
                let Mode::Create {
                    name,
                    seed,
                    creative,
                    options,
                    ..
                } = &title.mode
                else {
                    return;
                };
                batch.text_scaled(
                    &self.gui.atlas,
                    "HOME  >  SINGLEPLAYER  >  CREATE WORLD",
                    8.0,
                    5.0,
                    0.85,
                    TEXT_MUTED,
                    true,
                );

                let (mode_lbl, mode_col) =
                    Self::game_mode_badge(if *creative { 1 } else { 0 }, options.hardcore);
                let diff_idx = options.difficulty().min(DIFFICULTIES.len() - 1);
                let diff_lbl = lang::get_or(
                    &format!("options.difficulty.{}", DIFFICULTIES[diff_idx]),
                    DIFFICULTIES[diff_idx],
                );
                let preset_lbl = Self::world_preset_label(options.world_type);

                let card_w = (w - 40.0).min(336.0);
                let card_h = (h - 68.0).clamp(96.0, 128.0);
                let cx = ((w - card_w) * 0.5).floor();
                let cy = ((h - card_h) * 0.5).floor();
                self.draw_stone_panel(batch, [cx, cy, card_w, card_h]);

                let disp_name = if name.trim().is_empty() {
                    "New World"
                } else {
                    name.as_str()
                };
                let (nm_fit, ns) = self.fit_text_width(disp_name, card_w - 28.0, 1.40);
                batch.text_scaled(
                    &self.gui.atlas,
                    &nm_fit,
                    cx + 14.0,
                    cy + 12.0,
                    ns,
                    TEXT_WHITE,
                    true,
                );

                let badge_y = cy + 32.0;
                let bw1 = HudBatch::text_width(&self.gui.atlas, mode_lbl) * 0.85 + 12.0;
                self.draw_title_card(
                    batch,
                    [cx + 14.0, badge_y, bw1, 14.0],
                    rgba(36, 36, 38, 240),
                    mode_col,
                    None,
                );
                batch.text_scaled(
                    &self.gui.atlas,
                    mode_lbl,
                    cx + 20.0,
                    badge_y + 3.0,
                    0.85,
                    mode_col,
                    true,
                );

                let bw2 = HudBatch::text_width(&self.gui.atlas, &diff_lbl) * 0.85 + 12.0;
                self.draw_title_card(
                    batch,
                    [cx + 18.0 + bw1, badge_y, bw2, 14.0],
                    rgba(36, 36, 38, 240),
                    BORDER_SOFT,
                    None,
                );
                batch.text_scaled(
                    &self.gui.atlas,
                    &diff_lbl,
                    cx + 24.0 + bw1,
                    badge_y + 3.0,
                    0.85,
                    TEXT_WHITE,
                    true,
                );

                let seed_txt = if seed.trim().is_empty() {
                    "Seed: Random".to_owned()
                } else {
                    format!("Seed: {}", seed.trim())
                };
                batch.text_scaled(
                    &self.gui.atlas,
                    &format!("World Type: {preset_lbl}"),
                    cx + 14.0,
                    cy + 54.0,
                    0.90,
                    TEXT_WHITE,
                    true,
                );
                let (sd_fit, ss) = self.fit_text_width(&seed_txt, card_w - 28.0, 0.88);
                batch.text_scaled(
                    &self.gui.atlas,
                    &sd_fit,
                    cx + 14.0,
                    cy + 67.0,
                    ss,
                    TEXT_MUTED,
                    true,
                );

                let cmds = if options.commands(*creative) {
                    "Commands: ON"
                } else {
                    "Commands: OFF"
                };
                let structs = if options.structures {
                    "Structures: ON"
                } else {
                    "Structures: OFF"
                };
                let bonus = if options.bonus_chest() {
                    "Bonus Chest"
                } else {
                    "No Bonus"
                };
                let flags = format!("{cmds}  ·  {structs}  ·  {bonus}");
                let (fl_fit, fs) = self.fit_text_width(&flags, card_w - 28.0, 0.82);
                batch.text_scaled(
                    &self.gui.atlas,
                    &fl_fit,
                    cx + 14.0,
                    cy + card_h - 19.0,
                    fs,
                    ACCENT_BLUE,
                    true,
                );

                let hint = "Configure options on the bottom touch screen";
                let hw = HudBatch::text_width(&self.gui.atlas, hint) * 0.82;
                batch.text_scaled(
                    &self.gui.atlas,
                    hint,
                    (w - hw) * 0.5,
                    h - 11.0,
                    0.82,
                    TEXT_MUTED,
                    true,
                );
            }
            Some(BottomTitleKind::MultiplayerList) => {
                if let Some(title) = self.title.as_mut()
                    && let Mode::Multiplayer(screen) = &mut title.mode
                {
                    let total = screen.list_summaries().len();
                    if screen.selected_row().is_none() && total > 0 {
                        screen.set_selected_row(Some(0));
                    }
                }
                let Some(title) = self.title.as_ref() else {
                    return;
                };
                let Mode::Multiplayer(screen) = &title.mode else {
                    return;
                };
                let summaries = screen.list_summaries();
                let sel = screen.selected_row();

                batch.text_scaled(
                    &self.gui.atlas,
                    "HOME  >  MULTIPLAYER",
                    8.0,
                    5.0,
                    0.85,
                    TEXT_MUTED,
                    true,
                );
                let scan_lbl = format!("{} Listed", summaries.len());
                let sw = HudBatch::text_width(&self.gui.atlas, &scan_lbl) * 0.85;
                batch.text_scaled(
                    &self.gui.atlas,
                    &scan_lbl,
                    w - sw - 8.0,
                    5.0,
                    0.85,
                    ACCENT_BLUE,
                    true,
                );

                let card_w = (w - 36.0).min(340.0);
                let card_h = (h - 68.0).clamp(92.0, 126.0);
                let cx = ((w - card_w) * 0.5).floor();
                let cy = ((h - card_h) * 0.5).floor();

                if let Some(idx) = sel
                    && let Some(srv) = summaries.get(idx)
                {
                    let accent = if srv.is_lan {
                        ACCENT_GREEN
                    } else if srv.online {
                        ACCENT_BLUE
                    } else if srv.pinging {
                        ACCENT_AMBER
                    } else {
                        ACCENT_RED
                    };
                    self.draw_stone_panel(batch, [cx, cy, card_w, card_h]);

                    let icon_sz = (card_h - 32.0).clamp(42.0, 58.0);
                    let ix = cx + 14.0;
                    let iy = cy + (card_h - icon_sz) * 0.5;
                    batch.fill(
                        &self.gui.atlas,
                        ix - 2.0,
                        iy - 2.0,
                        icon_sz + 4.0,
                        icon_sz + 4.0,
                        BORDER_DARK,
                    );
                    screen.draw_row_icon(&self.gui.atlas, batch, idx, ix, iy, icon_sz);

                    let tx = ix + icon_sz + 14.0;
                    let tw_avail = (cx + card_w - 12.0 - tx).max(60.0);
                    let (sname, sscale) = self.fit_text_width(&srv.name, tw_avail, 1.35);
                    batch.text_scaled(
                        &self.gui.atlas,
                        &sname,
                        tx,
                        iy + 2.0,
                        sscale,
                        TEXT_WHITE,
                        true,
                    );

                    let status_txt = if srv.is_lan {
                        "LAN Game".to_owned()
                    } else if let Some(ms) = srv.ping_ms {
                        format!("{ms} ms")
                    } else if srv.pinging {
                        "Pinging...".to_owned()
                    } else if srv.incompatible {
                        "Incompatible".to_owned()
                    } else {
                        "Offline".to_owned()
                    };
                    let pill_y = iy + 20.0;
                    let spw = HudBatch::text_width(&self.gui.atlas, &status_txt) * 0.85 + 10.0;
                    self.draw_title_card(
                        batch,
                        [tx, pill_y, spw, 13.0],
                        rgba(36, 36, 38, 240),
                        accent,
                        None,
                    );
                    batch.text_scaled(
                        &self.gui.atlas,
                        &status_txt,
                        tx + 5.0,
                        pill_y + 3.0,
                        0.85,
                        accent,
                        true,
                    );

                    if let Some((online, max)) = srv.players {
                        let p_txt = format!("{online}/{max} players");
                        let px_pos = tx + spw + 6.0;
                        batch.text_scaled(
                            &self.gui.atlas,
                            &p_txt,
                            px_pos,
                            pill_y + 3.0,
                            0.85,
                            TEXT_WHITE,
                            true,
                        );
                    }

                    let motd_clean = srv.motd.replace('\n', " ");
                    let (motd_fit, ms) = self.fit_text_width(&motd_clean, tw_avail, 0.82);
                    batch.text_scaled(
                        &self.gui.atlas,
                        &motd_fit,
                        tx,
                        pill_y + 19.0,
                        ms,
                        TEXT_MUTED,
                        true,
                    );

                    if !self.settings.file.bool("hideServerAddress") {
                        let (addr_fit, as_s) = self.fit_text_width(&srv.address, tw_avail, 0.80);
                        batch.text_scaled(
                            &self.gui.atlas,
                            &addr_fit,
                            tx,
                            pill_y + 31.0,
                            as_s,
                            TEXT_MUTED,
                            true,
                        );
                    }
                } else {
                    self.draw_stone_panel(batch, [cx, cy, card_w, 72.0]);
                    let msg = "No servers saved — tap + Add or Direct below";
                    let mw = HudBatch::text_width(&self.gui.atlas, msg) * 0.92;
                    batch.text_scaled(
                        &self.gui.atlas,
                        msg,
                        cx + (card_w - mw) * 0.5,
                        cy + 32.0,
                        0.92,
                        TEXT_WHITE,
                        true,
                    );
                }
                let hint = "Select a server below or tap Direct Connection";
                let hw = HudBatch::text_width(&self.gui.atlas, hint) * 0.82;
                batch.text_scaled(
                    &self.gui.atlas,
                    hint,
                    (w - hw) * 0.5,
                    h - 11.0,
                    0.82,
                    TEXT_MUTED,
                    true,
                );
            }
            Some(BottomTitleKind::MultiplayerWarning) => {
                let card_w = (w - 48.0).min(336.0);
                let card_h = (h - 60.0).clamp(96.0, 136.0);
                let cx = ((w - card_w) * 0.5).floor();
                let cy = ((h - card_h) * 0.5).floor();
                self.draw_stone_panel(batch, [cx, cy, card_w, card_h]);
                let hdr = lang::get_or(
                    "multiplayerWarning.header",
                    "Caution: Third-Party Online Play",
                );
                let hw = HudBatch::text_width(&self.gui.atlas, &hdr) * 1.05;
                batch.text_scaled(
                    &self.gui.atlas,
                    &hdr,
                    cx + (card_w - hw) * 0.5,
                    cy + 12.0,
                    1.05,
                    ACCENT_AMBER,
                    true,
                );
                let msg = lang::get_or(
                    "multiplayerWarning.message",
                    "Online play is offered by third-party servers that are not owned, operated, or supervised by Mojang Studios or Microsoft.",
                );
                let msg_text = horizon_client::text::Text::literal(msg);
                let lines = self.gui.atlas.font.split(&msg_text, (card_w - 24.0) / 0.85);
                for (i, line) in lines.into_iter().take(5).enumerate() {
                    let s = line.plain();
                    batch.text_scaled(
                        &self.gui.atlas,
                        &s,
                        cx + 12.0,
                        cy + 30.0 + i as f32 * 10.0,
                        0.85,
                        TEXT_WHITE,
                        true,
                    );
                }
            }
            _ => {}
        }
    }

    // -------------------------------------------------------------------------
    // Bottom Screen Rendering & Touch Interaction
    // -------------------------------------------------------------------------

    pub(super) fn draw_shulkds_title_bottom(&mut self, batch: &mut HudBatch, rect: &ViewportRect) {
        let Some(kind) = self.current_title_kind() else {
            return;
        };
        let update_modal = self.shulkds_update_modal_view();
        let tag = if update_modal.is_some() {
            7
        } else {
            match kind {
                BottomTitleKind::Main => 0,
                BottomTitleKind::Select => 1,
                BottomTitleKind::Create => 2,
                BottomTitleKind::MultiplayerList => 3,
                BottomTitleKind::MultiplayerWarning => 4,
                BottomTitleKind::Options => 5,
                BottomTitleKind::DetailModal => 6,
            }
        };
        self.sync_title_mode_tag(tag);
        self.shulkds_title.hits.clear();
        self.shulkds_title.scroll_rect = None;
        self.shulkds_title.max_scroll = 0.0;

        let (bx, by, bw, bh) = (rect.x, rect.y, rect.width, rect.height);
        let screen_w = batch.width;
        if bx > 0.0 || (bx + bw) < screen_w {
            let bezel = rgba(10, 10, 14, 255);
            let border = rgba(28, 28, 34, 255);
            if bx > 0.0 {
                batch.fill(&self.gui.atlas, 0.0, by, bx, bh, bezel);
                batch.fill(&self.gui.atlas, bx - 1.0, by, 1.0, bh, border);
            }
            let rx = bx + bw;
            if rx < screen_w {
                batch.fill(&self.gui.atlas, rx, by, screen_w - rx, bh, bezel);
                batch.fill(&self.gui.atlas, rx, by, 1.0, bh, border);
            }
        }

        // Official Minecraft tiled dark Dirt menu backdrop
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
                rgba(74, 74, 74, 255),
            );
        } else {
            batch.fill(&self.gui.atlas, bx, by, bw, bh, rgba(34, 24, 18, 255));
        }

        if let Some(upd) = update_modal {
            self.draw_bottom_title_update(batch, bx, by, bw, bh, &upd);
            return;
        }

        match kind {
            BottomTitleKind::Main => self.draw_bottom_title_main(batch, bx, by, bw, bh),
            BottomTitleKind::Select => self.draw_bottom_title_select(batch, bx, by, bw, bh),
            BottomTitleKind::Create => self.draw_bottom_title_create(batch, bx, by, bw, bh),
            BottomTitleKind::MultiplayerList => {
                self.draw_bottom_title_multiplayer(batch, bx, by, bw, bh)
            }
            BottomTitleKind::MultiplayerWarning => {
                self.draw_bottom_title_warning(batch, bx, by, bw, bh)
            }
            BottomTitleKind::Options => self.draw_bottom_title_options(batch, bx, by, bw, bh),
            BottomTitleKind::DetailModal => self.draw_bottom_title_modal(batch, bx, by, bw, bh),
        }
    }

    fn draw_bottom_title_main(&mut self, batch: &mut HudBatch, bx: f32, by: f32, bw: f32, bh: f32) {
        let pad = 8.0;
        let ix = bx + pad;
        let iw = (bw - pad * 2.0).max(80.0);

        // Header strip
        let hdr_y = by + 6.0;
        let hdr_h = 16.0;
        let ver_hdr = format!("Minecraft {}", horizon_core::VANILLA_VERSION);
        batch.text_scaled(
            &self.gui.atlas,
            &ver_hdr,
            ix + 2.0,
            hdr_y + 6.0,
            0.92,
            TEXT_MUTED,
            true,
        );
        let player_name = self.worlds_name();
        let prof_lbl = format!("Profile: {player_name}");
        let prof_w =
            (HudBatch::text_width(&self.gui.atlas, &prof_lbl) * 0.80 + 16.0).clamp(76.0, iw * 0.62);
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + iw - prof_w, hdr_y, prof_w, hdr_h],
            &prof_lbl,
            Some(ACCENT_SILVER),
            true,
            false,
            HitAct::MainProfile,
        );

        let world_cnt = self.title.as_ref().map_or(0, |t| t.world_count_cached());
        let allows_mp = self.title.as_ref().is_some_and(|t| t.allows_multiplayer);

        let util_h = 16.0;
        let util_y = by + bh - util_h - 8.0;
        let cards_y = hdr_y + hdr_h + 7.0;
        let cards_h = (util_y - 7.0 - cards_y).max(60.0);
        let gap = 6.0;
        let row1_h = ((cards_h - gap) * 0.50).floor().clamp(26.0, 38.0);
        let row2_h = ((cards_h - gap) * 0.50).floor().clamp(24.0, 36.0);
        let grid_h = row1_h + gap + row2_h;
        let cards_y = cards_y + ((cards_h - grid_h) * 0.5).max(0.0).floor();
        let col_w = ((iw - gap) * 0.5).floor();
        let col2_w = (iw - col_w - gap).max(col_w);

        let sp_sub = if world_cnt == 0 {
            "Create Your First World".to_owned()
        } else if world_cnt == 1 {
            "1 Saved World".to_owned()
        } else {
            format!("{world_cnt} Saved Worlds")
        };
        let sp_title = lang::get_or("menu.singleplayer", "Singleplayer");
        let mp_title = lang::get_or("menu.multiplayer", "Multiplayer");
        let opt_title = lang::get_or("menu.options", "Options");

        self.draw_hero_menu_tile(
            batch,
            bx,
            by,
            [ix, cards_y, col_w, row1_h],
            &sp_title,
            &sp_sub,
            ACCENT_GREEN,
            Some("block/grass_block_side"),
            false,
            true,
            HitAct::MainSingleplayer,
        );
        self.draw_hero_menu_tile(
            batch,
            bx,
            by,
            [ix + col_w + gap, cards_y, col2_w, row1_h],
            &mp_title,
            if allows_mp {
                "Servers & LAN"
            } else {
                "Multiplayer Disabled"
            },
            ACCENT_BLUE,
            Some("item/ender_eye"),
            false,
            allows_mp,
            HitAct::MainMultiplayer,
        );
        self.draw_hero_menu_tile(
            batch,
            bx,
            by,
            [ix, cards_y + row1_h + gap, col_w, row2_h],
            "Friends",
            "Join Invite Code",
            ACCENT_AMBER,
            Some("gui/sprites/friends/friends"),
            false,
            true,
            HitAct::MainFriends,
        );
        self.draw_hero_menu_tile(
            batch,
            bx,
            by,
            [ix + col_w + gap, cards_y + row1_h + gap, col2_w, row2_h],
            &opt_title,
            "Video, Sound & Pad",
            ACCENT_SILVER,
            None,
            true,
            true,
            HitAct::MainOptions,
        );

        // Bottom 4-pill utility bar
        let u_gap = 4.0;
        let uw = ((iw - u_gap * 3.0) / 4.0).floor();
        let last_uw = (iw - (uw + u_gap) * 3.0).max(uw);
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, util_y, uw, util_h],
            "Language",
            None,
            true,
            false,
            HitAct::MainLanguage,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + uw + u_gap, util_y, uw, util_h],
            "Access.",
            None,
            true,
            false,
            HitAct::MainAccess,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + (uw + u_gap) * 2.0, util_y, uw, util_h],
            "Credits",
            None,
            true,
            false,
            HitAct::MainCredits,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + (uw + u_gap) * 3.0, util_y, last_uw, util_h],
            "Quit",
            Some(ACCENT_RED),
            true,
            false,
            HitAct::MainQuit,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_hero_menu_tile(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        rect: [f32; 4],
        title: &str,
        subtitle: &str,
        _accent: u32,
        sprite: Option<&str>,
        gear_icon: bool,
        enabled: bool,
        act: HitAct,
    ) {
        let focus_ordinal = self
            .shulkds_title
            .hits
            .iter()
            .filter(|h| h.focusable)
            .count();
        let focused = self.shulkds_title.focus_idx == focus_ordinal && enabled;
        self.shulkds_title.hits.push(HitItem {
            rect: [rect[0] - bx, rect[1] - by, rect[2], rect[3]],
            act,
            enabled,
            focusable: true,
        });
        let hovered =
            enabled && self.hit_under_pointer([rect[0] - bx, rect[1] - by, rect[2], rect[3]]);
        let pressed = hovered && self.shulkds_title.pointer_down;
        self.draw_official_button(batch, rect, enabled, focused, hovered, false, pressed);

        let press_off = if pressed { 1.0 } else { 0.0 };
        let icon_sz = (rect[3] - 18.0).clamp(14.0, 24.0);
        let ix = rect[0] + 9.0;
        let iy = rect[1] + (rect[3] - icon_sz) * 0.5 + press_off;
        let mut text_x = ix;
        let sprite_key = sprite.and_then(|sp| {
            if self.gui.atlas.sprites.contains_key(sp) {
                Some(sp)
            } else if sp == "gui/sprites/friends/friends"
                && self.gui.atlas.sprites.contains_key("item/compass_16")
            {
                Some("item/compass_16")
            } else {
                None
            }
        });
        if let Some(sp) = sprite_key {
            self.draw_title_card(
                batch,
                [ix - 2.0, iy - 2.0, icon_sz + 4.0, icon_sz + 4.0],
                rgba(16, 16, 20, 150),
                rgba(0, 0, 0, 175),
                None,
            );
            batch.sprite_scaled(
                &self.gui.atlas,
                sp,
                ix,
                iy,
                icon_sz,
                icon_sz,
                if enabled {
                    u32::MAX
                } else {
                    rgba(140, 140, 140, 180)
                },
            );
            text_x = ix + icon_sz + 8.0;
        } else if gear_icon {
            self.draw_title_card(
                batch,
                [ix - 2.0, iy - 2.0, icon_sz + 4.0, icon_sz + 4.0],
                rgba(16, 16, 20, 150),
                rgba(0, 0, 0, 175),
                None,
            );
            self.draw_3ds_options_icon(batch, ix, iy, icon_sz);
            text_x = ix + icon_sz + 8.0;
        }

        let avail = (rect[0] + rect[2] - 6.0 - text_x).max(20.0);
        let (t_fit, ts) = self.fit_text_width(title, avail, 1.05);
        let (s_fit, ss) = self.fit_text_width(subtitle, avail, 0.78);
        let block_h = 8.0 * ts + 3.0 + 8.0 * ss;
        let ty0 = rect[1] + (rect[3] - block_h) * 0.5 + press_off;
        let fg = if !enabled {
            rgba(160, 160, 160, 255)
        } else if focused || hovered {
            rgba(255, 255, 160, 255)
        } else {
            TEXT_WHITE
        };
        let sub_fg = if !enabled {
            rgba(128, 128, 134, 255)
        } else {
            rgba(214, 218, 228, 255)
        };
        batch.text_scaled(&self.gui.atlas, &t_fit, text_x, ty0, ts, fg, true);
        batch.text_scaled(
            &self.gui.atlas,
            &s_fit,
            text_x,
            ty0 + 8.0 * ts + 3.0,
            ss,
            sub_fg,
            true,
        );
    }

    fn draw_bottom_title_select(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        bw: f32,
        bh: f32,
    ) {
        let pad = 8.0;
        let ix = bx + pad;
        let iw = (bw - pad * 2.0).max(80.0);

        // Header: [< Back]  Singleplayer  [+ New]
        let hdr_y = by + 6.0;
        let hdr_h = 16.0;
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, hdr_y, 54.0, hdr_h],
            "< Back",
            None,
            true,
            false,
            HitAct::BackToMain,
        );
        let hdr = lang::get_or("menu.singleplayer", "Singleplayer");
        let hw = HudBatch::text_width(&self.gui.atlas, &hdr);
        batch.text(
            &self.gui.atlas,
            &hdr,
            ix + (iw - hw) * 0.5,
            hdr_y + 4.0,
            TEXT_WHITE,
            true,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + iw - 58.0, hdr_y, 58.0, hdr_h],
            "+ New",
            Some(ACCENT_GREEN),
            true,
            false,
            HitAct::SelectCreate,
        );

        let dock_h = 18.0;
        let dock_y = by + bh - dock_h - 8.0;
        let list_y = hdr_y + hdr_h + 6.0;
        let list_h = (dock_y - 6.0 - list_y).max(40.0);

        let worlds_snapshot: Vec<(WorldEntry, bool)> = if let Some(title) = self.title.as_ref()
            && let Mode::Select { worlds, selected } = &title.mode
        {
            worlds
                .iter()
                .enumerate()
                .map(|(i, w)| (w.clone(), *selected == Some(i)))
                .collect()
        } else {
            Vec::new()
        };

        let row_h = 30.0;
        let row_gap = 4.0;
        let row_step = row_h + row_gap;
        let content_h = (worlds_snapshot.len() as f32 * row_step - row_gap).max(0.0);
        let max_scroll = (content_h - list_h).max(0.0);
        self.shulkds_title.max_scroll = max_scroll;
        self.shulkds_title.world_scroll = self.shulkds_title.world_scroll.clamp(0.0, max_scroll);
        self.shulkds_title.scroll_rect = Some([pad, list_y - by, iw, list_h]);

        let list_w = if max_scroll > 0.0 { iw - 7.0 } else { iw };
        let clip_start = batch.vertices.len();
        if worlds_snapshot.is_empty() {
            self.draw_stone_panel(batch, [ix, list_y, list_w, 40.0]);
            let msg = "No worlds saved — tap + New to create one";
            let mw = HudBatch::text_width(&self.gui.atlas, msg) * 0.85;
            batch.text_scaled(
                &self.gui.atlas,
                msg,
                ix + (list_w - mw) * 0.5,
                list_y + 16.0,
                0.85,
                TEXT_MUTED,
                true,
            );
        } else {
            for (i, (world, is_sel)) in worlds_snapshot.iter().enumerate() {
                let ry = list_y + i as f32 * row_step - self.shulkds_title.world_scroll;
                if ry + row_h < list_y || ry > list_y + list_h {
                    continue;
                }
                let focus_ord = self
                    .shulkds_title
                    .hits
                    .iter()
                    .filter(|h| h.focusable)
                    .count();
                if self.shulkds_title.pending_row_focus == Some(i) {
                    self.shulkds_title.focus_idx = focus_ord;
                    self.shulkds_title.pending_row_focus = None;
                }
                let focused = self.shulkds_title.focus_idx == focus_ord;
                let r = [ix, ry, list_w, row_h];
                self.shulkds_title.hits.push(HitItem {
                    rect: Self::clipped_list_hit_rect(r, bx, by, list_y, list_h),
                    act: HitAct::SelectRow(i),
                    enabled: true,
                    focusable: true,
                });
                let (_, m_col) = Self::game_mode_badge(world.game_mode, world.hardcore);
                let hovered = self.hit_under_pointer([r[0] - bx, r[1] - by, r[2], r[3]]);
                self.draw_official_button(batch, r, true, focused, hovered, *is_sel, false);
                batch.fill(&self.gui.atlas, ix + 1.0, ry + 1.0, 3.0, row_h - 2.0, m_col);

                let icon_sz = 22.0;
                if let Some(title) = self.title.as_ref() {
                    title.draw_world_icon_at(
                        &self.gui.atlas,
                        batch,
                        world,
                        i % horizon_client::hud::WORLD_ICON_SLOTS,
                        ix + 8.0,
                        ry + 4.0,
                        icon_sz,
                    );
                }
                let tx = ix + 35.0;
                let avail = (ix + list_w - 8.0 - tx).max(30.0);
                let (wname, ws) = self.fit_text_width(&world.name, avail, 0.96);
                batch.text_scaled(&self.gui.atlas, &wname, tx, ry + 5.0, ws, TEXT_WHITE, true);
                let (m_lbl, _) = Self::game_mode_badge(world.game_mode, world.hardcore);
                let sub = format!(
                    "{m_lbl} · {}",
                    if world.version.is_empty() {
                        horizon_core::VANILLA_VERSION
                    } else {
                        &world.version
                    }
                );
                let (sub_fit, ss) = self.fit_text_width(&sub, avail, 0.78);
                batch.text_scaled(
                    &self.gui.atlas,
                    &sub_fit,
                    tx,
                    ry + 17.0,
                    ss,
                    TEXT_MUTED,
                    true,
                );
            }
        }
        batch.clip_since(clip_start, ix, list_y, ix + list_w, list_y + list_h);

        // Scrollbar
        if max_scroll > 0.0 {
            let sb_x = ix + iw - 4.0;
            batch.fill(
                &self.gui.atlas,
                sb_x,
                list_y,
                3.0,
                list_h,
                rgba(18, 18, 20, 220),
            );
            let thumb_h = ((list_h / content_h) * list_h).clamp(16.0, list_h);
            let thumb_y =
                list_y + (self.shulkds_title.world_scroll / max_scroll) * (list_h - thumb_h);
            batch.fill(
                &self.gui.atlas,
                sb_x,
                thumb_y,
                3.0,
                thumb_h,
                rgba(164, 164, 170, 255),
            );
        }

        // Action dock: [Play World] [Edit] [Copy] [Del]
        let sel_world = worlds_snapshot.iter().find(|(_, s)| *s).map(|(w, _)| w);
        let can_play = sel_world.is_some_and(WorldEntry::can_play);
        let can_edit = sel_world.is_some_and(WorldEntry::can_edit);
        let play_lbl = if sel_world.is_some_and(|w| w.corrupted) {
            "Recover"
        } else {
            "Play World"
        };

        let d_gap = 4.0;
        let small_w = ((iw - d_gap * 3.0) * 0.175).floor().clamp(36.0, 54.0);
        let play_w = (iw - small_w * 3.0 - d_gap * 3.0).max(60.0);
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, dock_y, play_w, dock_h],
            play_lbl,
            Some(ACCENT_GREEN),
            can_play,
            false,
            HitAct::SelectPlay,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + play_w + d_gap, dock_y, small_w, dock_h],
            "Edit",
            None,
            can_edit,
            false,
            HitAct::SelectEdit,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [
                ix + play_w + d_gap + small_w + d_gap,
                dock_y,
                small_w,
                dock_h,
            ],
            "Copy",
            None,
            can_edit,
            false,
            HitAct::SelectCopy,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [
                ix + play_w + d_gap + (small_w + d_gap) * 2.0,
                dock_y,
                small_w,
                dock_h,
            ],
            "Del",
            Some(ACCENT_RED),
            sel_world.is_some(),
            false,
            HitAct::SelectDelete,
        );
    }

    fn draw_bottom_title_create(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        bw: f32,
        bh: f32,
    ) {
        let pad = 8.0;
        let ix = bx + pad;
        let iw = (bw - pad * 2.0).max(80.0);

        let (name, seed, creative, focus, options) = if let Some(title) = self.title.as_ref()
            && let Mode::Create {
                name,
                seed,
                creative,
                focus,
                options,
            } = &title.mode
        {
            (
                name.clone(),
                seed.clone(),
                *creative,
                *focus,
                options.clone(),
            )
        } else {
            return;
        };

        // Top tabs: [<] [Game] [World] [More]
        let hdr_y = by + 6.0;
        let hdr_h = 16.0;
        let back_w = 26.0;
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, hdr_y, back_w, hdr_h],
            "<",
            None,
            true,
            false,
            HitAct::CreateButton(22),
        );
        let tab_area_x = ix + back_w + 4.0;
        let tab_area_w = (iw - back_w - 4.0).max(60.0);
        let tab_w = ((tab_area_w - 6.0) / 3.0).floor();
        let tab_labels = ["Game", "World", "More"];
        for (ti, lbl) in tab_labels.iter().enumerate() {
            let tw = if ti == 2 {
                tab_area_w - (tab_w + 3.0) * 2.0
            } else {
                tab_w
            };
            self.draw_title_pill_btn(
                batch,
                bx,
                by,
                [tab_area_x + ti as f32 * (tab_w + 3.0), hdr_y, tw, hdr_h],
                lbl,
                if options.tab == ti {
                    Some(ACCENT_GREEN)
                } else {
                    None
                },
                true,
                options.tab == ti,
                HitAct::CreateTab(ti),
            );
        }

        let dock_h = 18.0;
        let dock_y = by + bh - dock_h - 8.0;
        let body_y = hdr_y + hdr_h + 7.0;
        let body_h = (dock_y - 7.0 - body_y).max(60.0);
        let row_h = ((body_h - 26.0) / 4.0).floor().clamp(16.0, 26.0);

        match options.tab {
            0 => {
                let content_h = 4.0 * row_h + 26.0;
                let y0 = body_y + ((body_h - content_h) * 0.5).floor().max(0.0);
                // World Name field (tap focuses & triggers Android keyboard)
                batch.text_scaled(
                    &self.gui.atlas,
                    "World Name",
                    ix,
                    y0,
                    0.80,
                    TEXT_MUTED,
                    true,
                );
                let name_y = y0 + 10.0;
                let name_rect = [ix, name_y, iw, row_h];
                self.shulkds_title.hits.push(HitItem {
                    rect: [
                        name_rect[0] - bx,
                        name_rect[1] - by,
                        name_rect[2],
                        name_rect[3],
                    ],
                    act: HitAct::CreateFocusField(0),
                    enabled: true,
                    focusable: true,
                });
                self.draw_text_field_box(batch, name_rect, focus == 0);
                let caret = if focus == 0
                    && self.shulkds_text_caret_shown()
                    && (self.started.elapsed().as_millis() / 450) % 2 == 0
                {
                    "|"
                } else {
                    ""
                };
                let disp = format!("{name}{caret}");
                let (d_fit, ds) = self.fit_text_width(&disp, iw - 12.0, 0.95);
                batch.text_scaled(
                    &self.gui.atlas,
                    &d_fit,
                    ix + 6.0,
                    name_y + (row_h - 8.0 * ds) * 0.5,
                    ds,
                    TEXT_WHITE,
                    true,
                );

                // Game Mode 3-way selector
                let mode_y = name_y + row_h + 6.0;
                let mw = ((iw - 6.0) / 3.0).floor();
                let mw2 = iw - (mw + 3.0) * 2.0;
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix, mode_y, mw, row_h],
                    "Survival",
                    Some(ACCENT_GREEN),
                    true,
                    !creative && !options.hardcore,
                    HitAct::CreateMode(false, false),
                );
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix + mw + 3.0, mode_y, mw, row_h],
                    "Hardcore",
                    Some(ACCENT_RED),
                    true,
                    options.hardcore,
                    HitAct::CreateMode(false, true),
                );
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix + (mw + 3.0) * 2.0, mode_y, mw2, row_h],
                    "Creative",
                    Some(ACCENT_AMBER),
                    true,
                    creative && !options.hardcore,
                    HitAct::CreateMode(true, false),
                );

                // Difficulty 4-way selector
                let diff_y = mode_y + row_h + 5.0;
                let dw = ((iw - 9.0) / 4.0).floor();
                let cur_diff = options.difficulty();
                let d_labels = ["Peaceful", "Easy", "Normal", "Hard"];
                for (di, dl) in d_labels.iter().enumerate() {
                    let w_col = if di == 3 { iw - (dw + 3.0) * 3.0 } else { dw };
                    self.draw_title_pill_btn(
                        batch,
                        bx,
                        by,
                        [ix + di as f32 * (dw + 3.0), diff_y, w_col, row_h],
                        dl,
                        None,
                        !options.hardcore,
                        cur_diff == di,
                        HitAct::CreateDifficulty(di),
                    );
                }

                // Allow Commands toggle
                let cmd_y = diff_y + row_h + 5.0;
                let cmd_on = options.commands(creative);
                let cmd_lbl = if cmd_on {
                    "Allow Commands: ON"
                } else {
                    "Allow Commands: OFF"
                };
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix, cmd_y, iw, row_h.min(dock_y - 4.0 - cmd_y)],
                    cmd_lbl,
                    Some(ACCENT_BLUE),
                    !options.hardcore,
                    cmd_on,
                    HitAct::CreateButton(24),
                );
            }
            1 => {
                // World Type + Customize
                let content_h = 3.0 * row_h + 24.0;
                let wt_y = body_y + ((body_h - content_h) * 0.5).floor().max(0.0);
                let preset_lbl = format!(
                    "World Type: {}",
                    Self::world_preset_label(options.world_type)
                );
                let has_cust = matches!(
                    options.world_type,
                    WorldPreset::Flat | WorldPreset::SingleBiomeSurface
                );
                if has_cust {
                    let cust_w = 72.0;
                    self.draw_title_pill_btn(
                        batch,
                        bx,
                        by,
                        [ix, wt_y, iw - cust_w - 4.0, row_h],
                        &preset_lbl,
                        Some(ACCENT_BLUE),
                        true,
                        false,
                        HitAct::CreateButton(25),
                    );
                    self.draw_title_pill_btn(
                        batch,
                        bx,
                        by,
                        [ix + iw - cust_w, wt_y, cust_w, row_h],
                        "Customize",
                        Some(ACCENT_AMBER),
                        true,
                        false,
                        HitAct::CreateButton(26),
                    );
                } else {
                    self.draw_title_pill_btn(
                        batch,
                        bx,
                        by,
                        [ix, wt_y, iw, row_h],
                        &preset_lbl,
                        Some(ACCENT_BLUE),
                        true,
                        false,
                        HitAct::CreateButton(25),
                    );
                }

                // Seed input
                let slbl_y = wt_y + row_h + 7.0;
                batch.text_scaled(
                    &self.gui.atlas,
                    "Seed for the World Generator",
                    ix,
                    slbl_y,
                    0.80,
                    TEXT_MUTED,
                    true,
                );
                let seed_y = slbl_y + 10.0;
                let seed_rect = [ix, seed_y, iw, row_h];
                self.shulkds_title.hits.push(HitItem {
                    rect: [
                        seed_rect[0] - bx,
                        seed_rect[1] - by,
                        seed_rect[2],
                        seed_rect[3],
                    ],
                    act: HitAct::CreateFocusField(1),
                    enabled: true,
                    focusable: true,
                });
                self.draw_text_field_box(batch, seed_rect, focus == 1);
                let caret_active = focus == 1 && self.shulkds_text_caret_shown();
                let caret = if caret_active && (self.started.elapsed().as_millis() / 450) % 2 == 0 {
                    "|"
                } else {
                    ""
                };
                let s_disp = if seed.is_empty() && !caret_active {
                    "Leave blank for random seed".to_owned()
                } else {
                    format!("{seed}{caret}")
                };
                let s_col = if seed.is_empty() && !caret_active {
                    TEXT_MUTED
                } else {
                    TEXT_WHITE
                };
                let (s_fit, ss) = self.fit_text_width(&s_disp, iw - 12.0, 0.90);
                batch.text_scaled(
                    &self.gui.atlas,
                    &s_fit,
                    ix + 6.0,
                    seed_y + (row_h - 8.0 * ss) * 0.5,
                    ss,
                    s_col,
                    true,
                );

                // Structures & Bonus Chest
                let sw_y = seed_y + row_h + 7.0;
                let sw_w = ((iw - 4.0) * 0.5).floor();
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix, sw_y, sw_w, row_h],
                    if options.structures {
                        "Structures: ON"
                    } else {
                        "Structures: OFF"
                    },
                    Some(ACCENT_GREEN),
                    true,
                    options.structures,
                    HitAct::CreateButton(27),
                );
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix + sw_w + 4.0, sw_y, iw - sw_w - 4.0, row_h],
                    if options.bonus_chest() {
                        "Bonus Chest: ON"
                    } else {
                        "Bonus Chest: OFF"
                    },
                    Some(ACCENT_AMBER),
                    !options.hardcore,
                    options.bonus_chest(),
                    HitAct::CreateButton(28),
                );
            }
            _ => {
                // More tab: Game Rules, Data Packs, Experiments
                let btn_h = ((body_h - 12.0) / 3.0).floor().clamp(18.0, 26.0);
                let content_h = btn_h * 3.0 + 8.0;
                let y0 = body_y + ((body_h - content_h) * 0.5).floor().max(0.0);
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix, y0, iw, btn_h],
                    "Game Rules...",
                    Some(ACCENT_BLUE),
                    true,
                    false,
                    HitAct::CreateButton(29),
                );
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix, y0 + 4.0 + btn_h, iw, btn_h],
                    "Data Packs...",
                    Some(ACCENT_GREEN),
                    true,
                    false,
                    HitAct::CreateButton(31),
                );
                self.draw_title_pill_btn(
                    batch,
                    bx,
                    by,
                    [ix, y0 + 8.0 + btn_h * 2.0, iw, btn_h],
                    "Experiments...",
                    Some(ACCENT_AMBER),
                    true,
                    false,
                    HitAct::CreateButton(30),
                );
            }
        }

        // Bottom Create / Cancel dock
        let cw = ((iw - 6.0) * 0.68).floor();
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, dock_y, cw, dock_h],
            "Create New World",
            Some(ACCENT_GREEN),
            true,
            false,
            HitAct::CreateButton(21),
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + cw + 6.0, dock_y, iw - cw - 6.0, dock_h],
            "Cancel",
            Some(ACCENT_RED),
            true,
            false,
            HitAct::CreateButton(22),
        );
    }

    fn draw_bottom_title_multiplayer(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        bw: f32,
        bh: f32,
    ) {
        let pad = 8.0;
        let ix = bx + pad;
        let iw = (bw - pad * 2.0).max(80.0);

        let hdr_y = by + 6.0;
        let hdr_h = 16.0;
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, hdr_y, 54.0, hdr_h],
            "< Back",
            None,
            true,
            false,
            HitAct::BackToMain,
        );
        let hdr = lang::get_or("menu.multiplayer", "Multiplayer");
        let hw = HudBatch::text_width(&self.gui.atlas, &hdr);
        batch.text(
            &self.gui.atlas,
            &hdr,
            ix + (iw - hw) * 0.5,
            hdr_y + 4.0,
            TEXT_WHITE,
            true,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + iw - 96.0, hdr_y, 44.0, hdr_h],
            "Refresh",
            None,
            true,
            false,
            HitAct::MpButton(45),
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + iw - 48.0, hdr_y, 48.0, hdr_h],
            "+ Add",
            Some(ACCENT_GREEN),
            true,
            false,
            HitAct::MpButton(42),
        );

        let dock_h = 18.0;
        let dock_y = by + bh - dock_h - 8.0;
        let list_y = hdr_y + hdr_h + 6.0;
        let list_h = (dock_y - 6.0 - list_y).max(40.0);

        let (summaries, sel, saved_count) = if let Some(title) = self.title.as_ref()
            && let Mode::Multiplayer(screen) = &title.mode
        {
            (
                screen.list_summaries(),
                screen.selected_row(),
                screen.saved_server_count(),
            )
        } else {
            (Vec::new(), None, 0)
        };

        let row_h = 28.0;
        let row_gap = 4.0;
        let row_step = row_h + row_gap;
        let content_h = (summaries.len() as f32 * row_step - row_gap).max(0.0);
        let max_scroll = (content_h - list_h).max(0.0);
        self.shulkds_title.max_scroll = max_scroll;
        self.shulkds_title.server_scroll = self.shulkds_title.server_scroll.clamp(0.0, max_scroll);
        self.shulkds_title.scroll_rect = Some([pad, list_y - by, iw, list_h]);

        let list_w = if max_scroll > 0.0 { iw - 7.0 } else { iw };
        let clip_start = batch.vertices.len();
        if summaries.is_empty() {
            self.draw_stone_panel(batch, [ix, list_y, list_w, 40.0]);
            let msg = "No servers — tap + Add or Direct Connect";
            let mw = HudBatch::text_width(&self.gui.atlas, msg) * 0.85;
            batch.text_scaled(
                &self.gui.atlas,
                msg,
                ix + (list_w - mw) * 0.5,
                list_y + 16.0,
                0.85,
                TEXT_MUTED,
                true,
            );
        } else {
            for (i, srv) in summaries.iter().enumerate() {
                let ry = list_y + i as f32 * row_step - self.shulkds_title.server_scroll;
                if ry + row_h < list_y || ry > list_y + list_h {
                    continue;
                }
                let is_sel = sel == Some(i);
                let focus_ord = self
                    .shulkds_title
                    .hits
                    .iter()
                    .filter(|h| h.focusable)
                    .count();
                if self.shulkds_title.pending_row_focus == Some(i) {
                    self.shulkds_title.focus_idx = focus_ord;
                    self.shulkds_title.pending_row_focus = None;
                }
                let focused = self.shulkds_title.focus_idx == focus_ord;
                let r = [ix, ry, list_w, row_h];
                self.shulkds_title.hits.push(HitItem {
                    rect: Self::clipped_list_hit_rect(r, bx, by, list_y, list_h),
                    act: HitAct::MpRow(i),
                    enabled: true,
                    focusable: true,
                });
                let accent = if srv.is_lan {
                    ACCENT_GREEN
                } else if srv.online {
                    ACCENT_BLUE
                } else if srv.pinging {
                    ACCENT_AMBER
                } else {
                    ACCENT_RED
                };
                let hovered = self.hit_under_pointer([r[0] - bx, r[1] - by, r[2], r[3]]);
                self.draw_official_button(batch, r, true, focused, hovered, is_sel, false);
                batch.fill(
                    &self.gui.atlas,
                    ix + 1.0,
                    ry + 1.0,
                    3.0,
                    row_h - 2.0,
                    accent,
                );

                if let Some(title) = self.title.as_ref()
                    && let Mode::Multiplayer(screen) = &title.mode
                {
                    screen.draw_row_icon(&self.gui.atlas, batch, i, ix + 8.0, ry + 3.0, 22.0);
                }

                let right_txt = if srv.is_lan {
                    "LAN".to_owned()
                } else if let Some((on, max)) = srv.players {
                    format!("{on}/{max}")
                } else if srv.pinging {
                    "...".to_owned()
                } else {
                    "Off".to_owned()
                };
                let rtw = HudBatch::text_width(&self.gui.atlas, &right_txt) * 0.80;
                batch.text_scaled(
                    &self.gui.atlas,
                    &right_txt,
                    ix + list_w - rtw - 6.0,
                    ry + 5.0,
                    0.80,
                    accent,
                    true,
                );

                let tx = ix + 34.0;
                let avail = (ix + list_w - rtw - 12.0 - tx).max(30.0);
                let (sn_fit, ss) = self.fit_text_width(&srv.name, avail, 0.94);
                batch.text_scaled(&self.gui.atlas, &sn_fit, tx, ry + 5.0, ss, TEXT_WHITE, true);
                let (m_fit, ms) =
                    self.fit_text_width(&srv.motd.replace('\n', " "), iw - 42.0, 0.76);
                batch.text_scaled(&self.gui.atlas, &m_fit, tx, ry + 16.0, ms, TEXT_MUTED, true);
            }
        }
        batch.clip_since(clip_start, ix, list_y, ix + list_w, list_y + list_h);

        if max_scroll > 0.0 {
            let sb_x = ix + iw - 4.0;
            batch.fill(
                &self.gui.atlas,
                sb_x,
                list_y,
                3.0,
                list_h,
                rgba(18, 18, 20, 220),
            );
            let thumb_h = ((list_h / content_h) * list_h).clamp(16.0, list_h);
            let thumb_y =
                list_y + (self.shulkds_title.server_scroll / max_scroll) * (list_h - thumb_h);
            batch.fill(
                &self.gui.atlas,
                sb_x,
                thumb_y,
                3.0,
                thumb_h,
                rgba(164, 164, 170, 255),
            );
        }

        // Dock: [Join] [Direct] [Edit] [Del]
        let has_sel = sel.is_some();
        let saved_sel = sel.is_some_and(|i| i < saved_count);
        let d_gap = 4.0;
        let small_w = ((iw - d_gap * 3.0) * 0.19).floor().clamp(38.0, 56.0);
        let join_w = (iw - small_w * 3.0 - d_gap * 3.0).max(56.0);
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, dock_y, join_w, dock_h],
            "Join Server",
            Some(ACCENT_GREEN),
            has_sel,
            false,
            HitAct::MpButton(40),
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + join_w + d_gap, dock_y, small_w, dock_h],
            "Direct",
            Some(ACCENT_BLUE),
            true,
            false,
            HitAct::MpButton(41),
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [
                ix + join_w + d_gap + small_w + d_gap,
                dock_y,
                small_w,
                dock_h,
            ],
            "Edit",
            None,
            saved_sel,
            false,
            HitAct::MpButton(43),
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [
                ix + join_w + d_gap + (small_w + d_gap) * 2.0,
                dock_y,
                small_w,
                dock_h,
            ],
            "Del",
            Some(ACCENT_RED),
            saved_sel,
            false,
            HitAct::MpButton(44),
        );
    }

    fn draw_bottom_title_warning(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        bw: f32,
        bh: f32,
    ) {
        let checked = self
            .title
            .as_ref()
            .is_some_and(|t| matches!(t.mode, Mode::MultiplayerWarning { check: true }));
        let pad = 12.0;
        let ix = bx + pad;
        let iw = (bw - pad * 2.0).max(80.0);
        let mid_y = by + bh * 0.5 - 26.0;
        let chk_lbl = if checked {
            "[x] Do not show this screen again"
        } else {
            "[ ] Do not show this screen again"
        };
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, mid_y, iw, 20.0],
            chk_lbl,
            Some(ACCENT_AMBER),
            true,
            checked,
            HitAct::WarningCheck,
        );
        let btn_w = ((iw - 6.0) * 0.5).floor();
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, mid_y + 28.0, btn_w, 20.0],
            "Proceed",
            Some(ACCENT_GREEN),
            true,
            false,
            HitAct::WarningProceed,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix + btn_w + 6.0, mid_y + 28.0, iw - btn_w - 6.0, 20.0],
            "Back",
            Some(ACCENT_RED),
            true,
            false,
            HitAct::WarningBack,
        );
    }

    fn draw_bottom_title_options(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        bw: f32,
        bh: f32,
    ) {
        let pad = 8.0;
        let ix = bx + pad;
        let iw = (bw - pad * 2.0).max(80.0);
        batch.text_scaled(
            &self.gui.atlas,
            "OPTIONS QUICK-JUMP",
            ix + 2.0,
            by + 10.0,
            1.0,
            ACCENT_SILVER,
            true,
        );

        let cur = self.ui.options.page();
        let dock_h = 18.0;
        let dock_y = by + bh - dock_h - 8.0;
        let grid_y0 = by + 26.0;
        let avail_h = (dock_y - 7.0 - grid_y0).max(76.0);
        let gap = 5.0;
        let col_w = ((iw - gap) * 0.5).floor();
        let col2_w = iw - col_w - gap;
        let row_h = ((avail_h - gap * 3.0) / 4.0).floor().clamp(16.0, 34.0);
        let grid_total_h = row_h * 4.0 + gap * 3.0;
        let grid_y = grid_y0 + ((avail_h - grid_total_h) * 0.5).floor().max(0.0);

        let pages = [
            ("Video Settings", Page::Video, ACCENT_BLUE),
            ("Music & Sounds", Page::Sound, ACCENT_AMBER),
            ("Controls", Page::Controls, ACCENT_GREEN),
            ("Language", Page::Language, ACCENT_SILVER),
            ("Resource Packs", Page::Packs, ACCENT_AMBER),
            ("Mods", Page::Mods, ACCENT_VIOLET),
            ("Horizon / ShulkDS", Page::Horizon, ACCENT_GREEN),
            ("Extras", Page::Extras, ACCENT_BLUE),
        ];
        for (i, &(lbl, page, accent)) in pages.iter().enumerate() {
            let r = i / 2;
            let c = i % 2;
            let px = ix + if c == 0 { 0.0 } else { col_w + gap };
            let pw = if c == 0 { col_w } else { col2_w };
            let py = grid_y + r as f32 * (row_h + gap);
            self.draw_title_pill_btn(
                batch,
                bx,
                by,
                [px, py, pw, row_h],
                lbl,
                Some(accent),
                true,
                cur == page,
                HitAct::OptionsPage(page),
            );
        }
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, dock_y, iw, dock_h],
            "Done / Back",
            Some(ACCENT_GREEN),
            true,
            false,
            HitAct::OptionsDone,
        );
    }

    fn draw_bottom_title_modal(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        bw: f32,
        bh: f32,
    ) {
        let name = match self.title.as_ref().map(|t| &t.mode) {
            Some(Mode::Edit { .. }) => "Edit World",
            Some(Mode::Delete { .. }) => "Delete World",
            Some(Mode::Account(_)) => "Account & Profile",
            Some(Mode::CreateFlat { .. }) => "Superflat Customization",
            Some(Mode::CreateBuffet { .. }) => "Single Biome Picker",
            Some(Mode::Multiplayer(MpScreen::Edit { .. })) => "Server Details",
            Some(Mode::Multiplayer(MpScreen::Direct { .. })) => "Direct Connection",
            Some(Mode::Multiplayer(MpScreen::Friend { .. })) => "Join a Friend",
            Some(Mode::Multiplayer(MpScreen::ConfirmDelete { .. })) => "Delete Server",
            Some(Mode::Multiplayer(MpScreen::Disconnected { .. })) => "Disconnected",
            _ => "Menu Dialog",
        };
        let pad = 12.0;
        let ix = bx + pad;
        let iw = (bw - pad * 2.0).max(80.0);
        let ch = 64.0;
        let cy = by + (bh - ch - 34.0) * 0.5;
        self.draw_stone_panel(batch, [ix, cy, iw, ch]);
        let nw = HudBatch::text_width(&self.gui.atlas, name);
        batch.text(
            &self.gui.atlas,
            name,
            ix + (iw - nw) * 0.5,
            cy + 18.0,
            TEXT_WHITE,
            true,
        );
        let sub = "Complete edits on the top screen";
        let sw = HudBatch::text_width(&self.gui.atlas, sub) * 0.85;
        batch.text_scaled(
            &self.gui.atlas,
            sub,
            ix + (iw - sw) * 0.5,
            cy + 36.0,
            0.85,
            TEXT_MUTED,
            true,
        );
        self.draw_title_pill_btn(
            batch,
            bx,
            by,
            [ix, by + bh - 28.0, iw, 18.0],
            "< Back",
            Some(ACCENT_SILVER),
            true,
            false,
            HitAct::ModalBack,
        );
    }

    // -------------------------------------------------------------------------
    // Bottom Pointer Input & Actuation
    // -------------------------------------------------------------------------

    pub(super) fn handle_shulkds_title_pointer(
        &mut self,
        norm_x: f32,
        norm_y: f32,
        action: BottomPointerAction,
    ) {
        let rect = self.shulkds_bottom_gui_rect();
        let lx = norm_x.clamp(0.0, 1.0) * rect.width;
        let ly = norm_y.clamp(0.0, 1.0) * rect.height;
        self.shulkds_title.pointer_pos = Some((lx, ly));

        match action {
            BottomPointerAction::Down {
                button: PointerButton::Primary,
            } => {
                self.shulkds_title.pointer_down = true;
                self.shulkds.dashboard.pointer_down = true;
                self.shulkds_title.drag_moved = false;
                self.shulkds_title.drag_last_y = ly;
                self.shulkds_title.drag_start_y = self.shulkds_title.scroll_rect.and_then(|sr| {
                    (lx >= sr[0] && lx <= sr[0] + sr[2] && ly >= sr[1] && ly <= sr[1] + sr[3])
                        .then_some(ly)
                });
            }
            BottomPointerAction::Down { .. } => {}
            BottomPointerAction::Move => {
                if self.shulkds_title.pointer_down
                    && let Some(start_y) = self.shulkds_title.drag_start_y
                {
                    let dy = ly - self.shulkds_title.drag_last_y;
                    self.shulkds_title.drag_last_y = ly;
                    if (ly - start_y).abs() > 6.0 {
                        self.shulkds_title.drag_moved = true;
                    }
                    if self.shulkds_title.drag_moved {
                        let max = self.shulkds_title.max_scroll;
                        match self.current_title_kind() {
                            Some(BottomTitleKind::Select) => {
                                self.shulkds_title.world_scroll =
                                    (self.shulkds_title.world_scroll - dy).clamp(0.0, max);
                            }
                            Some(BottomTitleKind::MultiplayerList) => {
                                self.shulkds_title.server_scroll =
                                    (self.shulkds_title.server_scroll - dy).clamp(0.0, max);
                            }
                            _ => {}
                        }
                    }
                }
            }
            BottomPointerAction::Up {
                button: PointerButton::Primary,
            } => {
                let was_down = self.shulkds_title.pointer_down;
                let was_drag = self.shulkds_title.drag_moved;
                self.shulkds_title.pointer_down = false;
                self.shulkds.dashboard.pointer_down = false;
                self.shulkds_title.drag_start_y = None;
                self.shulkds_title.drag_moved = false;
                if was_down && !was_drag {
                    let hit = self.shulkds_title.hits.iter().position(|h| {
                        h.enabled
                            && lx >= h.rect[0]
                            && lx <= h.rect[0] + h.rect[2]
                            && ly >= h.rect[1]
                            && ly <= h.rect[1] + h.rect[3]
                    });
                    if let Some(idx) = hit {
                        let act = self.shulkds_title.hits[idx].act;
                        if !matches!(act, HitAct::CreateFocusField(_)) {
                            self.dismiss_soft_keyboard();
                        }
                        self.shulkds_title.focus_idx = self.shulkds_title.hits[..idx]
                            .iter()
                            .filter(|h| h.focusable)
                            .count();
                        self.execute_shulkds_title_hit(act);
                    } else {
                        self.dismiss_soft_keyboard();
                    }
                }
            }
            BottomPointerAction::Up { .. } => {
                self.shulkds_title.pointer_down = false;
                self.shulkds.dashboard.pointer_down = false;
                self.shulkds_title.drag_start_y = None;
                self.shulkds_title.drag_moved = false;
            }
            BottomPointerAction::Scroll { delta_y } => {
                let max = self.shulkds_title.max_scroll;
                match self.current_title_kind() {
                    Some(BottomTitleKind::Select) => {
                        self.shulkds_title.world_scroll =
                            (self.shulkds_title.world_scroll - delta_y * 18.0).clamp(0.0, max);
                    }
                    Some(BottomTitleKind::MultiplayerList) => {
                        self.shulkds_title.server_scroll =
                            (self.shulkds_title.server_scroll - delta_y * 18.0).clamp(0.0, max);
                    }
                    _ => {}
                }
            }
        }
    }

    fn execute_shulkds_title_hit(&mut self, act: HitAct) {
        self.audio.play_button_click();
        match act {
            HitAct::MainSingleplayer => {
                if let Some(title) = self.title.as_mut() {
                    title.enter_singleplayer();
                }
            }
            HitAct::MainMultiplayer => {
                if let Some(title) = self.title.as_mut() {
                    title.open_multiplayer();
                }
            }
            HitAct::MainFriends => {
                if let Some(title) = self.title.as_mut() {
                    title.open_friends();
                }
            }
            HitAct::MainOptions => self.shulkds_title.pending_action = Some(Action::Options),
            HitAct::MainLanguage => self.shulkds_title.pending_action = Some(Action::Language),
            HitAct::MainAccess => self.shulkds_title.pending_action = Some(Action::Accessibility),
            HitAct::MainCredits => self.shulkds_title.pending_action = Some(Action::Credits),
            HitAct::MainQuit => self.shulkds_title.pending_action = Some(Action::Quit),
            HitAct::MainProfile => {
                if accounts::in_game() {
                    if let Some(title) = self.title.as_mut() {
                        title.open_account();
                    }
                } else {
                    self.open_options_at(true, Page::Skin);
                }
            }
            HitAct::BackToMain => {
                if let Some(title) = self.title.as_mut() {
                    title.mode = Mode::Main;
                }
            }
            HitAct::SelectRow(idx) => {
                let mut should_play = false;
                if let Some(title) = self.title.as_mut()
                    && let Mode::Select { worlds, selected } = &mut title.mode
                {
                    if *selected == Some(idx) && worlds.get(idx).is_some_and(WorldEntry::can_play) {
                        should_play = true;
                    } else if idx < worlds.len() {
                        *selected = Some(idx);
                    }
                }
                if should_play && let Some(title) = self.title.as_mut() {
                    let action = title.play_selected_world();
                    if action != Action::None {
                        self.shulkds_title.pending_action = Some(action);
                    }
                }
            }
            HitAct::SelectPlay => {
                if let Some(title) = self.title.as_mut() {
                    let action = title.play_selected_world();
                    if action != Action::None {
                        self.shulkds_title.pending_action = Some(action);
                    }
                }
            }
            HitAct::SelectCreate => {
                if let Some(title) = self.title.as_mut() {
                    title.begin_create_world();
                }
            }
            HitAct::SelectEdit => {
                if let Some(title) = self.title.as_mut() {
                    title.edit_selected_world();
                }
            }
            HitAct::SelectCopy => {
                if let Some(title) = self.title.as_mut() {
                    title.recreate_selected_world();
                }
            }
            HitAct::SelectDelete => {
                if let Some(title) = self.title.as_mut() {
                    title.delete_selected_world();
                }
            }
            HitAct::CreateTab(t) => {
                if let Some(title) = self.title.as_mut() {
                    title.select_create_tab_index(t);
                }
            }
            HitAct::CreateFocusField(f) => {
                if let Some(title) = self.title.as_mut() {
                    title.set_create_focus(f);
                }
                self.request_field_soft_keyboard();
            }
            HitAct::CreateMode(cre, hc) => {
                if let Some(title) = self.title.as_mut() {
                    title.set_create_game_mode(cre, hc);
                }
            }
            HitAct::CreateDifficulty(d) => {
                if let Some(title) = self.title.as_mut() {
                    title.set_create_difficulty(d);
                }
            }
            HitAct::CreateButton(id) => {
                if let Some(title) = self.title.as_mut() {
                    let action = title.trigger_create_button(id);
                    if action != Action::None {
                        self.shulkds_title.pending_action = Some(action);
                    }
                }
            }
            HitAct::MpRow(idx) => {
                let mut should_join = false;
                if let Some(title) = self.title.as_mut()
                    && let Mode::Multiplayer(screen) = &mut title.mode
                {
                    if screen.selected_row() == Some(idx) {
                        should_join = true;
                    } else {
                        screen.set_selected_row(Some(idx));
                    }
                }
                if should_join && let Some(title) = self.title.as_mut() {
                    let saves = title.saves.clone();
                    if let Mode::Multiplayer(screen) = &mut title.mode
                        && let MpOutcome::Connect { address, lan } =
                            screen.trigger_list_button(40, &saves)
                    {
                        self.shulkds_title.pending_action = Some(Action::Connect { address, lan });
                    }
                }
            }
            HitAct::MpButton(id) => {
                if let Some(title) = self.title.as_mut() {
                    let saves = title.saves.clone();
                    if let Mode::Multiplayer(screen) = &mut title.mode {
                        match screen.trigger_list_button(id, &saves) {
                            MpOutcome::Connect { address, lan } => {
                                self.shulkds_title.pending_action =
                                    Some(Action::Connect { address, lan });
                            }
                            MpOutcome::Back => title.mode = Mode::Main,
                            MpOutcome::None => {}
                        }
                    }
                }
            }
            HitAct::WarningCheck => {
                if let Some(title) = self.title.as_mut()
                    && let Mode::MultiplayerWarning { check } = &mut title.mode
                {
                    *check = !*check;
                }
            }
            HitAct::WarningProceed => {
                if let Some(title) = self.title.as_mut() {
                    title.trigger_warning_button(70);
                }
            }
            HitAct::WarningBack => {
                if let Some(title) = self.title.as_mut() {
                    title.trigger_warning_button(71);
                }
            }
            HitAct::OptionsPage(page) => {
                self.open_options_at(true, page);
            }
            HitAct::OptionsDone => {
                let _ = self.options_key(KeyCode::Escape, None);
            }
            HitAct::ModalBack => {
                if let Some(title) = self.title.as_mut() {
                    title.escape();
                }
            }
            HitAct::UpdateAction(idx) => {
                self.shulkds_update_act(idx as usize);
            }
        }
    }

    fn draw_bottom_title_update(
        &mut self,
        batch: &mut HudBatch,
        bx: f32,
        by: f32,
        bw: f32,
        bh: f32,
        upd: &ShulkdsUpdateView,
    ) {
        let card_w = (bw - 24.0).clamp(160.0, 290.0);
        let pad = 12.0;
        let inner_w = card_w - pad * 2.0;
        let msg_text = horizon_client::text::Text::literal(upd.body.clone());
        let body_lines: Vec<String> = self
            .gui
            .atlas
            .font
            .split(&msg_text, inner_w / 0.82)
            .into_iter()
            .take(5)
            .map(|l| l.plain())
            .collect();
        let bar_h = if upd.progress.is_some() { 14.0 } else { 0.0 };
        let btn_h = 22.0;
        let card_h = (pad + 16.0 + body_lines.len() as f32 * 10.0 + 10.0 + bar_h + btn_h + pad)
            .clamp(96.0, bh - 16.0);
        let cx = bx + ((bw - card_w) * 0.5).floor();
        let cy = by + ((bh - card_h) * 0.5).floor();
        self.draw_stone_panel(batch, [cx, cy, card_w, card_h]);
        batch.fill(
            &self.gui.atlas,
            cx + 2.0,
            cy + 2.0,
            card_w - 4.0,
            2.0,
            ACCENT_GREEN,
        );

        let (t_fit, ts) = self.fit_text_width(&upd.stage_title, inner_w, 1.0);
        let tw = HudBatch::text_width(&self.gui.atlas, &t_fit) * ts;
        batch.text_scaled(
            &self.gui.atlas,
            &t_fit,
            cx + (card_w - tw) * 0.5,
            cy + 9.0,
            ts,
            TEXT_WHITE,
            true,
        );

        let mut ly = cy + 24.0;
        for line in &body_lines {
            batch.text_scaled(&self.gui.atlas, line, cx + pad, ly, 0.82, TEXT_MUTED, true);
            ly += 10.0;
        }
        if let Some(frac) = upd.progress {
            let py = ly + 4.0;
            batch.fill(
                &self.gui.atlas,
                cx + pad,
                py,
                inner_w,
                6.0,
                rgba(20, 22, 26, 255),
            );
            if frac > 0.0 {
                batch.fill(
                    &self.gui.atlas,
                    cx + pad,
                    py,
                    (inner_w * frac).max(6.0),
                    6.0,
                    ACCENT_GREEN,
                );
            }
        }

        let btn_y = cy + card_h - pad - btn_h;
        let n = upd.buttons.len().max(1);
        let gap = 6.0;
        let each_w = ((inner_w - gap * (n as f32 - 1.0)) / n as f32).floor();
        for (i, label) in upd.buttons.iter().enumerate() {
            let x = cx + pad + i as f32 * (each_w + gap);
            let w = if i + 1 == n {
                cx + pad + inner_w - x
            } else {
                each_w
            };
            let accent = if i == 0 && n > 1 {
                Some(ACCENT_GREEN)
            } else if i == 1 && n == 3 {
                Some(ACCENT_AMBER)
            } else {
                Some(ACCENT_RED)
            };
            self.draw_title_pill_btn(
                batch,
                bx,
                by,
                [x, btn_y, w, btn_h],
                label,
                accent,
                true,
                false,
                HitAct::UpdateAction(i as u8),
            );
        }
    }

    pub(super) fn trigger_check_updates(&mut self) {
        #[cfg(target_os = "android")]
        self.check_updates_now();
        #[cfg(not(target_os = "android"))]
        {
            self.shulkds
                .dashboard
                .set_status("Updates require an Android build");
        }
    }

    #[cfg(not(target_os = "android"))]
    pub(super) fn shulkds_update_modal_view(&self) -> Option<ShulkdsUpdateView> {
        None
    }

    #[cfg(not(target_os = "android"))]
    pub(super) fn shulkds_update_sig(&self) -> u64 {
        0
    }

    #[cfg(not(target_os = "android"))]
    pub(super) fn shulkds_update_act(&mut self, _button: usize) {}

    pub(super) fn shulkds_title_pad_button(
        &mut self,
        _event_loop: &dyn EventLoopCtl,
        button: Button,
    ) -> Result<bool> {
        let Some(kind) = self.current_title_kind() else {
            return Ok(false);
        };
        let mut focusables: Vec<(usize, [f32; 4], HitAct)> = Vec::new();
        let mut ord = 0usize;
        for h in &self.shulkds_title.hits {
            if h.focusable {
                if h.enabled {
                    focusables.push((ord, h.rect, h.act));
                }
                ord += 1;
            }
        }
        match button {
            Button::East => {
                if let Some(upd) = self.shulkds_update_modal_view() {
                    let cancel_idx = upd.buttons.len().saturating_sub(1);
                    self.shulkds_update_act(cancel_idx);
                    return Ok(true);
                }
                if self.dismiss_soft_keyboard() {
                    return Ok(true);
                }
                match kind {
                    BottomTitleKind::Main => {}
                    BottomTitleKind::Create => {
                        self.execute_shulkds_title_hit(HitAct::CreateButton(22))
                    }
                    _ => self.execute_shulkds_title_hit(HitAct::ModalBack),
                }
                Ok(true)
            }
            Button::LeftBumper if kind == BottomTitleKind::Create => {
                if let Some(title) = self.title.as_mut()
                    && let Mode::Create { options, .. } = &title.mode
                {
                    let prev = (options.tab + 2) % 3;
                    title.select_create_tab_index(prev);
                    self.audio.play_button_click();
                }
                Ok(true)
            }
            Button::RightBumper if kind == BottomTitleKind::Create => {
                if let Some(title) = self.title.as_mut()
                    && let Mode::Create { options, .. } = &title.mode
                {
                    let next = (options.tab + 1) % 3;
                    title.select_create_tab_index(next);
                    self.audio.play_button_click();
                }
                Ok(true)
            }
            Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight => {
                if focusables.is_empty() {
                    return Ok(true);
                }
                let Some(cur_pos) = focusables
                    .iter()
                    .position(|(o, _, _)| *o == self.shulkds_title.focus_idx)
                else {
                    self.shulkds_title.focus_idx = focusables[0].0;
                    self.audio.play_button_click();
                    return Ok(true);
                };
                let (_, cur_rect, cur_act) = focusables[cur_pos];

                if matches!(button, Button::DPadUp | Button::DPadDown) {
                    let list_h = self.shulkds_title.scroll_rect.map_or(120.0, |sr| sr[3]);
                    let max_scroll = self.shulkds_title.max_scroll;
                    match cur_act {
                        HitAct::SelectRow(idx) if kind == BottomTitleKind::Select => {
                            let total = self.title.as_ref().map_or(0, |t| match &t.mode {
                                Mode::Select { worlds, .. } => worlds.len(),
                                _ => 0,
                            });
                            let step = if button == Button::DPadDown && idx + 1 < total {
                                Some(idx + 1)
                            } else if button == Button::DPadUp && idx > 0 {
                                Some(idx - 1)
                            } else {
                                None
                            };
                            if let Some(next_idx) = step {
                                if let Some(title) = self.title.as_mut()
                                    && let Mode::Select { selected, .. } = &mut title.mode
                                {
                                    *selected = Some(next_idx);
                                }
                                let row_top = next_idx as f32 * 34.0;
                                let row_bot = row_top + 30.0;
                                if row_bot > self.shulkds_title.world_scroll + list_h {
                                    self.shulkds_title.world_scroll =
                                        (row_bot - list_h).clamp(0.0, max_scroll);
                                } else if row_top < self.shulkds_title.world_scroll {
                                    self.shulkds_title.world_scroll =
                                        row_top.clamp(0.0, max_scroll);
                                }
                                self.shulkds_title.pending_row_focus = Some(next_idx);
                                self.audio.play_button_click();
                                return Ok(true);
                            }
                        }
                        HitAct::MpRow(idx) if kind == BottomTitleKind::MultiplayerList => {
                            let total = self.title.as_ref().map_or(0, |t| match &t.mode {
                                Mode::Multiplayer(screen) => screen.list_summaries().len(),
                                _ => 0,
                            });
                            let step = if button == Button::DPadDown && idx + 1 < total {
                                Some(idx + 1)
                            } else if button == Button::DPadUp && idx > 0 {
                                Some(idx - 1)
                            } else {
                                None
                            };
                            if let Some(next_idx) = step {
                                if let Some(title) = self.title.as_mut()
                                    && let Mode::Multiplayer(screen) = &mut title.mode
                                {
                                    screen.set_selected_row(Some(next_idx));
                                }
                                let row_top = next_idx as f32 * 32.0;
                                let row_bot = row_top + 28.0;
                                if row_bot > self.shulkds_title.server_scroll + list_h {
                                    self.shulkds_title.server_scroll =
                                        (row_bot - list_h).clamp(0.0, max_scroll);
                                } else if row_top < self.shulkds_title.server_scroll {
                                    self.shulkds_title.server_scroll =
                                        row_top.clamp(0.0, max_scroll);
                                }
                                self.shulkds_title.pending_row_focus = Some(next_idx);
                                self.audio.play_button_click();
                                return Ok(true);
                            }
                        }
                        _ => {}
                    }
                }

                let rects: Vec<[f32; 4]> = focusables.iter().map(|(_, r, _)| *r).collect();
                if let Some(next_pos) =
                    horizon_client::gamepad::focus_step_rect(cur_rect, button, &rects)
                {
                    let (next_ord, _, next_act) = focusables[next_pos];
                    self.shulkds_title.focus_idx = next_ord;
                    match next_act {
                        HitAct::SelectRow(idx) => {
                            if let Some(title) = self.title.as_mut()
                                && let Mode::Select { worlds, selected } = &mut title.mode
                                && idx < worlds.len()
                            {
                                *selected = Some(idx);
                            }
                        }
                        HitAct::MpRow(idx) => {
                            if let Some(title) = self.title.as_mut()
                                && let Mode::Multiplayer(screen) = &mut title.mode
                            {
                                screen.set_selected_row(Some(idx));
                            }
                        }
                        _ => {}
                    }
                    self.audio.play_button_click();
                }
                Ok(true)
            }
            Button::South => {
                let target = focusables
                    .iter()
                    .find(|(o, _, _)| *o == self.shulkds_title.focus_idx)
                    .map(|(_, _, act)| *act);
                if let Some(act) = target {
                    self.execute_shulkds_title_hit(act);
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
