//! Android: the prompt when GitHub has a newer ShulkDS release than this one (`starling_releases`).
//! Update downloads its APK and hands it to Android's installer, which asks the player (and, the
//! first time, to allow installing from ShulkDS) and offers to open the game again once it is in;
//! Tomorrow asks again in a day; No, at the next start.
//!
//! Only a release asks: a debug build is signed with another key (`STARLING_UPDATE_TEST=<version>`
//! overrides that for local testing).
use super::Game;
use super::shulkds_title::ShulkdsUpdateView;
use horizon_client::{
    hud::HudBatch,
    starling_releases::{self as releases, Update as Release},
    ui_theme::{Face, Ui, color},
};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use winit::event::{Touch, TouchPhase};

/// How long Tomorrow keeps the prompt away.
const LATER: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, PartialEq)]
enum Stage {
    Checking,
    Ask,
    Downloading,
    /// The installer was shown; it may be shown again.
    Installing,
    UpToDate(String),
    Failed(String),
}

#[derive(Clone)]
struct DevOverride {
    repositories: Vec<String>,
    token: Option<String>,
    asset_prefix: String,
}

/// What the check and the download threads hand the game.
#[derive(Default)]
struct Shared {
    release: Option<Release>,
    check_done: Option<(u64, String, Result<Option<Release>, String>)>,
    done: u64,
    total: u64,
    /// The APK's name in files/updates/ once downloaded, or why it is not.
    downloaded: Option<Result<String, String>>,
    token: Option<String>,
}

#[derive(Default)]
pub(super) struct UpdateOffer {
    started: bool,
    manual: bool,
    generation: u64,
    stage: Option<Stage>,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    /// The buttons as last drawn (UI units), at most three.
    buttons: Cell<[Option<[f32; 4]>; 3]>,
}

fn updates() -> PathBuf {
    horizon_client::logging::private_home().join("updates")
}

