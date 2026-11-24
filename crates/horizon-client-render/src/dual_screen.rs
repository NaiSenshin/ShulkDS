//! ShulkDS: Dual-Screen Presenter and Layout Engine for Horizon.
//!
//! Provides the primary abstraction for dual-screen clamshell rendering,
//! supporting both single-window clamshell emulation (with customizable aspect-ratio
//! presets for Retroid Pocket Duo and AYN Thor) and multi-surface dual-window mode
//! that directly mirrors the Android `DisplayManager` / `Presentation` (`ANativeWindow`) pipeline.

use crate::hud::{HudAtlas, HudBatch};
use std::time::Instant;

/// Operating presentation mode for dual-screen display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DualScreenMode {
    /// Single desktop window containing both screens stacked vertically with a clamshell hinge bezel.
    #[default]
    Emulated,
    /// Two independent OS windows / native surfaces (Top 3D world + Bottom touch dashboard).
    DualWindow,
}

impl DualScreenMode {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "emulated" | "clamshell" | "single" | "single-window" => Some(Self::Emulated),
            "dual-window" | "dual" | "multi-surface" | "presentation" => Some(Self::DualWindow),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Emulated => "emulated",
            Self::DualWindow => "dual-window",
        }
    }
}

/// Hardware aspect-ratio and dimension preset for dual-screen handhelds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DevicePreset {
    /// Retroid Pocket Duo: 5.5" 1920x1080 (16:9) top display + 4.5" 1280x960 (4:3) bottom display.
    #[default]
    RpDuo,
    /// AYN Thor: 1920x1080 (16:9) top display + 1240x1080 (~31:27) bottom display.
    AynThor,
    /// Custom dual-screen configuration.
    Custom {
        top_aspect: (u32, u32),
        bottom_aspect: (u32, u32),
    },
}

impl DevicePreset {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "rp-duo" | "rpduo" | "retroid" | "retroid-pocket-duo" | "16:9" => Some(Self::RpDuo),
            "ayn-thor" | "ayn" | "thor" | "thor-ds" => Some(Self::AynThor),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::RpDuo => "Retroid Pocket Duo (16:9 Top + 4:3 Bottom)",
            Self::AynThor => "AYN Thor (16:9 Top + 31:27 Bottom)",
            Self::Custom { .. } => "Custom Dual-Screen",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Self::RpDuo => "rp-duo",
            Self::AynThor => "ayn-thor",
            Self::Custom { .. } => "custom",
        }
    }

    /// Top screen aspect ratio (width / height).
    pub fn top_aspect_ratio(&self) -> f32 {
        match self {
            Self::RpDuo | Self::AynThor => 16.0 / 9.0,
            Self::Custom { top_aspect, .. } => top_aspect.0 as f32 / top_aspect.1.max(1) as f32,
        }
    }

    /// Bottom screen aspect ratio (width / height).
    pub fn bottom_aspect_ratio(&self) -> f32 {
        match self {
            Self::RpDuo => 4.0 / 3.0, // Retroid Pocket Duo has 4.5" 1280x960 (4:3) bottom display
            Self::AynThor => 1240.0 / 1080.0, // ~1.14815 (~31:27)
            Self::Custom { bottom_aspect, .. } => {
                bottom_aspect.0 as f32 / bottom_aspect.1.max(1) as f32
            }
        }
    }

    /// Native physical panel dimensions `(top_px, bottom_px)` for this handheld preset.
    pub fn native_panel_sizes(&self) -> ([u32; 2], [u32; 2]) {
        match self {
            Self::RpDuo => ([1920, 1080], [1280, 960]),
            Self::AynThor => ([1920, 1080], [1240, 1080]),
            Self::Custom {
                top_aspect,
                bottom_aspect,
            } => {
                let tw = (1080.0 * top_aspect.0 as f32 / top_aspect.1.max(1) as f32).round() as u32;
                let bw = (1080.0 * bottom_aspect.0 as f32 / bottom_aspect.1.max(1) as f32).round()
                    as u32;
                ([tw.max(1), 1080], [bw.max(1), 1080])
            }
        }
    }
}

/// Where the persistent gameplay hotbar is shown in ShulkDS dual-screen mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HotbarPosition {
    #[default]
    BottomScreenBottom,
    BottomScreenTop,
    TopScreenClassic,
}

