//! The pause menu (PauseScreen) and death screen (DeathScreen) layouts.
use super::{Game, Screen};
use horizon_client::{
    Result,
    confirm_screen::Confirm,
    dialog::Dialog,
    hud::{HudBatch, rgba},
    text::{self, Text},
    ui,
    widgets::{self, ScreenInfo},
};
use horizon_world::nbt::Tag;
use winit::keyboard::KeyCode;

impl Screen {
    /// Screen.extractBackground for non-isInGameUi screens, and
    /// PauseScreen.extractBackground only when showPauseMenu is true.
    pub(super) fn blurs_background(self, pause_menu: bool) -> bool {
        match self {
            Self::Paused => pause_menu,
            Self::Options
            | Self::Stats
            | Self::Advancements
            | Self::WorldOptions
            | Self::Friends
            | Self::Social
            | Self::ServerLinks
            | Self::ConfirmLink
            | Self::DialogWarning
            | Self::DebugOptions => true,
            Self::Playing
            | Self::GameModeSwitcher
            | Self::Chat
            | Self::Creative
            | Self::Inventory
            | Self::Dead
            | Self::CommandBlock
            | Self::BlockEditor
            | Self::Mod => false,
            // WinScreen.extractBackground: the poem fills the screen with the End portal, the
            // credits alone blur. Game::menu_blur knows which one rolls.
            Self::Win => false,
        }
    }
}

/// Compute the centered pause menu origin, title Y, first-button Y, and row step
/// within `(ox, oy, w, h)` (top screen in ShulkDS mode).
fn pause_metrics_at(ox: f32, oy: f32, w: f32, h: f32, extra_rows: u8) -> (f32, f32, f32, f32) {
    let total_rows = 4.0 + f32::from(extra_rows);
    let compact = h < 130.0 + 24.0 * f32::from(extra_rows);
    let title_gap = if compact { 14.0 } else { 18.0 };
    let step = if compact {
        ((h - 10.0 - title_gap - 20.0) / (total_rows - 1.0).max(1.0))
            .floor()
            .clamp(20.0, 24.0)
    } else {
        24.0
    };
    let block_h = title_gap + (total_rows - 1.0) * step + 20.0;
    let title_y = oy + ((h - block_h) * 0.5).floor().max(4.0);
    let y_start = title_y + title_gap;
    let x0 = ox + ((w - 212.0) * 0.5).floor();
    (x0, title_y, y_start, step)
}

/// PauseScreen buttons: (x, y, width, label key, action id, active).
/// `links`: the `minecraft:server_links` dialog's button, which
/// `PauseScreen.getCustomAdditions` falls back to when `pause_screen_additions`
/// is empty and the server sent links — a full-width row that pushes the last
/// two rows down.
/// `additions`: the `pause_screen_additions` button instead of the links one
/// (vanilla shows one or the other, never both); its label is the dialog's
/// external title, so the caller draws it (id 14).
fn pause_buttons_at(
    ox: f32,
    oy: f32,
    w: f32,
    h: f32,
    remote: bool,
    links: bool,
    additions: bool,
    split: bool,
) -> Vec<(f32, f32, f32, &'static str, u8, bool)> {
    let row3 = links || additions;
    let extra_rows = u8::from(row3) + u8::from(split);
    let (x0, _, y_start, step) = pause_metrics_at(ox, oy, w, h, extra_rows);
    let down = step * f32::from(extra_rows);
    let mut v = vec![
        (x0 + 4.0, y_start, 204.0, "menu.returnToGame", 0, true),
        (x0 + 4.0, y_start + step, 98.0, "gui.advancements", 4, true),
        (x0 + 110.0, y_start + step, 98.0, "gui.stats", 3, true),
    ];
    if additions {
        // addCustomDialogButtons: one full-width button, labelled with the
        // dialog's CommonDialogData.computeExternalTitle.
        v.push((x0 + 4.0, y_start + step * 2.0, 204.0, "", 14, true));
    } else if links {
        // CommonDialogData.computeExternalTitle of Dialogs.SERVER_LINKS.
        v.push((x0 + 4.0, y_start + step * 2.0, 204.0, "menu.server_links", 11, true));
    }
    if split {
        let y = y_start + step * 2.0 + if row3 { step } else { 0.0 };
        v.push((x0 + 4.0, y, 204.0, SPLIT_SETTINGS, 30, true));
    }
    let opt_y = y_start + step * 2.0 + down;
    v.extend([
        (x0 + 4.0, opt_y, 98.0, "menu.options", 1, true),
        (
            x0 + 110.0,
            opt_y,
            98.0,
            "options.worldOptions.button",
            9,
            true,
        ),
        (
            x0 + 4.0,
            opt_y + step,
            204.0,
            if remote {
                "menu.disconnect"
            } else {
                "menu.returnToMenu"
            },
            2,
            true,
        ),
    ]);
    v
}

