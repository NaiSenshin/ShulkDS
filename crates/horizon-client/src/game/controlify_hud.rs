//! Controlify-inspired in-game HUD overlays:
//! 1. 8-slice Radial Quick-Action Wheel (hold D-Pad Down in-game).

use super::{Game, Screen};
use horizon_client::{
    dual_screen::DashboardTab,
    gamepad::RadialMenu,
    hud::HudBatch,
};

#[inline]
fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from_le_bytes([r, g, b, a])
}

const RADIAL_LABELS: [&str; RadialMenu::SLICES] = [
    "Next Tab",
    "Camera",
    "Coords",
    "Sort Inv",
    "Auto Equip",
    "Drop Stack",
    "Focus",
    "Pause",
];

impl Game<'_> {
    /// Execute the action for a selected Controlify radial menu slice.
    pub(super) fn execute_radial_action(&mut self, slice: usize) {
        match slice {
            0 => {
                // Cycle 3DS bottom-screen tab
                let all = DashboardTab::ALL;
                let cur = all
                    .iter()
                    .position(|&t| t == self.shulkds.dashboard.active_tab)
                    .unwrap_or(0);
                let next_tab = all[(cur + 1) % all.len()];
                if next_tab != self.shulkds.dashboard.active_tab {
                    self.return_shulkds_carried_to_inventory();
                    self.shulkds.dashboard.active_tab = next_tab;
                }
            }
            1 => {
                // Toggle 1st / 3rd person perspective
                self.camera_type = self.camera_type.cycle();
            }
            2 => {
                // Toggle top-screen coordinates
                self.shulkds.dashboard.show_coords = !self.shulkds.dashboard.show_coords;
            }
            3 => {
                self.execute_quick_sort();
            }
            4 => {
                self.execute_auto_equip();
            }
            5 => {
                self.drop_held(true);
            }
            6 => {
                self.handle_shulkds_input_toggle();
            }
            7 => {
                self.pause_focus = None;
                self.screen = super::Screen::Paused;
                self.pause_menu = true;
                self.capture(false);
            }
            _ => {}
        }
        self.audio.play_button_click();
    }

    /// Render the 8-slice Controlify Radial Quick-Action Wheel on the top screen.
    pub(super) fn draw_controlify_radial_menu(&self, batch: &mut HudBatch) {
        if !self.pad.radial.open || self.screen != Screen::Playing || self.title.is_some() {
            return;
        }
        let (tx, ty, tw, th) = self.shulkds_top_gui_rect(batch.scale);
        let cx = tx + tw * 0.5;
        let cy = ty + th * 0.5;
        let radius = (th.min(tw) * 0.32).clamp(52.0, 90.0);

        // Backdrop ring
        let panel_w = radius * 2.3;
        let panel_h = radius * 2.3;
        batch.fill(
            &self.gui.atlas,
            cx - panel_w * 0.5,
            cy - panel_h * 0.5,
            panel_w,
            panel_h,
            rgba(12, 14, 18, 175),
        );

        let title = "Quick Wheel";
        let tw_lbl = HudBatch::text_width(&self.gui.atlas, title);
        batch.text(
            &self.gui.atlas,
            title,
            cx - tw_lbl * 0.5,
            cy - 4.0,
            rgba(230, 230, 240, 255),
            true,
        );

        let slice_arc = std::f32::consts::TAU / RadialMenu::SLICES as f32;
        for (i, label) in RADIAL_LABELS.iter().enumerate() {
            let angle = (i as f32) * slice_arc;
            let sx = cx + angle.sin() * radius;
            let sy = cy - angle.cos() * radius;
            let selected = self.pad.radial.selected == Some(i);

            let lw = HudBatch::text_width(&self.gui.atlas, label);
            let bw = (lw + 10.0).max(46.0);
            let bh = 16.0;
            let bx = sx - bw * 0.5;
            let by = sy - bh * 0.5;

            let (bg, border, fg) = if selected {
                (
                    rgba(56, 140, 210, 235),
                    rgba(255, 255, 255, 255),
                    rgba(255, 255, 255, 255),
                )
            } else {
                (
                    rgba(28, 30, 38, 220),
                    rgba(75, 80, 95, 220),
                    rgba(205, 210, 220, 255),
                )
            };

            batch.fill(&self.gui.atlas, bx, by, bw, bh, bg);
            batch.fill(&self.gui.atlas, bx, by, bw, 1.0, border);
            batch.fill(&self.gui.atlas, bx, by + bh - 1.0, bw, 1.0, border);
            batch.fill(&self.gui.atlas, bx, by, 1.0, bh, border);
            batch.fill(&self.gui.atlas, bx + bw - 1.0, by, 1.0, bh, border);
            batch.text(
                &self.gui.atlas,
                label,
                bx + (bw - lw) * 0.5,
                by + 4.0,
                fg,
                true,
            );
        }
    }
}