impl HotbarPosition {
    pub const ALL: [Self; 3] = [
        Self::BottomScreenBottom,
        Self::BottomScreenTop,
        Self::TopScreenClassic,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::BottomScreenBottom => "bottom",
            Self::BottomScreenTop => "top",
            Self::TopScreenClassic => "classic",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "top" | "bottom_top" | "top_of_bottom" => Self::BottomScreenTop,
            "classic" | "top_classic" | "top_screen" => Self::TopScreenClassic,
            _ => Self::BottomScreenBottom,
        }
    }

    pub fn cycle(self, back: bool) -> Self {
        let idx = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        let next = if back {
            (idx + Self::ALL.len() - 1) % Self::ALL.len()
        } else {
            (idx + 1) % Self::ALL.len()
        };
        Self::ALL[next]
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::BottomScreenBottom => "Bottom Screen (Bottom)",
            Self::BottomScreenTop => "Bottom Screen (Top)",
            Self::TopScreenClassic => "Top Screen (Classic)",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::BottomScreenBottom => "horizon.shulkds.hotbar.bottom",
            Self::BottomScreenTop => "horizon.shulkds.hotbar.top",
            Self::TopScreenClassic => "horizon.shulkds.hotbar.classic",
        }
    }
}

/// Compute the GUI scale for the secondary touch display so near-square panels like
/// the AYN Thor (1240x1080) stay at crisp 4x scaling instead of dropping to 3x.
pub fn shulkds_bottom_hud_scale(sec_size: [u32; 2], touch_scale: f32) -> f32 {
    let mut scale = 1u32;
    while sec_size[0] / (scale + 1) >= 300 && sec_size[1] / (scale + 1) >= 225 {
        scale += 1;
    }
    let mult = if touch_scale.is_finite() {
        touch_scale.clamp(0.75, 1.5)
    } else {
        1.0
    };
    (scale as f32 * mult).max(1.0)
}

/// Dual-screen configuration.
#[derive(Clone, Debug)]
pub struct DualScreenConfig {
    /// Whether ShulkDS dual-screen presentation is active (default: true).
    pub enabled: bool,
    /// Selected presentation mode.
    pub mode: DualScreenMode,
    /// Handheld preset.
    pub preset: DevicePreset,
    /// Clamshell divider / hinge height in pixels for emulated mode.
    pub hinge_height: u32,
    /// Whether to display preset info, status, and FPS on the hinge bar.
    pub show_hinge_info: bool,
    /// Minimalist HUD on top screen (minimal status, hide full hotbar/modals).
    pub top_hud_minimal: bool,
    /// Bottom viewport touch GUI scale multiplier.
    pub touch_gui_scale: f32,
}

impl Default for DualScreenConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: DualScreenMode::Emulated,
            preset: DevicePreset::RpDuo,
            hinge_height: 32,
            show_hinge_info: true,
            top_hud_minimal: true,
            touch_gui_scale: 1.0,
        }
    }
}

/// Viewport rectangle in pixel units.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ViewportRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ViewportRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width: width.max(1.0),
            height: height.max(1.0),
        }
    }

    #[inline]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < (self.x + self.width) && py >= self.y && py < (self.y + self.height)
    }

    #[inline]
    pub fn aspect_ratio(&self) -> f32 {
        self.width / self.height.max(1.0)
    }

    #[inline]
    pub fn size_u32(&self) -> [u32; 2] {
        [self.width.round() as u32, self.height.round() as u32]
    }

    #[inline]
    pub fn to_scissor_rect(&self) -> [u32; 4] {
        [
            self.x.max(0.0) as u32,
            self.y.max(0.0) as u32,
            self.width.max(1.0) as u32,
            self.height.max(1.0) as u32,
        ]
    }

    /// Scale a physical viewport rect to GUI coordinates for HudBatch.
    #[inline]
    pub fn to_gui(&self, scale: f32) -> Self {
        let s = scale.max(1.0);
        Self {
            x: (self.x / s).round(),
            y: (self.y / s).round(),
            width: (self.width / s).round().max(1.0),
            height: (self.height / s).round().max(1.0),
        }
    }
}

/// Computed layout of the dual screens.
#[derive(Clone, Debug, PartialEq)]
pub struct DualScreenLayout {
    pub total_width: u32,
    pub total_height: u32,
    pub top_viewport: ViewportRect,
    pub hinge_viewport: ViewportRect,
    pub bottom_viewport: ViewportRect,
    pub mode: DualScreenMode,
    pub preset: DevicePreset,
}