/// Written by Tomorrow: the time (Unix seconds) before which the prompt is not made.
fn later() -> PathBuf {
    horizon_client::logging::private_home().join("update-later")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn load_dev_override() -> DevOverride {
    let (default_repos, asset_prefix, _) =
        releases::channel_for_package(crate::android::package_id().as_deref());
    DevOverride {
        repositories: default_repos.iter().map(|s| (*s).to_owned()).collect(),
        token: None,
        asset_prefix: asset_prefix.to_owned(),
    }
}

fn current_app_version(_dev: &DevOverride) -> Option<String> {
    if let Some(pretend) = std::env::var("STARLING_UPDATE_TEST")
        .ok()
        .filter(|v| !v.is_empty())
    {
        return Some(pretend);
    }
    match crate::android::app_version() {
        Some((version, true)) => Some(version),
        _ => None,
    }
}

/// Fetches a URL with optional GitHub Bearer auth, stripping Authorization before following
/// any cross-origin CDN redirect (e.g. GitHub API asset -> S3 presigned URL).
fn get_response(
    agent: &ureq::Agent,
    url: &str,
    user_agent: &str,
    token: Option<&str>,
    accept: Option<&str>,
) -> Result<ureq::Response, String> {
    let mut req = agent.get(url).set("User-Agent", user_agent);
    if let Some(a) = accept {
        req = req.set("Accept", a);
    }
    if let Some(tok) = token {
        req = req.set("Authorization", &format!("Bearer {tok}"));
    }
    let resp = req.call().map_err(|e| e.to_string())?;
    if matches!(resp.status(), 301 | 302 | 303 | 307 | 308)
        && let Some(loc) = resp.header("Location")
    {
        if !loc.starts_with("https://") {
            return Err("Unsafe redirect while downloading update.".into());
        }
        return ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .build()
            .get(loc)
            .set("User-Agent", user_agent)
            .call()
            .map_err(|e| e.to_string());
    }
    Ok(resp)
}

/// A repository's latest release, as GitHub's API serves it.
fn latest(url: &str, current: &str, token: Option<&str>) -> Result<serde_json::Value, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let ua = format!("ShulkDS/{current}");
    let text = get_response(&agent, url, &ua, token, Some("application/vnd.github+json"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn fetch_sums_sha256(
    sums_url: &str,
    asset_name: &str,
    current: &str,
    token: Option<&str>,
) -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(if token.is_some() { 0 } else { 5 })
        .timeout(Duration::from_secs(15))
        .build();
    let ua = format!("ShulkDS/{current}");
    let accept = if token.is_some() && sums_url.starts_with("https://api.github.com/") {
        Some("application/octet-stream")
    } else {
        None
    };
    let text = get_response(&agent, sums_url, &ua, token, accept)
        .ok()?
        .into_string()
        .ok()?;
    let bytes = releases::sha256_from_sums(&text, asset_name)?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The newest ShulkDS release on GitHub, when it is newer than `current` and has its APK:
/// every configured repository asked, an error only when none answered.
fn check(current: &str, cfg: &DevOverride) -> Result<Option<Release>, String> {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    let abi = releases::process_abi();
    for repository in &cfg.repositories {
        match latest(
            &releases::latest_url(repository),
            current,
            cfg.token.as_deref(),
        ) {
            Ok(release) => found.extend(releases::update_in_with_prefix(
                &release,
                current,
                abi,
                &cfg.asset_prefix,
            )),
            Err(e) => errors.push(format!("{repository}: {e}")),
        }
    }
    if errors.len() == cfg.repositories.len() {
        return Err(errors.join("; "));
    }
    let mut best = releases::newest(found);
    if let Some(ref mut rel) = best
        && rel.sha256.is_none()
        && let Some(sums_url) = &rel.sums_url
    {
        rel.sha256 = fetch_sums_sha256(sums_url, &rel.name, current, cfg.token.as_deref());
    }
    Ok(best)
}

/// The release's APK into files/updates/, checked; its name there.
fn download(
    release: &Release,
    token: Option<&str>,
    shared: &Mutex<Shared>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let ok = |n: &str| {
        !n.is_empty()
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    if !ok(&release.name) || !release.url.starts_with("https://") {
        return Err(format!(
            "the release's APK is named oddly: {}",
            release.name
        ));
    }
    let dir = updates();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", release.name));

    let use_api_asset = token.is_some() && release.api_asset_url.starts_with("https://");
    let fetch_url = if use_api_asset {
        &release.api_asset_url
    } else {
        &release.url
    };
    // No limit on the whole download (a slow network), only on each wait.
    let agent = ureq::AgentBuilder::new()
        .redirects(if use_api_asset { 0 } else { 5 })
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .build();
    let ua = format!("ShulkDS/{}", release.version);
    let accept = use_api_asset.then_some("application/octet-stream");
    let response = get_response(&agent, fetch_url, &ua, token, accept)?;
    let total = response
        .header("Content-Length")
        .and_then(|l| l.parse().ok())
        .unwrap_or(release.size);
    shared.lock().unwrap_or_else(PoisonError::into_inner).total = total;
    let mut body = response.into_reader();
    let mut file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut done = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".into());
        }
        let n = body.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        file.write_all(&buffer[..n])
            .map_err(|e| format!("{}: {e}", part.display()))?;
        done += n as u64;
        shared.lock().unwrap_or_else(PoisonError::into_inner).done = done;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if release.sha256.as_ref().is_some_and(|want| *want != got) || done == 0 {
        let _ = std::fs::remove_file(&part);
        return Err("The download came out damaged. Try again.".into());
    }
    std::fs::rename(&part, dir.join(&release.name)).map_err(|e| e.to_string())?;
    Ok(release.name.clone())
}

/// The UI's scale for a window of `w` x `h` pixels (as `Ui::new`).
fn density(w: f32, h: f32) -> f32 {
    (w / 1040.0).min(h / 700.0).clamp(1.0, 4.0)
}

fn spawn_check_thread(
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    current: String,
    cfg: DevOverride,
) {
    shared.lock().unwrap_or_else(PoisonError::into_inner).token = cfg.token.clone();
    let _ = std::thread::Builder::new()
        .name("update check".into())
        .spawn(move || {
            let result = check(&current, &cfg);
            if let Ok(Some(ref r)) = result {
                log::info!("Horizon: ShulkDS {} is out (this is {current})", r.version);
            } else if let Err(ref e) = result {
                log::info!("Horizon: could not look for a newer ShulkDS: {e}");
            }
            shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .check_done = Some((generation, current, result));
        });
}

impl Game<'_> {
    /// Trigger an immediate update check from Options -> Horizon / ShulkDS.
    pub(super) fn check_updates_now(&mut self) {
        let cfg = load_dev_override();
        let current = current_app_version(&cfg).unwrap_or_else(|| {
            crate::android::app_version()
                .map(|(v, _)| v)
                .unwrap_or_else(|| horizon_core::release::VERSION.to_owned())
        });
        if self.screen == super::Screen::Options {
            let _ = self.options_key(winit::keyboard::KeyCode::Escape, None);
        }
        let offer = &mut self.update_offer;
        offer.started = true;
        offer.manual = true;
        offer.generation = offer.generation.wrapping_add(1);
        offer.cancel.store(false, Ordering::Relaxed);
        offer.stage = Some(Stage::Checking);
        let check_gen = offer.generation;
        {
            let mut s = offer.shared.lock().unwrap_or_else(PoisonError::into_inner);
            s.release = None;
            s.check_done = None;
            s.done = 0;
            s.total = 0;
            s.downloaded = None;
        }
        spawn_check_thread(offer.shared.clone(), check_gen, current, cfg);
    }

    /// Once at start (the check, in the background), then the download while it runs.
    pub(super) fn poll_update_offer(&mut self) {
        let auto_enabled = self.settings.extras.shulkds_auto_updates;
        let offer = &mut self.update_offer;
        if !offer.started {
            offer.started = true;
            // An update installed, or one left: the APK goes.
            let _ = std::fs::remove_dir_all(updates());
            if !auto_enabled {
                return;
            }
            let cfg = load_dev_override();
            let Some(current) = current_app_version(&cfg) else {
                return;
            };
            let until = std::fs::read_to_string(later())
                .ok()
                .and_then(|t| t.trim().parse::<u64>().ok());
            if until.is_some_and(|t| now() < t) {
                return;
            }
            offer.generation = offer.generation.wrapping_add(1);
            spawn_check_thread(offer.shared.clone(), offer.generation, current, cfg);
            return;
        }
        let mut shared = offer.shared.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((check_gen, checked_ver, result)) = shared.check_done.take()
            && check_gen == offer.generation
        {
            let was_manual = offer.manual;
            offer.manual = false;
            match result {
                Ok(Some(release)) => {
                    shared.release = Some(release);
                    if !offer.cancel.load(Ordering::Relaxed) {
                        offer.stage = Some(Stage::Ask);
                    }
                }
                Ok(None) => {
                    if was_manual {
                        offer.stage = Some(Stage::UpToDate(checked_ver));
                    }
                }
                Err(e) => {
                    if was_manual {
                        offer.stage = Some(Stage::Failed(e));
                    }
                }
            }
        }
        if offer.stage == Some(Stage::Downloading)
            && let Some(result) = shared.downloaded.take()
        {
            offer.stage = Some(match result {
                Ok(name) if crate::android::install_update(&name) => Stage::Installing,
                Ok(_) => Stage::Failed("Android has no installer that takes the update.".into()),
                Err(e) => Stage::Failed(e),
            });
        }
    }

    /// Over the title (or when triggered manually), and not over the folder offer.
    fn update_offer_shown(&self) -> Option<Stage> {
        if self.files_offer_shown().is_some() {
            return None;
        }
        if self.title.is_none() && !self.update_offer.manual && self.update_offer.stage.is_none() {
            return None;
        }
        if self.title.is_none() && self.screen == super::Screen::Options {
            return None;
        }
        self.update_offer.stage.clone()
    }

    fn release(&self) -> Option<Release> {
        self.update_offer
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .release
            .clone()
    }

    fn build_update_strings(
        stage: &Stage,
        release: Option<&Release>,
        done: u64,
        total: u64,
    ) -> ShulkdsUpdateView {
        let mb = |bytes: u64| format!("{:.0}", bytes as f64 / 1_000_000.0);
        let version = release.map_or_else(String::new, |r| r.version.clone());
        let size_bytes = release.map_or(0, |r| r.size);
        let highlights = release.map_or_else(Vec::new, |r| r.highlights.clone());
        let (stage_title, body, progress, buttons): (
            String,
            String,
            Option<f32>,
            Vec<&'static str>,
        ) = match stage {
            Stage::Checking => (
                "Checking for Updates".into(),
                "Contacting GitHub for the latest ShulkDS release...".into(),
                None,
                vec!["Cancel"],
            ),
            Stage::Ask => (
                format!("ShulkDS {version} is out"),
                format!(
                    "Update now? ShulkDS downloads it ({} MB) and Android installs it over this \
                     build. Your worlds and settings stay.",
                    mb(size_bytes)
                ),
                None,
                vec!["Update", "Tomorrow", "No"],
            ),
            Stage::Downloading => {
                let frac = if total > 0 {
                    (done as f32 / total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (
                    format!("Downloading ShulkDS {version}"),
                    format!("{} of {} MB", mb(done), mb(total)),
                    Some(frac),
                    vec!["Cancel"],
                )
            }
            Stage::Installing => (
                format!("Install ShulkDS {version}"),
                "Android asks to install the update (the first time, allow unknown installs for \
                 ShulkDS). Once finished, open ShulkDS again."
                    .into(),
                None,
                vec!["Install", "Later"],
            ),
            Stage::UpToDate(ver) => (
                "ShulkDS is Up to Date".into(),
                format!("You are already running the latest release (v{ver})."),
                None,
                vec!["Close"],
            ),
            Stage::Failed(e) => (
                "Update Check Failed".into(),
                e.clone(),
                None,
                if release.is_some() {
                    vec!["Try again", "Later"]
                } else {
                    vec!["Retry check", "Close"]
                },
            ),
        };
        ShulkdsUpdateView {
            version,
            size_mb: if size_bytes > 0 {
                mb(size_bytes)
            } else {
                String::new()
            },
            highlights,
            stage_title,
            body,
            progress,
            buttons,
        }
    }

    pub(super) fn shulkds_update_modal_view(&self) -> Option<ShulkdsUpdateView> {
        let stage = self.update_offer_shown()?;
        let release = self.release();
        let (done, total) = {
            let shared = self
                .update_offer
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (
                shared.done,
                shared.total.max(release.as_ref().map_or(0, |r| r.size)),
            )
        };
        Some(Self::build_update_strings(
            &stage,
            release.as_ref(),
            done,
            total,
        ))
    }

    pub(super) fn shulkds_update_sig(&self) -> u64 {
        let Some(stage) = &self.update_offer.stage else {
            return 0;
        };
        let disc: u64 = match stage {
            Stage::Checking => 1,
            Stage::Ask => 2,
            Stage::Downloading => 3,
            Stage::Installing => 4,
            Stage::UpToDate(_) => 5,
            Stage::Failed(_) => 6,
        };
        let (done, total, has_rel) = {
            let shared = self
                .update_offer
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (
                shared.done,
                shared.total,
                u64::from(shared.release.is_some()),
            )
        };
        let pct = if total > 0 {
            ((done * 100) / total).min(100)
        } else {
            done / (64 * 1024)
        };
        (disc << 32) ^ (has_rel << 24) ^ pct
    }

    pub(super) fn shulkds_update_act(&mut self, button: usize) {
        let Some(stage) = self.update_offer_shown() else {
            return;
        };
        let release = self.release();
        let (shared_arc, token) = {
            let s = self
                .update_offer
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (self.update_offer.shared.clone(), s.token.clone())
        };
        let offer = &mut self.update_offer;
        match (stage, button) {
            (Stage::Ask, 0) | (Stage::Failed(_), 0) if release.is_some() => {
                if let Some(release) = release {
                    offer.stage = Some(Stage::Downloading);
                    offer.cancel.store(false, Ordering::Relaxed);
                    let cancel = offer.cancel.clone();
                    {
                        let mut s = shared_arc.lock().unwrap_or_else(PoisonError::into_inner);
                        s.done = 0;
                        s.downloaded = None;
                    }
                    let _ = std::thread::Builder::new()
                        .name("update download".into())
                        .spawn(move || {
                            let result = download(&release, token.as_deref(), &shared_arc, &cancel);
                            if let Err(e) = &result {
                                log::warn!("Horizon: the update did not download: {e}");
                            }
                            shared_arc
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .downloaded = Some(result);
                        });
                }
            }
            (Stage::Failed(_), 0) => {
                self.check_updates_now();
            }
            (Stage::Ask, 1) => {
                let _ = std::fs::write(later(), (now() + LATER.as_secs()).to_string());
                offer.stage = None;
                offer.cancel.store(true, Ordering::Relaxed);
            }
            (Stage::Installing, 0) => {
                if let Some(release) = release
                    && !crate::android::install_update(&release.name)
                {
                    offer.stage = Some(Stage::Failed(
                        "Android has no installer that takes the update.".into(),
                    ));
                }
            }
            // Checking/Downloading Cancel, Ask No, Installing Later, UpToDate Close, Failed Close:
            _ => {
                offer.manual = false;
                offer.generation = offer.generation.wrapping_add(1);
                offer.cancel.store(true, Ordering::Relaxed);
                offer.stage = None;
            }
        }
    }

    /// A card in the middle of the top/main screen when not using the dual-screen title modal.
    pub(super) fn draw_update_offer(&self, batch: &mut HudBatch) {
        if self.shulkds_title_split_mode() {
            return;
        }
        let Some(stage) = self.update_offer_shown() else {
            return;
        };
        let release = self.release();
        let (done, total) = {
            let shared = self
                .update_offer
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (
                shared.done,
                shared.total.max(release.as_ref().map_or(0, |r| r.size)),
            )
        };
        let view = Self::build_update_strings(&stage, release.as_ref(), done, total);
        let title = view.stage_title;
        let body = view.body;
        let buttons = view.buttons;

        let mut ui = Ui::new(&self.gui.atlas, batch, (-1.0, -1.0));
        let (w, h) = (ui.w, ui.h);
        ui.fill(0.0, 0.0, w, h, color::with_alpha(color::BG, 0.72));
        let card_w = (w - 48.0).min(470.0);
        let pad = 22.0;
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in body.split(' ') {
            let next = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && ui.measure(Face::Regular, 14.0, &next) > card_w - 2.0 * pad {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = next;
            }
        }
        lines.push(line);
        let bar = view.progress.is_some();
        let button_h = 40.0;
        let card_h = pad
            + 26.0
            + 12.0
            + lines.len() as f32 * 21.0
            + if bar { 22.0 } else { 0.0 }
            + 22.0
            + button_h
            + pad;
        let (x, y) = ((w - card_w) / 2.0, ((h - card_h) / 2.0).max(8.0));
        ui.rounded(x, y, card_w, card_h, 14.0, color::BG);
        ui.border(x, y, card_w, card_h, 14.0, color::BORDER);
        ui.text(Face::Semibold, 18.0, x + pad, y + pad, color::TEXT, &title);
        let mut ty = y + pad + 26.0 + 12.0;
        for line in &lines {
            ui.text(Face::Regular, 14.0, x + pad, ty, color::MUTED, line);
            ty += 21.0;
        }
        if let Some(f) = view.progress {
            let bw = card_w - 2.0 * pad;
            let bar_y = ty + 8.0;
            ui.rounded(x + pad, bar_y, bw, 6.0, 3.0, color::RAISED);
            if f > 0.0 {
                ui.rounded(x + pad, bar_y, (bw * f).max(6.0), 6.0, 3.0, color::ACCENT);
            }
        }
        // The first button is the primary one, on the right; the others to its left.
        let by = y + card_h - pad - button_h;
        let gap = 12.0;
        let n = buttons.len() as f32;
        let bw = (card_w - 2.0 * pad - gap * (n - 1.0)) / n;
        let mut drawn = [None; 3];
        for (i, text) in buttons.iter().enumerate() {
            let bx = x + card_w - pad - bw - i as f32 * (bw + gap);
            let rect = [bx, by, bw, button_h];
            let (fill, ink) = if i == 0 && buttons.len() > 1 {
                (color::ACCENT, color::ON_ACCENT)
            } else {
                (color::RAISED, color::TEXT)
            };
            ui.rounded(rect[0], rect[1], rect[2], rect[3], 10.0, fill);
            ui.text_centered(
                Face::Semibold,
                15.0,
                rect[0] + rect[2] / 2.0,
                rect[1] + (rect[3] - 15.0) / 2.0 - 2.0,
                ink,
                text,
            );
            drawn[i] = Some(rect);
        }
        ui.finish();
        self.update_offer.buttons.set(drawn);
    }

    /// While the card shows it takes every touch; a lifted finger on a button acts.
    pub(super) fn update_offer_touch(&mut self, touch: &Touch) -> bool {
        if self.shulkds_title_split_mode() {
            return false;
        }
        if self.update_offer_shown().is_none() {
            return false;
        }
        if touch.phase != TouchPhase::Ended {
            return true;
        }
        let [w, h] = self.touch_size();
        let d = density(w as f32, h as f32);
        let p = [touch.location.x as f32 / d, touch.location.y as f32 / d];
        let inside = |r: [f32; 4]| {
            p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
        };
        let Some(button) = self
            .update_offer
            .buttons
            .get()
            .iter()
            .position(|r| r.is_some_and(inside))
        else {
            return true;
        };
        self.shulkds_update_act(button);
        true
    }
}