#[allow(dead_code)]
fn pause_buttons(
    w: f32,
    h: f32,
    remote: bool,
    links: bool,
    additions: bool,
    split: bool,
) -> Vec<(f32, f32, f32, &'static str, u8, bool)> {
    pause_buttons_at(0.0, 0.0, w, h, remote, links, additions, split)
}

const SPLIT_SETTINGS: &str = "horizon.pages.split_settings";
/// DeathScreen.init: a hardcore player can only spectate.
/// `DeathScreen.TitleConfirmScreen`: its title and the yes and no labels.
fn death_confirm_texts() -> (Text, [Text; 2]) {
    let t = |key: &str| Text::translate(key, &[]);
    (
        t("deathScreen.quit.confirm"),
        [t("deathScreen.titleScreen"), t("deathScreen.respawn")],
    )
}
fn legacy_pause_buttons_at(
    ox: f32,
    oy: f32,
    w: f32,
    h: f32,
    remote: bool,
) -> Vec<(f32, f32, f32, &'static str, u8, bool)> {
    let rows: &[(&'static str, u8)] = &[
        ("menu.returnToGame", 0),
        ("menu.options", 1),
        ("gui.stats", 3),
        ("gui.advancements", 4),
        ("options.worldOptions.button", 9),
        (
            if remote {
                "menu.disconnect"
            } else {
                "menu.returnToMenu"
            },
            2,
        ),
    ];
    let step = if h < 165.0 { 22.0 } else { 25.0 };
    let total_h = (rows.len() - 1) as f32 * step + 20.0;
    let x0 = ox + ((w - 200.0) * 0.5).floor();
    let y0 = oy + ((h - total_h) * 0.5).floor().max(6.0);
    rows.iter()
        .enumerate()
        .map(|(i, &(key, id))| (x0, y0 + i as f32 * step, 200.0, key, id, true))
        .collect()
}

fn death_buttons_at(
    ox: f32,
    oy: f32,
    w: f32,
    h: f32,
    hardcore: bool,
    legacy: bool,
) -> [(f32, f32, &'static str, u8); 2] {
    let respawn = if hardcore {
        "deathScreen.spectate"
    } else {
        "deathScreen.respawn"
    };
    let x = ox + (w.ceil() / 2.0).floor() - 100.0;
    if legacy {
        return [
            (x, oy + (h / 2.0 + 20.0).floor(), respawn, 0),
            (
                x,
                oy + (h / 2.0 + 45.0).floor(),
                "deathScreen.titleScreen",
                1,
            ),
        ];
    }
    // DeathScreen.init: `width / 2 - 100`, `height / 4 + 72` and `+ 96`, in
    // int GUI pixels.
    let y = oy + if h < 220.0 {
        ((h - 120.0) * 0.5).max(10.0).floor()
    } else {
        (h.ceil() / 4.0).floor()
    };
    [
        (x, y + 72.0, respawn, 0),
        (x, y + 96.0, "deathScreen.titleScreen", 1),
    ]
}

#[allow(dead_code)]
fn death_buttons(w: f32, h: f32, hardcore: bool) -> [(f32, f32, &'static str, u8); 2] {
    death_buttons_at(0.0, 0.0, w, h, hardcore, false)
}
impl Game<'_> {
    /// `PauseScreen.getCustomAdditions`'s additions: the
    /// `minecraft:pause_screen_additions` tag's one dialog, or the
    /// `minecraft:custom_options` `dialog_list` when it names several. `None`
    /// when the tag is empty: the caller falls back to the server links
    /// button, as vanilla does.
    fn pause_screen_additions(&self) -> Option<(Text, Tag)> {
        let registries = &self.remote.as_ref()?.registries;
        let dialogs = registries.registries.get("minecraft:dialog")?;
        let tag = registries
            .dialog_tags
            .get("minecraft:pause_screen_additions")?;
        // addCustomDialogButtons' label: computeExternalTitle falls back to the title.
        let button = |dialog_tag: Tag| -> Option<(Text, Tag)> {
            let dialog = Dialog::parse(&dialog_tag)?;
            let label = dialog.external_title.unwrap_or(dialog.title);
            Some((label, dialog_tag))
        };
        match tag.len() {
            0 => None,
            1 => {
                let i = usize::try_from(tag[0]).ok()?;
                button(dialogs.get(i)?.1.clone()?)
            }
            // `Dialogs.CUSTOM_OPTIONS`: the dialog list over the same tag.
            _ => button(
                dialogs
                    .iter()
                    .find(|(name, _)| name == "minecraft:custom_options")?
                    .1
                    .clone()?,
            ),
        }
    }

    /// Singleplayer also has a connection, backed by the integrated server.
    pub(super) fn external_server(&self) -> bool {
        self.remote.is_some() && self.integrated.is_none()
    }
    pub(super) fn draw_pause_menu(&mut self, batch: &mut HudBatch) {
        let legacy = self.use_legacy_gui();
        let atlas = &self.gui.atlas;
        let (ox, oy, w, h) = (0.0, 0.0, batch.width, batch.height);
        ui::menu_background(batch, atlas, ox, oy, w, h, true);
        let title = Text::translate("menu.game", &[]);
        widgets::begin_screen(|| ScreenInfo::new("pause", title.plain()));
        let tw = atlas.font.width_text(&title);
        let links = !self.server_links().is_empty();
        let additions = self.pause_screen_additions();
        let split = self.options.split_guest || self.split_hud.menu;
        let extra_rows = u8::from(links || additions.is_some()) + u8::from(split);
        if !legacy {
            let (_, title_y, _, _) = pause_metrics_at(ox, oy, w, h, extra_rows);
            batch.text_styled(
                atlas,
                &title,
                (ox + (w - tw) / 2.0).floor(),
                title_y,
                u32::MAX,
                true,
            );
        }
        let mut tooltip = None;
        let buttons = if legacy {
            legacy_pause_buttons_at(ox, oy, w, h, self.external_server())
        } else {
            pause_buttons_at(
                ox,
                oy,
                w,
                h,
                self.external_server(),
                links,
                additions.is_some(),
                split,
            )
        };
        for (x, y, bw, key, id, active) in buttons {
            let is_focused = self.pause_focus == Some(id);
            let hover = active && (ui::hovered(self.cursor, x, y, bw, 20.0) || is_focused);
            let label = match (id, &additions) {
                (14, Some((label, _))) => label.clone(),
                _ if self.options.split_guest && key == "menu.disconnect" => {
                    Text::translate("horizon.split.leave", &[])
                }
                _ => Text::translate(key, &[]),
            };
            ui::button(batch, atlas, x, y, bw, 20.0, &label, active, hover);
            // PauseScreen's customAdditions button: CUSTOM_OPTIONS_TOOLTIP.
            if (id == 11 || id == 14) && hover {
                tooltip = Some(Text::translate("menu.custom_options.tooltip", &[]));
            }
        }
        // Custom additions tooltip when hovered.
        if let Some(tip) = tooltip {
            let lines: Vec<Text> = atlas
                .font
                .split_lines(&tip, batch.width)
                .into_iter()
                .map(|(line, _)| line)
                .collect();
            ui::tooltip(batch, atlas, &lines, self.cursor);
        }
    }
    pub(super) fn execute_pause_action(&mut self, hit: u8) -> Result<()> {
        match hit {
            0 => {
                self.pause_focus = None;
                self.screen = Screen::Playing;
                self.capture(true);
            }
            1 => self.open_options(false),
            3 => self.open_stats(),
            4 => self.open_advancements(true),
            9 => self.open_world_options(),
            // DialogConnectionAccess.openDialog(Dialogs.SERVER_LINKS).
            11 => {
                self.links_ui.scroll = 0.0;
                self.screen = Screen::ServerLinks;
            }
            30 => self.open_split_settings(),
            // addCustomDialogButtons: DialogConnectionAccess.openDialog of the
            // additions dialog, from the pause menu.
            14 => {
                if let Some((_, dialog)) = self.pause_screen_additions() {
                    self.pause_focus = None;
                    self.screen = Screen::Playing;
                    self.capture(true);
                    self.show_dialog(&dialog);
                }
            }
            2 => self.close_world()?,
            _ => {}
        }
        Ok(())
    }
    pub(super) fn pause_click(&mut self) -> Result<()> {
        let (w, h) = self.gui_size();
        let (ox, oy) = (0.0, 0.0);
        let links = !self.server_links().is_empty();
        let additions = self.pause_screen_additions();
        let buttons = if self.use_legacy_gui() {
            legacy_pause_buttons_at(ox, oy, w, h, self.external_server())
        } else {
            pause_buttons_at(
                ox,
                oy,
                w,
                h,
                self.external_server(),
                links,
                additions.is_some(),
                self.options.split_guest || self.split_hud.menu,
            )
        };
        let hit = buttons
            .into_iter()
            .find(|b| b.5 && ui::hovered(self.cursor, b.0, b.1, b.2, 20.0))
            .map(|b| b.4);
        if let Some(id) = hit {
            self.pause_focus = Some(id);
            self.audio.play_button_click();
            self.execute_pause_action(id)?;
        }
        Ok(())
    }
    pub(super) fn pause_key(&mut self, key: KeyCode) -> Result<()> {
        let (w, h) = self.gui_size();
        let (ox, oy) = (0.0, 0.0);
        let links = !self.server_links().is_empty();
        let additions = self.pause_screen_additions().is_some();
        let split = self.options.split_guest || self.split_hud.menu;
        let buttons = if self.use_legacy_gui() {
            legacy_pause_buttons_at(ox, oy, w, h, self.external_server())
        } else {
            pause_buttons_at(
                ox,
                oy,
                w,
                h,
                self.external_server(),
                links,
                additions,
                split,
            )
        };
        let active_ids: Vec<u8> = buttons.iter().filter(|b| b.5).map(|b| b.4).collect();
        if active_ids.is_empty() {
            return Ok(());
        }
        match key {
            KeyCode::Tab => {
                let reverse = self.has_shift_down();
                let next = match self.pause_focus {
                    Some(cur) => {
                        if let Some(pos) = active_ids.iter().position(|&id| id == cur) {
                            if reverse {
                                if pos == 0 {
                                    active_ids.last().copied()
                                } else {
                                    Some(active_ids[pos - 1])
                                }
                            } else if pos + 1 >= active_ids.len() {
                                active_ids.first().copied()
                            } else {
                                Some(active_ids[pos + 1])
                            }
                        } else if reverse {
                            active_ids.last().copied()
                        } else {
                            active_ids.first().copied()
                        }
                    }
                    None => {
                        if reverse {
                            active_ids.last().copied()
                        } else {
                            active_ids.first().copied()
                        }
                    }
                };
                self.pause_focus = next;
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let rects: Vec<(u8, f32, f32, f32)> = buttons
                    .into_iter()
                    .filter(|b| b.5)
                    .map(|b| (b.4, b.0, b.1, b.2))
                    .collect();
                let cur_id = self.pause_focus.unwrap_or(0);
                if let Some(cur) = rects.iter().find(|r| r.0 == cur_id) {
                    let (cx, cy) = (cur.1 + cur.3 / 2.0, cur.2 + 10.0);
                    let mut best: Option<(u8, f32)> = None;
                    for r in &rects {
                        if r.0 == cur_id {
                            continue;
                        }
                        let (rx, ry) = (r.1 + r.3 / 2.0, r.2 + 10.0);
                        let (dx, dy) = (rx - cx, ry - cy);
                        let valid = match key {
                            KeyCode::ArrowUp => dy < -4.0,
                            KeyCode::ArrowDown => dy > 4.0,
                            KeyCode::ArrowLeft => dx < -4.0,
                            KeyCode::ArrowRight => dx > 4.0,
                            _ => false,
                        };
                        if !valid {
                            continue;
                        }
                        let dist = match key {
                            KeyCode::ArrowUp | KeyCode::ArrowDown => dy.abs() + dx.abs() * 2.0,
                            KeyCode::ArrowLeft | KeyCode::ArrowRight => dx.abs() + dy.abs() * 2.0,
                            _ => f32::MAX,
                        };
                        if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                            best = Some((r.0, dist));
                        }
                    }
                    if let Some((next_id, _)) = best {
                        self.pause_focus = Some(next_id);
                    }
                } else {
                    self.pause_focus = Some(active_ids[0]);
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                if let Some(hit) = self.pause_focus {
                    self.audio.play_button_click();
                    self.execute_pause_action(hit)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// DeathScreen: gradient, the title at double size, the cause and score.
    pub(super) fn draw_death_screen(&mut self, batch: &mut HudBatch) {
        let legacy = self.use_legacy_gui();
        let atlas = &self.gui.atlas;
        let (ox, oy, w, h) = (0.0, 0.0, batch.width, batch.height);
        if legacy {
            batch.fill(atlas, ox, oy, w, h, rgba(56, 8, 12, 160));
        } else {
            // fillGradient(0x60500000 -> 0xA0803030).
            let rows = h.ceil() as i32;
            for y in 0..rows {
                let t = y as f32 / h.max(1.0);
                let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
                batch.fill(
                    atlas,
                    ox,
                    oy + y as f32,
                    w,
                    1.0,
                    rgba(
                        lerp(80.0, 128.0),
                        lerp(0.0, 48.0),
                        lerp(0.0, 48.0),
                        lerp(96.0, 160.0),
                    ),
                );
            }
        }
        // DeathScreen.handleExitToTitleScreen: the TitleConfirmScreen takes the
        // screen over, on the same background.
        if let Some(opened) = self.ui.death_confirm {
            let (title, labels) = death_confirm_texts();
            // ConfirmScreen.getNarrationMessage: its title and its (empty)
            // message.
            widgets::begin_screen(|| {
                ScreenInfo::new(
                    "death/confirm",
                    widgets::join_for_narration(&[&title.plain(), ""]),
                )
            });
            let size = self.gui_size();
            let mut confirm = Confirm::new(&atlas.font, size, &title, &Text::literal(""));
            confirm.focus = self.death_focus.map(|f| f as usize);
            confirm.draw_active(
                batch,
                atlas,
                self.cursor,
                [&labels[0], &labels[1]],
                opened.elapsed().as_millis() >= 1000,
            );
            return;
        }
        let title = if self.level.hardcore {
            "deathScreen.title.hardcore"
        } else {
            "deathScreen.title"
        };
        let title = Text::translate(title, &[]);
        widgets::begin_screen(|| ScreenInfo::new("death", title.plain()));
        let tw = atlas.font.width_text(&title);
        let title_y = if legacy {
            oy + (h / 4.0 + 10.0)
                .floor()
                .min((h / 2.0 - 32.0).floor())
                .max(15.0)
        } else if h < 220.0 {
            oy + 15.0
        } else {
            oy + 60.0
        };
        ui::text_scaled(
            batch,
            atlas,
            &title,
            (ox + w / 2.0 - tw).floor(),
            title_y,
            2.0,
            u32::MAX,
        );
        if let Some(cause) = &self.ui.death_text {
            let cw = atlas.font.width_text(cause);
            let cause_y = if legacy {
                title_y + 24.0
            } else if h < 220.0 {
                oy + 38.0
            } else {
                oy + 85.0
            };
            batch.text_styled(atlas, cause, (ox + (w - cw) / 2.0).floor(), cause_y, u32::MAX, true);
        }
        if !legacy {
            let score = Text::translate(
                "deathScreen.score.value",
                &[Text::colored(
                    self.experience.total.to_string(),
                    text::YELLOW,
                )],
            );
            let sw = atlas.font.width_text(&score);
            let score_y = if h < 220.0 { oy + 52.0 } else { oy + 100.0 };
            batch.text_styled(
                atlas,
                &score,
                (ox + (w - sw) / 2.0).floor(),
                score_y,
                u32::MAX,
                true,
            );
        }
        let active = self
            .ui
            .died_at
            .is_none_or(|t| t.elapsed().as_millis() >= 1000);
        for (x, y, key, id) in death_buttons_at(ox, oy, w, h, self.level.hardcore, legacy) {
            let is_focused = self.death_focus == Some(id);
            let hover = active && (ui::hovered(self.cursor, x, y, 200.0, 20.0) || is_focused);
            ui::button(
                batch,
                atlas,
                x,
                y,
                200.0,
                20.0,
                &Text::translate(key, &[]),
                active,
                hover,
            );
        }
    }
    /// LocalPlayer.respawn: the player comes back (the death screen's
    /// button, or at once with `immediate_respawn`).
    pub(super) fn respawn_player(&mut self) -> Result<()> {
        self.death_focus = None;
        self.ui.death_text = None;
        // LocalPlayer.respawn: KeyMapping.resetToggleKeys.
        self.toggles.respawn();
        self.remote_respawn();
        Ok(())
    }
    pub(super) fn death_click(&mut self) -> Result<()> {
        if self
            .ui
            .died_at
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return Ok(());
        }
        if let Some(opened) = self.ui.death_confirm {
            // ConfirmScreen.setDelay: both buttons wait 20 ticks.
            if opened.elapsed().as_millis() < 1000 {
                return Ok(());
            }
            let (title, _) = death_confirm_texts();
            let confirm = Confirm::new(
                &self.gui.atlas.font,
                self.gui_size(),
                &title,
                &Text::literal(""),
            );
            if let Some(yes) = confirm.answer_at(self.cursor) {
                self.death_focus = Some(if yes { 0 } else { 1 });
                self.death_confirm_answer(yes)?;
            }
            return Ok(());
        }
        let (w, h) = self.gui_size();
        let (ox, oy) = (0.0, 0.0);
        let hit = death_buttons_at(ox, oy, w, h, self.level.hardcore, self.use_legacy_gui())
            .into_iter()
            .find(|d| ui::hovered(self.cursor, d.0, d.1, 200.0, 20.0))
            .map(|d| d.3);
        if let Some(id) = hit {
            self.death_focus = Some(id);
        }
        // handleExitToTitleScreen: outside hardcore, Title Screen asks first.
        if hit == Some(1) && !self.level.hardcore {
            self.ui.death_confirm = Some(std::time::Instant::now());
            self.audio.play_button_click();
            return Ok(());
        }
        self.death_action(hit)
    }
    pub(super) fn death_key(&mut self, key: KeyCode) -> Result<()> {
        if self
            .ui
            .died_at
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return Ok(());
        }
        if let Some(opened) = self.ui.death_confirm {
            if opened.elapsed().as_millis() < 1000 {
                return Ok(());
            }
            match key {
                KeyCode::Escape => {
                    self.audio.play_button_click();
                    self.death_confirm_answer(false)?;
                }
                KeyCode::Tab => {
                    let reverse = self.has_shift_down();
                    self.death_focus = match self.death_focus {
                        Some(0) => Some(1),
                        Some(1) => Some(0),
                        _ => {
                            if reverse {
                                Some(1)
                            } else {
                                Some(0)
                            }
                        }
                    };
                }
                KeyCode::ArrowLeft => self.death_focus = Some(0),
                KeyCode::ArrowRight => self.death_focus = Some(1),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    let yes = self.death_focus != Some(1);
                    self.audio.play_button_click();
                    self.death_confirm_answer(yes)?;
                }
                _ => {}
            }
            return Ok(());
        }
        match key {
            KeyCode::Tab => {
                let reverse = self.has_shift_down();
                self.death_focus = match self.death_focus {
                    Some(0) => Some(1),
                    Some(1) => Some(0),
                    _ => {
                        if reverse {
                            Some(1)
                        } else {
                            Some(0)
                        }
                    }
                };
            }
            KeyCode::ArrowUp => self.death_focus = Some(0),
            KeyCode::ArrowDown => self.death_focus = Some(1),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                let Some(hit) = self.death_focus else {
                    return Ok(());
                };
                self.audio.play_button_click();
                if hit == 1 && !self.level.hardcore {
                    self.ui.death_confirm = Some(std::time::Instant::now());
                    self.death_focus = Some(0);
                    return Ok(());
                }
                self.death_action(Some(hit))?;
            }
            _ => {}
        }
        Ok(())
    }
    /// The confirm screen's answer: yes leaves the world, no respawns
    /// (`DeathScreen.handleExitToTitleScreen`'s callback).
    pub(super) fn death_confirm_answer(&mut self, yes: bool) -> Result<()> {
        self.ui.death_confirm = None;
        if yes {
            self.audio.play_button_click();
            self.close_world()
        } else {
            self.death_action(Some(0))
        }
    }
    /// A death screen button: 0 respawns (or spectates in hardcore), 1 leaves.
    fn death_action(&mut self, hit: Option<u8>) -> Result<()> {
        match hit {
            Some(0) => self.respawn_player()?,
            Some(1) => self.close_world()?,
            _ => {}
        }
        if hit.is_some() {
            self.audio.play_button_click();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Screen, death_buttons, pause_buttons, pause_metrics_at};

    #[test]
    fn death_buttons_sit_on_whole_gui_pixels() {
        // DeathScreen.init in ints: a 427x253 GUI (1709x1009 at scale 4) puts
        // Respawn at (113, 63 + 72), not (113.5, 63.25 + 72).
        let at = |w, h| death_buttons(w, h, false).map(|b| (b.0, b.1));
        assert_eq!(at(427.0, 253.0), [(113.0, 135.0), (113.0, 159.0)]);
        assert_eq!(at(320.0, 334.0), [(60.0, 155.0), (60.0, 179.0)]);
    }

    #[test]
    fn menu_blur_matches_vanilla_screen_backgrounds() {
        for screen in [
            Screen::Options,
            Screen::Stats,
            Screen::Advancements,
            Screen::WorldOptions,
        ] {
            assert!(screen.blurs_background(false));
            assert!(screen.blurs_background(true));
        }
        assert!(Screen::Paused.blurs_background(true));
        assert!(!Screen::Paused.blurs_background(false));
        for screen in [
            Screen::Playing,
            Screen::GameModeSwitcher,
            Screen::Chat,
            Screen::Creative,
            Screen::Inventory,
            Screen::Dead,
            Screen::CommandBlock,
            Screen::BlockEditor,
            Screen::Win,
        ] {
            assert!(!screen.blurs_background(true));
            assert!(!screen.blurs_background(false));
        }
    }

    #[test]
    fn pause_menu_uses_disconnect_for_servers_and_keeps_world_options() {
        for (remote, label) in [(false, "menu.returnToMenu"), (true, "menu.disconnect")] {
            let buttons = pause_buttons(427.0, 240.0, remote, false, false, false);
            let quit = buttons.iter().find(|b| b.4 == 2).unwrap();
            assert_eq!(quit.3, label);
            assert!(quit.5);
            assert!(buttons.iter().any(|b| b.4 == 9 && b.5));
            assert!(buttons.iter().all(|b| ![10, 12, 13, 15].contains(&b.4)));
        }
    }

    #[test]
    fn pause_menu_centers_vertically_without_removed_icon_gap() {
        let (_, title_y, y_start, step) = pause_metrics_at(0.0, 0.0, 427.0, 240.0, 0);
        let buttons = pause_buttons(427.0, 240.0, false, false, false, false);
        let first = buttons.first().unwrap();
        let last = buttons.last().unwrap();
        assert_eq!(first.1, y_start);
        assert_eq!(last.1, y_start + step * 3.0);
        let top_margin = title_y;
        let bottom_margin = 240.0 - (last.1 + 20.0);
        assert_eq!(top_margin, bottom_margin);
    }

    #[test]
    fn server_links_adds_a_full_width_row_above_options() {
        let with = pause_buttons(427.0, 240.0, true, true, false, false);
        let without = pause_buttons(427.0, 240.0, true, false, false, false);
        let links = with.iter().find(|b| b.4 == 11).unwrap();
        let stats = with.iter().find(|b| b.4 == 3).unwrap();
        assert_eq!(links.2, 204.0);
        assert_eq!(links.3, "menu.server_links");
        assert!(links.5);
        assert_eq!(links.1, stats.1 + 24.0);
        for b in &with {
            if b.4 == 1 || b.4 == 9 || b.4 == 2 {
                let same = without.iter().find(|o| o.4 == b.4).unwrap();
                assert_eq!(b.1, same.1 + 12.0, "{}", b.3);
            }
        }
    }

    #[test]
    fn pause_screen_additions_takes_the_server_links_row() {
        let with = pause_buttons(427.0, 240.0, true, true, true, false);
        let links = pause_buttons(427.0, 240.0, true, true, false, false);
        assert!(with.iter().all(|b| b.4 != 11));
        let button = with.iter().find(|b| b.4 == 14).unwrap();
        assert_eq!(
            (button.0, button.1, button.2, button.5),
            (links[3].0, links[3].1, 204.0, true)
        );
        for id in [1, 9, 2] {
            let add = with.iter().find(|b| b.4 == id).unwrap();
            let link = links.iter().find(|b| b.4 == id).unwrap();
            assert_eq!(add.1, link.1);
        }
    }
    #[test]
    fn split_settings_preserves_dialog_rows_and_omits_icon_actions() {
        for (links, additions) in [(false, false), (true, false), (false, true), (true, true)] {
            let buttons = pause_buttons(427.0, 300.0, true, links, additions, true);
            let split = buttons.iter().find(|b| b.4 == 30).unwrap();
            let options = buttons.iter().find(|b| b.4 == 1).unwrap();
            assert_eq!(options.1, split.1 + 24.0);
            if links || additions {
                let custom = buttons
                    .iter()
                    .find(|b| b.4 == if additions { 14 } else { 11 })
                    .unwrap();
                assert_eq!(split.1, custom.1 + 24.0);
            }
            assert!(buttons.iter().all(|b| ![10, 12, 13, 15].contains(&b.4)));
            assert!(
                !pause_buttons(427.0, 300.0, true, links, additions, false)
                    .iter()
                    .any(|b| b.4 == 30)
            );
        }
    }
}