impl DualScreenLayout {
    /// Compute the dual-screen layout given window dimensions and configuration.
    pub fn compute(width: u32, height: u32, config: &DualScreenConfig) -> Self {
        let w = width.max(2) as f32;
        let h = height.max(2) as f32;

        match config.mode {
            DualScreenMode::Emulated => {
                let top_aspect = config.preset.top_aspect_ratio();
                let bottom_aspect = config.preset.bottom_aspect_ratio();

                let hinge_h = (config.hinge_height as f32).min(h * 0.15).max(12.0);
                let usable_h = (h - hinge_h).max(2.0);

                // Both displays share the exact same physical display height (e.g. ~2.70" on RP Duo).
                // Clamshell fits uniformly inside the window preserving strict 16:9 top and 4:3 bottom ratios.
                let max_h_by_width = w / top_aspect;
                let max_h_by_height = usable_h / 2.0;
                let screen_h = max_h_by_width.min(max_h_by_height).floor().max(1.0);

                let top_w = (screen_h * top_aspect).round();
                let top_h = screen_h;
                let bottom_w = (screen_h * bottom_aspect).round();
                let bottom_h = screen_h;

                let clamshell_h = top_h + hinge_h + bottom_h;
                let start_y = ((h - clamshell_h) / 2.0).max(0.0).floor();
                let top_x = ((w - top_w) / 2.0).max(0.0).floor();
                let bottom_x = ((w - bottom_w) / 2.0).max(0.0).floor();

                let top_rect = ViewportRect::new(top_x, start_y, top_w, top_h);
                let hinge_rect = ViewportRect::new(top_x, start_y + top_h, top_w, hinge_h);
                let bottom_rect =
                    ViewportRect::new(bottom_x, start_y + top_h + hinge_h, bottom_w, bottom_h);

                Self {
                    total_width: width,
                    total_height: height,
                    top_viewport: top_rect,
                    hinge_viewport: hinge_rect,
                    bottom_viewport: bottom_rect,
                    mode: config.mode,
                    preset: config.preset,
                }
            }
            DualScreenMode::DualWindow => {
                // In dual-window mode, top and bottom viewports each fill their respective window
                let top_rect = ViewportRect::new(0.0, 0.0, w, h);
                let bottom_rect = ViewportRect::new(0.0, 0.0, w, h);
                let hinge_rect = ViewportRect::new(0.0, 0.0, 0.0, 0.0);

                Self {
                    total_width: width,
                    total_height: height,
                    top_viewport: top_rect,
                    hinge_viewport: hinge_rect,
                    bottom_viewport: bottom_rect,
                    mode: config.mode,
                    preset: config.preset,
                }
            }
        }
    }

    /// Scale this physical dual-screen layout to GUI coordinates for HudBatch.
    pub fn to_gui(&self, scale: f32) -> Self {
        let s = scale.max(1.0);
        Self {
            total_width: (self.total_width as f32 / s).ceil() as u32,
            total_height: (self.total_height as f32 / s).ceil() as u32,
            top_viewport: self.top_viewport.to_gui(s),
            hinge_viewport: self.hinge_viewport.to_gui(s),
            bottom_viewport: self.bottom_viewport.to_gui(s),
            mode: self.mode,
            preset: self.preset,
        }
    }

    /// Map a mouse coordinate on the window into a logical screen location.
    pub fn map_pointer(&self, window_x: f64, window_y: f64) -> ScreenPointerLocation {
        let x = window_x as f32;
        let y = window_y as f32;

        if self.top_viewport.contains(x, y) {
            let local_x = x - self.top_viewport.x;
            let local_y = y - self.top_viewport.y;
            ScreenPointerLocation::Top {
                pixel_x: local_x,
                pixel_y: local_y,
                norm_x: (local_x / self.top_viewport.width).clamp(0.0, 1.0),
                norm_y: (local_y / self.top_viewport.height).clamp(0.0, 1.0),
            }
        } else if self.hinge_viewport.contains(x, y) {
            ScreenPointerLocation::Hinge
        } else if self.bottom_viewport.contains(x, y) {
            let local_x = x - self.bottom_viewport.x;
            let local_y = y - self.bottom_viewport.y;
            ScreenPointerLocation::Bottom {
                pixel_x: local_x,
                pixel_y: local_y,
                norm_x: (local_x / self.bottom_viewport.width).clamp(0.0, 1.0),
                norm_y: (local_y / self.bottom_viewport.height).clamp(0.0, 1.0),
            }
        } else {
            ScreenPointerLocation::Outside
        }
    }
}

