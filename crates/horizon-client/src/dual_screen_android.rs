//! Secondary-display (`Presentation` / `ANativeWindow`) state machine and normalized touch routing
//! for dual-screen Android handhelds (Retroid Pocket Duo, AYN Thor).
//!
//! Kept 100% safe so `horizon-client-lib` (`#![forbid(unsafe_code)]`) and `horizon-client` share one
//! process-wide state singleton; the raw `ANativeWindow*` pointer is carried as an opaque `usize` and
//! converted only inside `horizon_client::android` (`#[cfg(target_os = "android")]`).

use crate::dual_screen::{BottomPointerAction, PointerButton};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

static SECONDARY_IME_ACTIVE: AtomicBool = AtomicBool::new(false);
static SECONDARY_IME_REFOCUS: AtomicBool = AtomicBool::new(false);
static SECONDARY_IME_GRACE_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

const SECONDARY_IME_FOCUS_GRACE: Duration = Duration::from_millis(600);

pub fn set_secondary_ime_active(active: bool) {
    let mut grace = SECONDARY_IME_GRACE_UNTIL
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if active {
        *grace = None;
        SECONDARY_IME_REFOCUS.store(false, Ordering::Release);
    } else {
        *grace = Some(Instant::now() + SECONDARY_IME_FOCUS_GRACE);
        SECONDARY_IME_REFOCUS.store(true, Ordering::Release);
    }
    SECONDARY_IME_ACTIVE.store(active, Ordering::Release);
}

pub fn secondary_ime_active() -> bool {
    SECONDARY_IME_ACTIVE.load(Ordering::Acquire)
}

pub fn secondary_ime_or_grace_active() -> bool {
    if SECONDARY_IME_ACTIVE.load(Ordering::Acquire) {
        return true;
    }
    let grace = SECONDARY_IME_GRACE_UNTIL
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    grace.is_some_and(|until| Instant::now() < until)
}

pub fn take_secondary_ime_refocus() -> bool {
    SECONDARY_IME_REFOCUS.swap(false, Ordering::AcqRel)
}

/// Android `MotionEvent` masked action constants (`MotionEvent.getActionMasked()`).
pub const ACTION_DOWN: i32 = 0;
pub const ACTION_UP: i32 = 1;
pub const ACTION_MOVE: i32 = 2;
pub const ACTION_CANCEL: i32 = 3;
pub const ACTION_POINTER_DOWN: i32 = 5;
pub const ACTION_POINTER_UP: i32 = 6;