/// Pointer location classified across the dual-screen viewports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenPointerLocation {
    Top {
        pixel_x: f32,
        pixel_y: f32,
        norm_x: f32,
        norm_y: f32,
    },
    Hinge,
    Bottom {
        pixel_x: f32,
        pixel_y: f32,
        norm_x: f32,
        norm_y: f32,
    },
    Outside,
}

/// Normalized pointer action for the bottom dashboard touch interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BottomPointerAction {
    Down { button: PointerButton },
    Up { button: PointerButton },
    Move,
    Scroll { delta_y: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PointerButton {
    #[default]
    Primary, // Left mouse button / single finger tap
    Secondary, // Right mouse button / long press
    Middle,
}

/// Active tab on the persistent bottom touch dashboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DashboardTab {
    #[default]
    Minimap,
    Inventory,
    Settings,
}

impl DashboardTab {
    pub const ALL: [Self; 3] = [Self::Minimap, Self::Inventory, Self::Settings];

    pub fn title(&self) -> &'static str {
        match self {
            Self::Minimap => "Map",
            Self::Inventory => "Inventory",
            Self::Settings => "Settings",
        }
    }

    pub fn icon_label(&self) -> &'static str {
        match self {
            Self::Minimap => "Map",
            Self::Inventory => "Inv",
            Self::Settings => "Opt",
        }
    }
}

/// State of the persistent bottom-screen dashboard.
#[derive(Clone, Debug)]
pub struct BottomDashboardState {
    /// Currently active navigation tab.
    pub active_tab: DashboardTab,
    /// Direct hotbar slot selected by touch (0..8).
    pub hotbar_tap_slot: Option<usize>,
    /// Quick action trigger: sort inventory.
    pub quick_sort_requested: bool,
    /// Quick action trigger: quick stack to open container.
    pub quick_stack_requested: bool,
    /// Quick action trigger: auto-equip best armor/weapons.
    pub auto_equip_requested: bool,
    /// Whether an in-world container is currently open on the bottom screen.
    pub container_open: bool,
    /// In-world coordinate of the open container.
    pub container_pos: Option<[i32; 3]>,
    /// Maximum player reach distance before container auto-closes (in blocks).
    pub container_reach_distance: f64,
    /// Map zoom scale in blocks per pixel (0.5, 1.0, 2.0, 4.0).
    pub map_zoom: f32,
    /// Whether to display player coordinates on the top screen (3DS style).
    pub show_coords: bool,
    /// Current pointer/stylus position normalized in bottom screen logical coordinates (0.0..1.0).
    pub norm_pointer_x: f32,
    pub norm_pointer_y: f32,
    /// Absolute GUI coordinates of pointer/stylus.
    pub pointer_x: f32,
    pub pointer_y: f32,
    /// Whether the pointer is currently pressed down.
    pub pointer_down: bool,
    /// Currently dragged item slot index (if any).
    pub drag_slot: Option<usize>,
    /// Active slider identifier being adjusted in Quick Settings.
    pub active_slider: Option<String>,
    /// Ephemeral notification or status message displayed on the bottom screen.
    pub status_message: Option<(String, Instant)>,
    /// Whether D-Pad is currently navigating the bottom screen UI.
    pub bottom_focus: bool,
    /// Currently focused widget index on the bottom screen.
    pub focus_index: usize,
    /// Interactive widget rectangles (`[x, y, w, h]`) populated each frame for D-Pad navigation.
    pub focus_rects: Vec<[f32; 4]>,
    /// Optional inventory slot index corresponding to each `focus_rects` entry.
    pub focus_slots: Vec<Option<usize>>,
    /// Whether each `focus_rects` entry is a push-button (uses blue hover highlight instead of slot outline).
    pub focus_buttons: Vec<bool>,
    /// Touch toggle: single tap immediately quick-moves / equips items (Bedrock touch style).
    pub quick_move_mode: bool,
    /// Last tapped slot and timestamp for double-tap quick-move detection.
    pub last_tap_slot: Option<(usize, Instant)>,
    /// Currently selected / focused slot across bottom-screen panels
    /// (0..36 player, 100..109 craft, 200 craft output, 300.. container, 500..505 equip).
    pub active_slot: Option<usize>,
    /// Previously selected / interacted-with slot ("last used" visual indicator).
    pub prev_slot: Option<usize>,
    /// Last timestamp the bottom touch screen was actively touched or dragged.
    pub last_bottom_touch: Option<Instant>,
    /// Last timestamp the secondary bottom HUD swapchain was presented.
    pub last_bottom_present: Option<Instant>,
    /// Signature of dynamic UI state rendered on the bottom screen for instant-change detection.
    pub last_bottom_sig: u64,
    /// Signature of slow telemetry rendered on the bottom Map tab (coords/yaw/FPS/day).
    pub last_bottom_telemetry_sig: u64,
}

impl Default for BottomDashboardState {
    fn default() -> Self {
        Self {
            active_tab: DashboardTab::Minimap,
            hotbar_tap_slot: None,
            quick_sort_requested: false,
            quick_stack_requested: false,
            auto_equip_requested: false,
            container_open: false,
            container_pos: None,
            container_reach_distance: 5.2,
            map_zoom: 2.0,
            show_coords: true,
            norm_pointer_x: 0.0,
            norm_pointer_y: 0.0,
            pointer_x: 0.0,
            pointer_y: 0.0,
            pointer_down: false,
            drag_slot: None,
            active_slider: None,
            status_message: None,
            bottom_focus: false,
            focus_index: 0,
            focus_rects: Vec::new(),
            focus_slots: Vec::new(),
            focus_buttons: Vec::new(),
            quick_move_mode: false,
            last_tap_slot: None,
            active_slot: Some(0),
            prev_slot: None,
            last_bottom_touch: None,
            last_bottom_present: None,
            last_bottom_sig: 0,
            last_bottom_telemetry_sig: 0,
        }
    }
}

impl BottomDashboardState {
    /// Set a status banner message on the bottom screen with a 3-second timeout.
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status_message = Some((text.into(), Instant::now()));
    }

    /// Check if the status message is still active.
    pub fn current_status(&self) -> Option<&str> {
        if let Some((msg, created)) = &self.status_message {
            if created.elapsed().as_secs() < 3 {
                return Some(msg.as_str());
            }
        }
        None
    }

    /// Check player distance to open container; if out of reach, auto-close container.
    pub fn check_container_reach(&mut self, player_pos: [f64; 3]) -> bool {
        if let Some(pos) = self.container_pos {
            let dx = player_pos[0] - (pos[0] as f64 + 0.5);
            let dy = player_pos[1] - (pos[1] as f64 + 0.5);
            let dz = player_pos[2] - (pos[2] as f64 + 0.5);
            let dist_sq = dx * dx + dy * dy + dz * dz;
            if dist_sq > (self.container_reach_distance * self.container_reach_distance) {
                self.container_open = false;
                self.container_pos = None;
                self.set_status("Container closed (out of reach)");
                return false;
            }
        }
        true
    }
}

/// Dual-screen presenter managing layout, viewport geometry, and bezel styling.
pub struct DualScreenPresenter {
    pub config: DualScreenConfig,
    pub layout: DualScreenLayout,
    pub dashboard: BottomDashboardState,
    pub top_fps: u32,
    /// Desktop input focus mode: true if mouse drives 3D camera on top, false if mouse acts as bottom stylus.
    pub mouse_focused_on_top: bool,
}

impl DualScreenPresenter {
    pub fn new(config: DualScreenConfig, width: u32, height: u32) -> Self {
        let layout = DualScreenLayout::compute(width, height, &config);
        Self {
            config,
            layout,
            dashboard: BottomDashboardState::default(),
            top_fps: 60,
            mouse_focused_on_top: false,
        }
    }