/// Normalize raw touch coordinates `(x, y)` against surface dimensions `(width, height)` into
/// clamped `0.0..=1.0`, returning `(0.0, 0.0)` safely when dimensions are zero or non-finite.
pub fn normalize_touch(x: f32, y: f32, width: f32, height: f32) -> (f32, f32) {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return (0.0, 0.0);
    }
    let nx = if x.is_finite() {
        (x / width).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let ny = if y.is_finite() {
        (y / height).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (nx, ny)
}

/// Tracks the single active finger driving the bottom-screen stylus pointer so secondary fingers
/// touching or lifting during an inventory drag never drop `self.carried` prematurely.
#[derive(Debug, Default)]
pub struct PrimaryPointerTracker {
    active_pointer: Option<i32>,
}

impl PrimaryPointerTracker {
    pub fn reset(&mut self) {
        self.active_pointer = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn on_motion_event(
        &mut self,
        primary_pointer_id: i32,
        action_masked: i32,
        action_pointer_id: i32,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<(f32, f32, BottomPointerAction)> {
        let (nx, ny) = normalize_touch(x, y, width, height);
        match action_masked {
            ACTION_DOWN => {
                self.active_pointer = Some(action_pointer_id);
                Some((
                    nx,
                    ny,
                    BottomPointerAction::Down {
                        button: PointerButton::Primary,
                    },
                ))
            }
            ACTION_POINTER_DOWN => {
                if self.active_pointer.is_none() {
                    self.active_pointer = Some(action_pointer_id);
                    Some((
                        nx,
                        ny,
                        BottomPointerAction::Down {
                            button: PointerButton::Primary,
                        },
                    ))
                } else {
                    None
                }
            }
            ACTION_MOVE => {
                if self.active_pointer == Some(primary_pointer_id) {
                    Some((nx, ny, BottomPointerAction::Move))
                } else {
                    None
                }
            }
            ACTION_POINTER_UP => {
                if self.active_pointer == Some(action_pointer_id) {
                    self.active_pointer = None;
                    Some((
                        nx,
                        ny,
                        BottomPointerAction::Up {
                            button: PointerButton::Primary,
                        },
                    ))
                } else {
                    None
                }
            }
            ACTION_UP | ACTION_CANCEL => {
                if self.active_pointer.take().is_some() {
                    Some((
                        nx,
                        ny,
                        BottomPointerAction::Up {
                            button: PointerButton::Primary,
                        },
                    ))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
struct SurfaceInner {
    generation: u64,
    window: Option<(usize, [u32; 2])>,
    gpu_attached_gen: Option<u64>,
}

/// Synchronizes `ANativeWindow*` lifecycle between Android's UI thread (`SurfaceHolder.Callback`)
/// and the game's render loop thread (`wgpu::Surface`), guaranteeing a bounded non-deadlocking wait
/// when `surfaceDestroyed` fires even if the render loop is paused or not yet started.
#[derive(Default)]
pub struct SecondarySurfaceState {
    inner: Mutex<SurfaceInner>,
    cvar: Condvar,
}

impl SecondarySurfaceState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a ready or resized `ANativeWindow` (`ptr != 0`). Returns a displaced previous
    /// `ANativeWindow` pointer (if `ptr` changed) after waiting briefly for GPU detach so the caller
    /// can `ANativeWindow_release` it safely.
    pub fn set_window(&self, ptr: usize, width: u32, height: u32) -> Option<usize> {
        if ptr == 0 {
            return self.clear_and_wait_for_gpu_detach(Duration::from_millis(250));
        }
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((old_ptr, old_size)) = guard.window
            && old_ptr == ptr
        {
            if old_size != [width, height] {
                guard.window = Some((ptr, [width, height]));
                guard.generation = guard.generation.wrapping_add(1);
            }
            // Balance the fresh +1 refcount acquired by `ANativeWindow_fromSurface` on `surfaceChanged`.
            return Some(old_ptr);
        }
        let displaced = guard.window.map(|(old_ptr, _)| old_ptr);
        if displaced.is_some() {
            guard.window = None;
            guard.generation = guard.generation.wrapping_add(1);
            let deadline = Instant::now() + Duration::from_millis(250);
            while guard.gpu_attached_gen.is_some() {
                let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                let (next_guard, res) = self
                    .cvar
                    .wait_timeout(guard, rem)
                    .unwrap_or_else(PoisonError::into_inner);
                guard = next_guard;
                if res.timed_out() {
                    break;
                }
            }
        }
        guard.window = Some((ptr, [width, height]));
        guard.generation = guard.generation.wrapping_add(1);
        displaced
    }

    /// Clear the active `ANativeWindow` and wait up to `timeout` only if the render loop currently
    /// holds a `VkSurfaceKHR` on it (`gpu_attached_gen.is_some()`). Returns immediately when idle.
    pub fn clear_and_wait_for_gpu_detach(&self, timeout: Duration) -> Option<usize> {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let old = guard.window.take().map(|(ptr, _)| ptr);
        if old.is_some() {
            guard.generation = guard.generation.wrapping_add(1);
        }
        let deadline = Instant::now() + timeout;
        while guard.gpu_attached_gen.is_some() {
            let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            let (next_guard, res) = self
                .cvar
                .wait_timeout(guard, rem)
                .unwrap_or_else(PoisonError::into_inner);
            guard = next_guard;
            if res.timed_out() {
                break;
            }
        }
        old
    }

    /// Polled by the render thread to detect surface attach, resize, or detach.
    pub fn poll_change(
        &self,
        seen_gen: u64,
        current_size: Option<[u32; 2]>,
    ) -> Option<(u64, Option<(usize, [u32; 2])>)> {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.generation != seen_gen || guard.window.map(|(_, s)| s) != current_size {
            Some((guard.generation, guard.window))
        } else {
            None
        }
    }

    pub fn mark_gpu_attached(&self, generation: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.window.is_some() && guard.generation == generation {
            guard.gpu_attached_gen = Some(generation);
        }
        self.cvar.notify_all();
    }

    pub fn mark_gpu_detached(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        guard.gpu_attached_gen = None;
        self.cvar.notify_all();
    }
}

struct TouchQueue {
    tracker: PrimaryPointerTracker,
    events: VecDeque<(f32, f32, BottomPointerAction)>,
}

fn surface_singleton() -> &'static SecondarySurfaceState {
    static STATE: OnceLock<SecondarySurfaceState> = OnceLock::new();
    STATE.get_or_init(SecondarySurfaceState::new)
}

fn touch_singleton() -> &'static Mutex<TouchQueue> {
    static TOUCHES: OnceLock<Mutex<TouchQueue>> = OnceLock::new();
    TOUCHES.get_or_init(|| {
        Mutex::new(TouchQueue {
            tracker: PrimaryPointerTracker::default(),
            events: VecDeque::with_capacity(32),
        })
    })
}

pub fn set_secondary_native_window(ptr: usize, width: u32, height: u32) -> Option<usize> {
    surface_singleton().set_window(ptr, width, height)
}

pub fn clear_secondary_native_window_and_wait(timeout: Duration) -> Option<usize> {
    reset_secondary_touches();
    surface_singleton().clear_and_wait_for_gpu_detach(timeout)
}

pub fn poll_secondary_surface_change(
    seen_gen: u64,
    current_size: Option<[u32; 2]>,
) -> Option<(u64, Option<(usize, [u32; 2])>)> {
    surface_singleton().poll_change(seen_gen, current_size)
}

pub fn mark_secondary_surface_attached(generation: u64) {
    surface_singleton().mark_gpu_attached(generation);
}

pub fn mark_secondary_surface_detached() {
    surface_singleton().mark_gpu_detached();
}

#[allow(clippy::too_many_arguments)]
pub fn push_secondary_motion_event(
    primary_pointer_id: i32,
    action_masked: i32,
    action_pointer_id: i32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let mut q = touch_singleton()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(ev) = q.tracker.on_motion_event(
        primary_pointer_id,
        action_masked,
        action_pointer_id,
        x,
        y,
        width,
        height,
    ) {
        if q.events.len() >= 256 {
            q.events.pop_front();
        }
        q.events.push_back(ev);
    }
}

pub fn drain_bottom_touches() -> Vec<(f32, f32, BottomPointerAction)> {
    let mut q = touch_singleton()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    q.events.drain(..).collect()
}

pub fn reset_secondary_touches() {
    let mut q = touch_singleton()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    q.tracker.reset();
    q.events.clear();
}

/// Decoded device battery and local clock state for the ShulkDS in-game HUD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceHudStatus {
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub use_24h: bool,
    pub local_hm: Option<(u8, u8)>,
}

/// Unpack the bit-packed status returned by `StarlingActivity.deviceHudStatus()`.
pub fn decode_device_hud_status(packed: i32) -> DeviceHudStatus {
    let raw = packed as u32;
    let pct_byte = (raw & 0xFF) as u8;
    let battery_percent = (pct_byte <= 100).then_some(pct_byte);
    let charging = ((raw >> 8) & 1) != 0;
    let use_24h = ((raw >> 9) & 1) != 0;
    let hour = ((raw >> 10) & 0x1F) as u8;
    let minute = ((raw >> 15) & 0x3F) as u8;
    let valid_time = ((raw >> 21) & 1) != 0 && hour < 24 && minute < 60;
    DeviceHudStatus {
        battery_percent,
        charging,
        use_24h,
        local_hm: valid_time.then_some((hour, minute)),
    }
}

/// Format `(hour24, minute)` in either 24-hour (`16:30`) or 12-hour (`4:30 PM`) clock format.
pub fn format_local_clock(hour24: u8, minute: u8, use_24h: bool) -> String {
    let h = hour24 % 24;
    let m = minute % 60;
    if use_24h {
        format!("{h:02}:{m:02}")
    } else {
        let suffix = if h < 12 { "AM" } else { "PM" };
        let h12 = match h % 12 {
            0 => 12,
            n => n,
        };
        format!("{h12}:{m:02} {suffix}")
    }
}

/// Best-effort Linux desktop sysfs battery reader for local offscreen testing.
#[cfg(all(target_os = "linux", not(target_os = "android")))]
pub fn read_linux_sysfs_battery() -> Option<(u8, bool)> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = std::fs::read_to_string(path.join("type")) else {
            continue;
        };
        if kind.trim() != "Battery" {
            continue;
        }
        let Ok(cap_str) = std::fs::read_to_string(path.join("capacity")) else {
            continue;
        };
        let Ok(cap) = cap_str.trim().parse::<u16>() else {
            continue;
        };
        let pct = cap.min(100) as u8;
        let charging = std::fs::read_to_string(path.join("status"))
            .map(|s| s.trim().eq_ignore_ascii_case("Charging"))
            .unwrap_or(false);
        return Some((pct, charging));
    }
    None
}

#[cfg(not(all(target_os = "linux", not(target_os = "android"))))]
pub fn read_linux_sysfs_battery() -> Option<(u8, bool)> {
    None
}

pub const BATTERY_ICON_W: usize = 17;
pub const BATTERY_ICON_H: usize = 9;
pub const BATTERY_SHELL_W: f32 = 16.0;
pub const BATTERY_SHELL_H: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryPixel {
    Empty,
    Shadow,
    Shell,
    Cavity,
    FillHi,
    FillBody,
    Bolt,
}

/// Rasterize a crisp `16x8` pixel-art battery icon (+1px non-overlapping drop shadow on a `17x9` grid).
pub fn battery_pixel_grid(
    percent: u8,
    charging: bool,
) -> [[BatteryPixel; BATTERY_ICON_W]; BATTERY_ICON_H] {
    let mut grid = [[BatteryPixel::Empty; BATTERY_ICON_W]; BATTERY_ICON_H];

    for x in 1..13 {
        grid[0][x] = BatteryPixel::Shell;
        grid[7][x] = BatteryPixel::Shell;
    }
    for y in 1..7 {
        grid[y][0] = BatteryPixel::Shell;
        grid[y][13] = BatteryPixel::Shell;
        for x in 1..13 {
            grid[y][x] = BatteryPixel::Cavity;
        }
    }
    for y in 2..6 {
        grid[y][14] = BatteryPixel::Shell;
    }
    for y in 3..5 {
        grid[y][15] = BatteryPixel::Shell;
    }

    let pct = percent.min(100);
    let cols = if pct == 0 {
        0
    } else {
        ((f32::from(pct) / 10.0).round() as usize).clamp(1, 10)
    };
    for dx in 0..cols {
        let x = 2 + dx;
        grid[2][x] = BatteryPixel::FillHi;
        for y in 3..6 {
            grid[y][x] = BatteryPixel::FillBody;
        }
    }

    if charging {
        const BOLT: [(usize, usize); 16] = [
            (7, 1),
            (8, 1),
            (6, 2),
            (7, 2),
            (5, 3),
            (6, 3),
            (7, 3),
            (8, 3),
            (6, 4),
            (7, 4),
            (8, 4),
            (9, 4),
            (7, 5),
            (8, 5),
            (6, 6),
            (7, 6),
        ];
        for &(bx, by) in &BOLT {
            for (nx, ny) in [
                (bx.saturating_sub(1), by),
                (bx + 1, by),
                (bx, by.saturating_sub(1)),
                (bx, by + 1),
            ] {
                if (1..13).contains(&nx) && (1..7).contains(&ny) {
                    grid[ny][nx] = BatteryPixel::Cavity;
                }
            }
        }
        for &(bx, by) in &BOLT {
            grid[by][bx] = BatteryPixel::Bolt;
        }
    }

    for y in 1..BATTERY_ICON_H {
        for x in 1..BATTERY_ICON_W {
            if grid[y][x] == BatteryPixel::Empty && grid[y - 1][x - 1] == BatteryPixel::Shell {
                grid[y][x] = BatteryPixel::Shadow;
            }
        }
    }

    grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dual_screen::{BottomPointerAction, PointerButton};
    use std::time::Duration;

    #[test]
    fn normalize_touch_zero_dimensions_returns_clamped_zero() {
        let (nx, ny) = normalize_touch(250.0, -40.0, 0.0, 0.0);
        assert!(nx.is_finite() && ny.is_finite());
        assert_eq!((nx, ny), (0.0, 0.0));

        let (nx2, ny2) = normalize_touch(640.0, 480.0, 1280.0, 960.0);
        assert!((nx2 - 0.5).abs() < 1e-6);
        assert!((ny2 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn primary_pointer_tracker_ignores_secondary_finger_up() {
        let mut tracker = PrimaryPointerTracker::default();
        // Finger 10 touches down (ACTION_DOWN = 0)
        assert_eq!(
            tracker.on_motion_event(10, 0, 10, 320.0, 240.0, 1280.0, 960.0),
            Some((
                0.25,
                0.25,
                BottomPointerAction::Down {
                    button: PointerButton::Primary
                }
            ))
        );
        // Finger 22 touches down (ACTION_POINTER_DOWN = 5) while finger 10 is still dragging
        assert_eq!(
            tracker.on_motion_event(10, 5, 22, 320.0, 240.0, 1280.0, 960.0),
            None
        );
        // Finger 22 lifts (ACTION_POINTER_UP = 6) — must NOT drop finger 10's drag!
        assert_eq!(
            tracker.on_motion_event(10, 6, 22, 320.0, 240.0, 1280.0, 960.0),
            None
        );
        // Finger 10 moves (ACTION_MOVE = 2)
        assert_eq!(
            tracker.on_motion_event(10, 2, 10, 640.0, 480.0, 1280.0, 960.0),
            Some((0.5, 0.5, BottomPointerAction::Move))
        );
        // Finger 10 lifts via ACTION_POINTER_UP = 6 (while another finger stays down) -> emits Up
        assert_eq!(
            tracker.on_motion_event(22, 6, 10, 640.0, 480.0, 1280.0, 960.0),
            Some((
                0.5,
                0.5,
                BottomPointerAction::Up {
                    button: PointerButton::Primary
                }
            ))
        );
    }

    #[test]
    fn secondary_surface_destroy_completes_when_render_loop_idle() {
        let state = SecondarySurfaceState::new();
        // Surface arrives while game loop is idle (`attached_in_gpu` is false)
        let prev = state.set_window(0xDEAD_BEEF, 1280, 960);
        assert_eq!(prev, None);
        // Same pointer on `surfaceChanged` returns `Some(old_ptr)` to balance ANativeWindow_fromSurface +1 ref
        let same = state.set_window(0xDEAD_BEEF, 1280, 960);
        assert_eq!(same, Some(0xDEAD_BEEF));
        // Surface is destroyed before render loop ever attached VkSurfaceKHR -> returns immediately
        let old = state.clear_and_wait_for_gpu_detach(Duration::from_millis(50));
        assert_eq!(old, Some(0xDEAD_BEEF));

        // Now simulate attached_in_gpu = true with render loop detaching on a worker thread
        let state_arc = std::sync::Arc::new(SecondarySurfaceState::new());
        state_arc.set_window(0xCAFE_BABE, 1280, 960);
        let polled = state_arc.poll_change(0, None);
        assert_eq!(polled, Some((1, Some((0xCAFE_BABE, [1280, 960])))));
        state_arc.mark_gpu_attached(1);

        let worker = state_arc.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            worker.mark_gpu_detached();
        });
        let released = state_arc.clear_and_wait_for_gpu_detach(Duration::from_millis(250));
        handle.join().unwrap();
        assert_eq!(released, Some(0xCAFE_BABE));
    }

    #[test]
    fn secondary_ime_grace_and_refocus_lifecycle() {
        set_secondary_ime_active(true);
        assert!(secondary_ime_active());
        assert!(secondary_ime_or_grace_active());
        assert!(!take_secondary_ime_refocus());

        set_secondary_ime_active(false);
        assert!(!secondary_ime_active());
        assert!(secondary_ime_or_grace_active());
        assert!(take_secondary_ime_refocus());
        assert!(!take_secondary_ime_refocus());
    }

    #[test]
    fn decode_device_hud_status_unpacks_all_fields() {
        let packed = (85u32 | (1 << 8) | (1 << 9) | (16 << 10) | (42 << 15) | (1 << 21)) as i32;
        assert_eq!(
            decode_device_hud_status(packed),
            DeviceHudStatus {
                battery_percent: Some(85),
                charging: true,
                use_24h: true,
                local_hm: Some((16, 42)),
            }
        );

        let unknown = (0xFFu32 | (25 << 10) | (1 << 21)) as i32;
        assert_eq!(
            decode_device_hud_status(unknown),
            DeviceHudStatus {
                battery_percent: None,
                charging: false,
                use_24h: false,
                local_hm: None,
            }
        );
    }

    #[test]
    fn format_local_clock_handles_12h_and_24h() {
        assert_eq!(format_local_clock(0, 5, false), "12:05 AM");
        assert_eq!(format_local_clock(9, 30, false), "9:30 AM");
        assert_eq!(format_local_clock(12, 0, false), "12:00 PM");
        assert_eq!(format_local_clock(16, 30, false), "4:30 PM");
        assert_eq!(format_local_clock(23, 59, false), "11:59 PM");

        assert_eq!(format_local_clock(0, 5, true), "00:05");
        assert_eq!(format_local_clock(9, 30, true), "09:30");
        assert_eq!(format_local_clock(12, 0, true), "12:00");
        assert_eq!(format_local_clock(16, 30, true), "16:30");
        assert_eq!(format_local_clock(23, 59, true), "23:59");
    }

    #[test]
    fn battery_pixel_grid_levels_bolt_and_non_overlapping_shadow() {
        let count_cols = |grid: &[[BatteryPixel; BATTERY_ICON_W]; BATTERY_ICON_H]| -> usize {
            (2..12)
                .filter(|&x| grid[2][x] == BatteryPixel::FillHi)
                .count()
        };

        let g0 = battery_pixel_grid(0, false);
        assert_eq!(count_cols(&g0), 0);

        let g5 = battery_pixel_grid(5, false);
        assert_eq!(count_cols(&g5), 1);

        let g50 = battery_pixel_grid(50, false);
        assert_eq!(count_cols(&g50), 5);

        let g100 = battery_pixel_grid(100, false);
        assert_eq!(count_cols(&g100), 10);

        let g_chg = battery_pixel_grid(60, true);
        let bolt_count = g_chg
            .iter()
            .flatten()
            .filter(|&&p| p == BatteryPixel::Bolt)
            .count();
        assert_eq!(bolt_count, 16);

        for y in 1..BATTERY_ICON_H {
            for x in 1..BATTERY_ICON_W {
                if g_chg[y][x] == BatteryPixel::Shadow {
                    assert_eq!(g_chg[y - 1][x - 1], BatteryPixel::Shell);
                }
            }
        }
    }
}