    /// Update window size and recompute layouts.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.layout = DualScreenLayout::compute(width, height, &self.config);
    }

    /// Toggle mouse pointer focus between Top Screen (3D look) and Bottom Screen (touch/stylus).
    pub fn toggle_mouse_focus(&mut self) -> bool {
        self.mouse_focused_on_top = !self.mouse_focused_on_top;
        let msg = if self.mouse_focused_on_top {
            "Input Mode: Top Screen (Mouse-Look)"
        } else {
            "Input Mode: Bottom Screen (Stylus / Touch)"
        };
        self.dashboard.set_status(msg);
        self.mouse_focused_on_top
    }

    /// Draw the visual clamshell hinge divider bar on the specified rectangle.
    pub fn draw_clamshell_hinge_rect(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        rect: &ViewportRect,
        fps: u32,
    ) {
        if self.config.mode != DualScreenMode::Emulated || rect.height <= 0.0 {
            return;
        }

        let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);

        // Hinge outer bezel background (dark metallic/matte)
        let bg_color = rgba(18, 18, 22, 255);
        let border_color = rgba(38, 38, 44, 255);
        let highlight_color = rgba(60, 60, 70, 255);
        let accent_color = rgba(70, 160, 250, 255);
        let text_color = rgba(220, 220, 230, 255);

        // Draw hinge bezel bar
        batch.fill(atlas, x, y, w, h, bg_color);
        // Top and bottom bevel lines
        batch.fill(atlas, x, y, w, 1.0, highlight_color);
        batch.fill(atlas, x, y + h - 1.0, w, 1.0, border_color);

        // Subtle center seam accent line
        let seam_y = y + (h / 2.0).floor();
        batch.fill(atlas, x, seam_y, w, 1.0, rgba(28, 28, 34, 255));

        if self.config.show_hinge_info && h >= 7.0 {
            let ty = y + (h - 8.0) / 2.0;
            // Left badge: SHULK-DS | Preset Name
            let badge_text = if w >= 420.0 {
                format!("SHULK-DS // {}", self.config.preset.display_name())
            } else {
                format!("SHULK-DS // {}", self.config.preset.short_name())
            };
            batch.text(atlas, &badge_text, x + 8.0, ty, accent_color, false);

            // Right badge: FPS + Input Mode Indicator
            let mode_str = if self.mouse_focused_on_top {
                "[FOCUS: TOP]"
            } else {
                "[FOCUS: TOUCH]"
            };
            let right_text = if w >= 460.0 {
                format!("{mode_str} | {fps} FPS (Tab to toggle)")
            } else {
                format!("{mode_str} | {fps} FPS")
            };
            let rw = HudBatch::text_width(atlas, &right_text);
            let rx = (x + w - rw - 8.0).max(x + 100.0);
            batch.text(atlas, &right_text, rx, ty, text_color, false);
        }
    }

    /// Draw the visual clamshell hinge divider bar between the top and bottom viewports in Emulated mode.
    /// Automatically scales to GUI canvas coordinates based on batch.scale.
    pub fn draw_clamshell_hinge(&self, batch: &mut HudBatch, atlas: &HudAtlas, fps: u32) {
        let gui_hinge = self.layout.to_gui(batch.scale).hinge_viewport;
        self.draw_clamshell_hinge_rect(batch, atlas, &gui_hinge, fps);
    }

    /// Draw matte chassis bezels covering letterbox and pillarbox margins around the clamshell in Emulated mode.
    pub fn draw_chassis_bezels(&self, batch: &mut HudBatch, atlas: &HudAtlas) {
        if self.config.mode != DualScreenMode::Emulated {
            return;
        }

        let gui = self.layout.to_gui(batch.scale);
        let screen_w = batch.width;
        let screen_h = batch.height;

        let bezel_color = rgba(10, 10, 14, 255);
        let border_color = rgba(28, 28, 34, 255);

        let top = &gui.top_viewport;
        let hinge = &gui.hinge_viewport;
        let bottom = &gui.bottom_viewport;

        // 1. Top Letterbox (if clamshell is vertically centered)
        if top.y > 0.0 {
            batch.fill(atlas, 0.0, 0.0, screen_w, top.y, bezel_color);
            batch.fill(atlas, 0.0, top.y - 1.0, screen_w, 1.0, border_color);
        }

        // 2. Bottom Letterbox (if clamshell ends above window bottom)
        let clamshell_bottom = bottom.y + bottom.height;
        if clamshell_bottom < screen_h {
            batch.fill(
                atlas,
                0.0,
                clamshell_bottom,
                screen_w,
                screen_h - clamshell_bottom,
                bezel_color,
            );
            batch.fill(atlas, 0.0, clamshell_bottom, screen_w, 1.0, border_color);
        }

        // 3. Pillarbox beside Top Screen (left & right)
        if top.x > 0.0 {
            batch.fill(atlas, 0.0, top.y, top.x, top.height, bezel_color);
            batch.fill(atlas, top.x - 1.0, top.y, 1.0, top.height, border_color);
        }
        let top_right = top.x + top.width;
        if top_right < screen_w {
            batch.fill(
                atlas,
                top_right,
                top.y,
                screen_w - top_right,
                top.height,
                bezel_color,
            );
            batch.fill(atlas, top_right, top.y, 1.0, top.height, border_color);
        }

        // 4. Pillarbox beside Hinge (left & right)
        if hinge.x > 0.0 {
            batch.fill(atlas, 0.0, hinge.y, hinge.x, hinge.height, bezel_color);
        }
        let hinge_right = hinge.x + hinge.width;
        if hinge_right < screen_w {
            batch.fill(
                atlas,
                hinge_right,
                hinge.y,
                screen_w - hinge_right,
                hinge.height,
                bezel_color,
            );
        }

        // 5. Pillarbox beside Bottom Screen (left & right)
        if bottom.x > 0.0 {
            batch.fill(atlas, 0.0, bottom.y, bottom.x, bottom.height, bezel_color);
            batch.fill(
                atlas,
                bottom.x - 1.0,
                bottom.y,
                1.0,
                bottom.height,
                border_color,
            );
        }
        let bottom_right = bottom.x + bottom.width;
        if bottom_right < screen_w {
            batch.fill(
                atlas,
                bottom_right,
                bottom.y,
                screen_w - bottom_right,
                bottom.height,
                bezel_color,
            );
            batch.fill(
                atlas,
                bottom_right,
                bottom.y,
                1.0,
                bottom.height,
                border_color,
            );
        }
    }
}

/// Helper function to pack RGBA into u32 (0xRRGGBBAA or 0xAABBGGRR depending on shader).
#[inline]
fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from_le_bytes([r, g, b, a])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_computation_rp_duo() {
        let config = DualScreenConfig {
            mode: DualScreenMode::Emulated,
            preset: DevicePreset::RpDuo,
            hinge_height: 24,
            ..Default::default()
        };
        let layout = DualScreenLayout::compute(960, 1104, &config);

        // 960x1104 matches clamshell aspect ratio: 540 top height, 24 hinge, 540 bottom height
        assert_eq!(layout.top_viewport.y, 0.0);
        assert_eq!(layout.top_viewport.height, 540.0);
        assert_eq!(layout.top_viewport.width, 960.0); // 16:9
        assert_eq!(layout.hinge_viewport.y, 540.0);
        assert_eq!(layout.hinge_viewport.height, 24.0);
        assert_eq!(layout.bottom_viewport.y, 564.0);
        assert_eq!(layout.bottom_viewport.height, 540.0); // Matching physical height to top screen
        assert_eq!(layout.bottom_viewport.width, 720.0); // 4:3 aspect ratio (75% of 960)
        assert_eq!(layout.bottom_viewport.x, 120.0); // Centered with 120px side margins
    }

    #[test]
    fn test_layout_computation_ayn_thor() {
        let config = DualScreenConfig {
            mode: DualScreenMode::Emulated,
            preset: DevicePreset::AynThor,
            hinge_height: 24,
            ..Default::default()
        };
        let layout = DualScreenLayout::compute(960, 1104, &config);

        assert_eq!(layout.top_viewport.height, 540.0);
        assert_eq!(layout.hinge_viewport.height, 24.0);
        assert_eq!(layout.bottom_viewport.height, 540.0);

        // Bottom width for 540 height: 540 * 1240 / 1080 = 620
        assert_eq!(layout.bottom_viewport.width, 620.0);
        // Centered horizontally: (960 - 620) / 2 = 170
        assert_eq!(layout.bottom_viewport.x, 170.0);
    }

    #[test]
    fn test_layout_scaling_on_window_resize() {
        let config = DualScreenConfig {
            mode: DualScreenMode::Emulated,
            preset: DevicePreset::RpDuo,
            hinge_height: 24,
            ..Default::default()
        };

        // 1. Wide window (1600x900): clamshell centered horizontally, preserving strict 16:9 and 4:3
        let wide = DualScreenLayout::compute(1600, 900, &config);
        assert_eq!(wide.top_viewport.height, 438.0);
        assert_eq!(wide.top_viewport.width, 779.0); // 438 * 16/9
        assert_eq!(wide.top_viewport.x, 410.0); // Centered
        assert_eq!(wide.top_viewport.y, 0.0);
        // Hinge matches top screen width
        assert_eq!(wide.hinge_viewport.x, 410.0);
        assert_eq!(wide.hinge_viewport.y, 438.0);
        assert_eq!(wide.hinge_viewport.width, 779.0);
        assert_eq!(wide.hinge_viewport.height, 24.0);
        // Bottom screen matches top display height, 4:3 aspect ratio, centered
        assert_eq!(wide.bottom_viewport.height, 438.0);
        assert_eq!(wide.bottom_viewport.width, 584.0); // 438 * 4/3
        assert_eq!(wide.bottom_viewport.x, 508.0); // Centered

        // 2. Tall window (800x1200): clamshell centered vertically
        let tall = DualScreenLayout::compute(800, 1200, &config);
        assert_eq!(tall.top_viewport.x, 0.0);
        assert_eq!(tall.top_viewport.y, 138.0); // Vertically centered
        assert_eq!(tall.top_viewport.width, 800.0); // 16:9
        assert_eq!(tall.top_viewport.height, 450.0);
        assert_eq!(tall.bottom_viewport.height, 450.0);
        assert_eq!(tall.bottom_viewport.width, 600.0); // 450 * 4/3
        assert_eq!(tall.bottom_viewport.x, 100.0); // (800 - 600) / 2 = 100
        assert_eq!(tall.bottom_viewport.y, 612.0); // 138 + 450 + 24 = 612
    }

    #[test]
    fn test_pointer_mapping() {
        let config = DualScreenConfig {
            mode: DualScreenMode::Emulated,
            preset: DevicePreset::RpDuo,
            hinge_height: 24,
            ..Default::default()
        };
        let layout = DualScreenLayout::compute(960, 1104, &config);

        // Point in top screen
        match layout.map_pointer(100.0, 100.0) {
            ScreenPointerLocation::Top {
                pixel_x, pixel_y, ..
            } => {
                assert_eq!(pixel_x, 100.0);
                assert_eq!(pixel_y, 100.0);
            }
            _ => panic!("Expected Top location"),
        }

        // Point on hinge
        assert_eq!(
            layout.map_pointer(100.0, 545.0),
            ScreenPointerLocation::Hinge
        );

        // Point in bottom screen (x=200 is between 120 and 840)
        match layout.map_pointer(200.0, 600.0) {
            ScreenPointerLocation::Bottom {
                pixel_x, pixel_y, ..
            } => {
                assert_eq!(pixel_x, 80.0); // 200 - 120 = 80
                assert_eq!(pixel_y, 36.0); // 600 - 564 = 36
            }
            _ => panic!("Expected Bottom location"),
        }
    }

    #[test]
    fn test_layout_to_gui() {
        let config = DualScreenConfig {
            mode: DualScreenMode::Emulated,
            preset: DevicePreset::RpDuo,
            hinge_height: 24,
            ..Default::default()
        };
        let phys_layout = DualScreenLayout::compute(960, 1104, &config);
        let gui_layout = phys_layout.to_gui(2.0);

        assert_eq!(gui_layout.top_viewport.y, 0.0);
        assert_eq!(gui_layout.top_viewport.height, 270.0);
        assert_eq!(gui_layout.hinge_viewport.y, 270.0);
        assert_eq!(gui_layout.hinge_viewport.height, 12.0);
        assert_eq!(gui_layout.bottom_viewport.y, 282.0);
        assert_eq!(gui_layout.bottom_viewport.height, 270.0);
        assert_eq!(gui_layout.bottom_viewport.width, 360.0);
        assert_eq!(gui_layout.bottom_viewport.x, 60.0);
    }

    #[test]
    fn test_bottom_hud_scale_keeps_rp_duo_and_ayn_thor_at_4x() {
        assert_eq!(shulkds_bottom_hud_scale([1280, 960], 1.0), 4.0);
        assert_eq!(shulkds_bottom_hud_scale([1240, 1080], 1.0), 4.0);
        assert_eq!(shulkds_bottom_hud_scale([960, 720], 1.0), 3.0);
        assert_eq!(shulkds_bottom_hud_scale([1240, 1080], 1.25), 5.0);
    }

    #[test]
    fn test_hotbar_position_round_trip_and_cycle() {
        for pos in HotbarPosition::ALL {
            assert_eq!(HotbarPosition::parse(pos.name()), pos);
        }
        assert_eq!(
            HotbarPosition::BottomScreenBottom.cycle(false),
            HotbarPosition::BottomScreenTop
        );
        assert_eq!(
            HotbarPosition::BottomScreenTop.cycle(false),
            HotbarPosition::TopScreenClassic
        );
        assert_eq!(
            HotbarPosition::TopScreenClassic.cycle(false),
            HotbarPosition::BottomScreenBottom
        );
        assert_eq!(
            HotbarPosition::BottomScreenBottom.cycle(true),
            HotbarPosition::TopScreenClassic
        );
    }
}
