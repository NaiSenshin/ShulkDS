// Horizon's side of the Distant Horizons port, added to the game as
// `game::distant_horizons_bridge`: the block table, chunks and world
// generator `game::distant_horizons` reads, the meshes it makes in Horizon's
// far-terrain format, and the session that runs it for the world the player
// is in. Only this module knows Horizon's types; it compiles only inside the
// game (the mod's Cargo library is runtime.rs alone).
//
// Part of Distant Horizons for Horizon Client, LGPL-3.0 (see runtime.rs).

use crate::game::Game;
use crate::game::distant_horizons::{
    self as dh, BlockInfo, BlockTable, ChunkView, ColumnSampler, DetailPlan, Dir, FullDataSource,
    Host, Quad, RenderSettings, SampledRun, Settings,
    engine::{Engine, Near, WorkSettings},
    material, pos,
};
use horizon_assets::model::Direction;
use horizon_client::{
    chunk::{Chunk, HEIGHT, MIN_Y},
    fast_hash::FxHashSet,
    lod::{Span as LodSpan, TileKey},
    mesher::{OPAQUE_LAYER, SectionMesh, TerrainVertex},
    streaming::{LodSampler, LodSource, WorkerContext},
};
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Chunks put in the LODs per frame at most; the rest wait for later frames.
const CHUNKS_PER_FRAME: usize = 8;
/// How often loaded chunks are checked for new and changed ones.
const SCAN_EVERY: Duration = Duration::from_millis(250);
/// How often one chunk may be read again after it changed.
const REREAD_EVERY: Duration = Duration::from_secs(5);
/// The far-terrain upload budget per frame, as Horizon's own far terrain.
const UPLOAD_BUDGET: Duration = Duration::from_millis(1);

/// What `BlockInfo` Horizon's block facts give each state.
fn block_table(facts: &horizon_client::blocks::BlockFacts) -> BlockTable {
    let states = (0..facts.states.len())
        .map(|i| {
            let state = BlockStateId(i as u32);
            let f = facts.get(state);
            let name = f.name.strip_prefix("minecraft:").unwrap_or(&f.name);
            let air = facts.is_air(state);
            let water = matches!(name, "water" | "bubble_column");
            let liquid = water || name == "lava";
            let ice = name == "ice" || name == "frosted_ice";
            let see_through = water
                || ice
                || name.contains("glass")
                || name == "slime_block"
                || name == "honey_block";
            let material = if air {
                material::AIR
            } else if water {
                material::WATER
            } else if name == "lava" {
                material::LAVA
            } else if name == "snow" || name == "snow_block" || name == "powder_snow" {
                material::SNOW
            } else if name.ends_with("_leaves") {
                material::LEAVES
            } else if name.contains("sand") {
                material::SAND
            } else if name == "grass_block" {
                material::GRASS
            } else if f.emission > 0 {
                material::ILLUMINATED
            } else {
                material::UNKNOWN
            };
            // Leaves draw opaque far away, as Horizon's own far terrain does.
            let opaque = (f.solid_render || name.ends_with("_leaves")) && !see_through;
            BlockInfo {
                air,
                opacity: if air {
                    0
                } else if opaque {
                    16
                } else {
                    f.dampening.min(15)
                },
                // Chorus is thin, but a visible part of the far End.
                solid: !air && (f.full_collision || name.starts_with("chorus_")),
                liquid,
                material,
                alpha: if see_through { 0xA0 } else { 255 },
                snow_layer: name == "snow",
                ice,
            }
        })
        .collect();
    BlockTable { states }
}

/// The block states and biomes the section files were written with: files
/// from another set are made again.
fn fingerprint(facts: &horizon_client::blocks::BlockFacts) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    0x4448_5f56_32u64.hash(&mut h);
    for s in &facts.states {
        s.name.hash(&mut h);
    }
    h.finish()
}

fn canonical_states(
    registry: &horizon_world::registry::BlockRegistry,
    facts: &horizon_client::blocks::BlockFacts,
    blocks: &BlockTable,
) -> Arc<[u32]> {
    let air = facts.air.0;
    (0..facts.states.len())
        .map(|i| {
            let id = BlockStateId(i as u32);
            if facts.is_air(id) {
                return air;
            }
            let Some(st) = registry.state(id) else {
                return id.0;
            };
            if st.properties.get("snowy").map(String::as_str) == Some("true") {
                return id.0;
            }
            match registry.default_state(&st.name) {
                Some(def) if blocks.get(def.0) == blocks.get(id.0) => def.0,
                _ => id.0,
            }
        })
        .collect::<Vec<_>>()
        .into()
}

/// A loaded chunk, read by `LodDataBuilder`.
struct LoadedChunk {
    chunk: Arc<Chunk>,
    canonical: Arc<[u32]>,
    air: u32,
}

impl ChunkView for LoadedChunk {
    fn chunk_x(&self) -> i32 {
        self.chunk.x
    }
    fn chunk_z(&self) -> i32 {
        self.chunk.z
    }
    fn min_build_height(&self) -> i32 {
        MIN_Y
    }
    fn max_build_height(&self) -> i32 {
        MIN_Y + HEIGHT
    }
    fn min_non_empty_height(&self) -> i32 {
        let air = self.air;
        let lowest = self.chunk.sections.iter().position(|s| !s.is_uniform(air));
        lowest.map_or(MIN_Y + HEIGHT, |s| MIN_Y + s as i32 * 16)
    }
    fn top(&self, x: i32, z: i32) -> i32 {
        let air = self.air;
        self.chunk
            .highest(air, |y| self.chunk.get(x, y, z) != air)
            .map_or(MIN_Y, |y| y + 1)
    }
    fn state(&self, x: i32, y: i32, z: i32) -> u32 {
        if (MIN_Y..MIN_Y + HEIGHT).contains(&y) {
            let s = self.chunk.get(x, y, z);
            self.canonical.get(s as usize).copied().unwrap_or(s)
        } else {
            self.air
        }
    }
    fn biome(&self, x: i32, y: i32, z: i32) -> u16 {
        self.chunk.biome(x, y, z)
    }
    fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let light = self.chunk.light(x, y, z);
        (light >> 4, light & 15)
    }
}

/// Horizon's far-terrain sampler of the world generator.
struct Sampler {
    inner: Box<dyn LodSampler>,
    canonical: Arc<[u32]>,
    blocks: Arc<BlockTable>,
}

fn convert_sampled_spans(
    spans: &[LodSpan],
    canonical: &[u32],
    blocks: &BlockTable,
    out: &mut Vec<SampledRun>,
) {
    let canon = |raw: u32| canonical.get(raw as usize).copied().unwrap_or(raw);
    let roof_min = spans
        .iter()
        .rev()
        .find(|span| {
            let info = blocks.get(canon(span.state.0));
            info.opacity == 16 && info.material != material::LEAVES && !info.liquid
        })
        .map(|span| span.min);
    out.extend(spans.iter().map(|span| {
        let sky = if roof_min.is_some_and(|y| span.max <= y) {
            0
        } else {
            ((span.light >> 4) & 15) as u8
        };
        SampledRun {
            min: i32::from(span.min),
            max: i32::from(span.max),
            state: canon(span.state.0),
            biome: span.biome,
            sky,
            block: ((span.light >> 12) & 15) as u8,
        }
    }));
}

impl ColumnSampler for Sampler {
    fn column(&mut self, x: i32, z: i32, out: &mut Vec<SampledRun>) {
        let column = self.inner.column(x, z);
        convert_sampled_spans(&column.spans, &self.canonical, &self.blocks, out);
    }
    fn section_grid(
        &mut self,
        base_x: i32,
        base_z: i32,
        width: i32,
    ) -> Option<Vec<Vec<SampledRun>>> {
        let side = dh::pos::WIDTH as usize;
        self.inner.set_approximate(width >= 4);
        let cols = self.inner.grid(base_x, base_z, width, side);
        if cols.len() != side * side {
            return None;
        }
        // `LodSampler::grid` returns row-major (`z * side + x`), whereas
        // `FullDataSource` indexes `dh::index(x, z)` (`x * side + z`).
        let mut out = vec![Vec::new(); side * side];
        for z in 0..side {
            for x in 0..side {
                let column = &cols[z * side + x];
                let mut runs = Vec::with_capacity(column.spans.len());
                convert_sampled_spans(&column.spans, &self.canonical, &self.blocks, &mut runs);
                out[x * side + z] = runs;
            }
        }
        Some(out)
    }
}

#[derive(Clone, Copy)]
enum Tint {
    None,
    Grass,
    Foliage,
    Dry,
    Water,
    Constant(u32),
}

/// A face's sprite: the block model's face looking that way, as Horizon's
/// own far terrain picks it (`lod::mesh::Mesher::material`).
#[derive(Clone, Copy)]
struct Style {
    layer: u16,
    tint: Tint,
    translucent: bool,
}

struct HorizonHost {
    context: Arc<WorkerContext>,
    blocks: Arc<BlockTable>,
    canonical: Arc<[u32]>,
    /// The world generator, once the game has one to give.
    source: Mutex<Option<Arc<dyn LodSource>>>,
    styles: Mutex<HashMap<(u32, u8), [Option<Style>; 2]>>,
}

fn direction(dir: Dir) -> Direction {
    match dir {
        Dir::Down => Direction::Down,
        Dir::Up => Direction::Up,
        Dir::North => Direction::North,
        Dir::South => Direction::South,
        Dir::West => Direction::West,
        Dir::East => Direction::East,
    }
}

impl HorizonHost {
    fn styles(&self, state: u32, dir: Dir) -> [Option<Style>; 2] {
        let key = (state, dir as u8);
        if let Some(found) = self.styles.lock().unwrap().get(&key) {
            return *found;
        }
        let styles = self.resolve(state, direction(dir));
        self.styles.lock().unwrap().insert(key, styles);
        styles
    }
    fn resolve(&self, state: u32, direction: Direction) -> [Option<Style>; 2] {
        let assets = &self.context.assets;
        let render = &self.context.render;
        let name = assets
            .registry
            .state(BlockStateId(state))
            .map_or("", |s| s.name.as_str());
        let fluid = match name {
            "minecraft:water" | "minecraft:bubble_column" => Some(("block/water_still", true)),
            "minecraft:lava" => Some(("block/lava_still", false)),
            _ => None,
        };
        if let Some((sprite, water)) = fluid {
            let layer = render
                .sprite_layers
                .get(sprite)
                .or_else(|| render.sprite_layers.get(&format!("minecraft:{sprite}")))
                .copied()
                .unwrap_or(0) as u16
                | if !water { OPAQUE_LAYER } else { 0 };
            let tint = if water { Tint::Water } else { Tint::None };
            return [
                Some(Style {
                    layer,
                    tint,
                    translucent: water,
                }),
                None,
            ];
        }
        if name == "minecraft:grass_block"
            && !matches!(direction, Direction::Up | Direction::Down)
            && assets
                .registry
                .state(BlockStateId(state))
                .and_then(|s| s.properties.get("snowy"))
                .map(String::as_str)
                != Some("true")
            && let Some(&layer) = render
                .sprite_layers
                .get("block/grass_block_top")
                .or_else(|| render.sprite_layers.get("minecraft:block/grass_block_top"))
        {
            return [
                Some(Style {
                    layer: (layer as u16) | OPAQUE_LAYER,
                    tint: Tint::Grass,
                    translucent: false,
                }),
                None,
            ];
        }
        let leaves = name.ends_with("_leaves");
        let tint = match name.strip_prefix("minecraft:").unwrap_or(name) {
            "spruce_leaves" => Tint::Constant(0x61_9961),
            "birch_leaves" => Tint::Constant(0x80_A755),
            "leaf_litter" => Tint::Dry,
            _ if leaves => Tint::Foliage,
            _ => Tint::Grass,
        };
        let models = render
            .model_catalog
            .collect(BlockStateId(state), BlockPos::new(0, 0, 0))
            .ok();
        let mut matching = models
            .iter()
            .flatten()
            .flat_map(|(_, model)| &model.faces)
            .filter(|face| face.quad.facing == direction);
        let mut faces = [None; 2];
        for slot in &mut faces {
            let Some(face) = matching.next() else { break };
            *slot = Some(Style {
                layer: face.layer as u16
                    | if leaves || face.opaque {
                        OPAQUE_LAYER
                    } else {
                        0
                    },
                tint: if face.quad.tint_index >= 0 {
                    tint
                } else {
                    Tint::None
                },
                translucent: face.translucent && !leaves,
            });
            // Grass's tinted side overlay must follow its untinted base.
            if name != "minecraft:grass_block"
                || matches!(direction, Direction::Up | Direction::Down)
            {
                break;
            }
        }
        // A model with no face this way (a plant): its first face, so the
        // far box still shows its colour.
        if faces[0].is_none()
            && let Some(face) = models
                .iter()
                .flatten()
                .flat_map(|(_, model)| &model.faces)
                .next()
        {
            faces[0] = Some(Style {
                layer: face.layer as u16
                    | if leaves || face.opaque {
                        OPAQUE_LAYER
                    } else {
                        0
                    },
                tint: if face.quad.tint_index >= 0 {
                    tint
                } else {
                    Tint::None
                },
                translucent: face.translucent && !leaves,
            });
        }
        faces
    }
}

impl Host for HorizonHost {
    type Mesh = SectionMesh;

    fn blocks(&self) -> &BlockTable {
        &self.blocks
    }
    fn build_height(&self) -> (i32, i32) {
        (MIN_Y, MIN_Y + HEIGHT)
    }
    fn sampler(&self) -> Option<Box<dyn ColumnSampler>> {
        let source = self.source.lock().unwrap().clone()?;
        Some(Box::new(Sampler {
            inner: LodSource::sampler(source),
            canonical: self.canonical.clone(),
            blocks: self.blocks.clone(),
        }))
    }
    fn mesh(
        &self,
        section: i64,
        source: &FullDataSource,
        quads: &[Quad],
        debug: bool,
    ) -> SectionMesh {
        let mut mesh = SectionMesh::default();
        let detail = usize::from(pos::detail_level(section) - pos::SECTION_MINIMUM_DETAIL_LEVEL);
        let debug_color =
            dh::DEBUG_DETAIL_LEVEL_COLORS[detail.min(dh::DEBUG_DETAIL_LEVEL_COLORS.len() - 1)];
        for q in quads {
            if q.dir == Dir::Down && q.plane <= MIN_Y {
                continue;
            }
            let (biome, state) = source.mapping.get(q.id);
            let info = self.context.biomes.get(biome);
            // Vanilla's directional shade (`ClientLevel.getShade`).
            let shade = match q.dir {
                Dir::Up => 1.0,
                Dir::Down => 0.5,
                Dir::North | Dir::South => 0.8,
                Dir::West | Dir::East => 0.6,
            };
            let light = u16::from(q.sky) << 4 | u16::from(q.block) << 12;
            for style in self.styles(state, q.dir).into_iter().flatten() {
                let rgb = if debug {
                    debug_color
                } else {
                    match style.tint {
                        Tint::None => 0xFF_FFFF,
                        Tint::Grass => info.grass,
                        Tint::Foliage => info.foliage,
                        Tint::Dry => info.dry_foliage,
                        Tint::Water => info.water,
                        Tint::Constant(c) => c,
                    }
                };
                let channel = |shift: u32| (((rgb >> shift) & 255) as f32 * shade) as u32;
                let color = channel(16) | channel(8) << 8 | channel(0) << 16 | 0xFF00_0000;
                let face = Face {
                    dir: q.dir,
                    plane: q.plane as f32,
                    layer: style.layer,
                    light,
                    color,
                    translucent: style.translucent || q.transparent,
                };
                emit(&mut mesh, face, [q.u0, q.v0, q.u1, q.v1].map(|v| v as f32));
            }
        }
        mesh.sort_solid();
        mesh
    }
}

struct Face {
    dir: Dir,
    plane: f32,
    layer: u16,
    light: u16,
    color: u32,
    translucent: bool,
}

/// The widest face in blocks: its tiled UVs must stay within
/// `TerrainVertex::tiled`'s range (Horizon's `lod::mesh::MAX_FACE`).
const MAX_FACE: f32 = 1024.0;

fn emit(mesh: &mut SectionMesh, face: Face, [u0, v0, u1, v1]: [f32; 4]) {
    let mut u = u0;
    while u < u1 {
        let u_end = u1.min(u + MAX_FACE);
        let mut v = v0;
        while v < v1 {
            let v_end = v1.min(v + MAX_FACE);
            emit_quad(mesh, &face, [u, v, u_end, v_end]);
            v = v_end;
        }
        u = u_end;
    }
}

/// One quad as Horizon's far terrain writes them (`lod::mesh::emit_quad`):
/// the same corner order and winding, and block-tiled UVs counted from the
/// face's corner.
fn emit_quad(mesh: &mut SectionMesh, face: &Face, [u0, v0, u1, v1]: [f32; 4]) {
    let p = face.plane;
    let mut points = match face.dir {
        Dir::Up | Dir::Down => [[u0, p, v1], [u1, p, v1], [u1, p, v0], [u0, p, v0]],
        Dir::North | Dir::South => [[u0, v1, p], [u1, v1, p], [u1, v0, p], [u0, v0, p]],
        Dir::West | Dir::East => [[p, v1, u1], [p, v1, u0], [p, v0, u0], [p, v0, u1]],
    };
    if matches!(face.dir, Dir::Down | Dir::South | Dir::East) {
        points.reverse();
    }
    let uvs = points.map(|pos| match face.dir {
        Dir::Up | Dir::Down => [pos[0], -pos[2]],
        Dir::North | Dir::South => [pos[0], -pos[1]],
        Dir::West | Dir::East => [pos[2], -pos[1]],
    });
    let corner = [0, 1].map(|i| {
        uvs.iter()
            .map(|uv| uv[i])
            .fold(f32::INFINITY, f32::min)
            .floor()
    });
    let out = if face.translucent {
        &mut mesh.translucent
    } else {
        &mut mesh.solid
    };
    out.extend(points.into_iter().zip(uvs).map(|(pos, uv)| {
        TerrainVertex::tiled(
            pos,
            [uv[0] - corner[0], uv[1] - corner[1]],
            face.layer,
            face.color,
            face.light,
        )
    }));
}

/// A section's far-terrain tile: `TileKey` level `l` is `16 << l` blocks,
/// a section of detail `d` is `2^d`.
fn tile(section: i64) -> TileKey {
    TileKey {
        x: pos::x(section),
        z: pos::z(section),
        level: pos::detail_level(section) - 4,
    }
}

/// The world, dimension and connection a session is for.
#[derive(Clone, PartialEq)]
struct WorldKey {
    root: PathBuf,
    dimension: &'static str,
    remote: bool,
}

struct Session {
    key: WorldKey,
    /// Where the sections are kept; none: in memory.
    dir: Option<PathBuf>,
    last_generator: Instant,
    engine: Engine<HorizonHost>,
    /// Loaded chunks put in the LODs, and when.
    canonical: Arc<[u32]>,
    seen: HashMap<(i32, i32), Instant>,
    pending: Vec<(i32, i32)>,
    last_scan: Option<Instant>,
    near_revision: Option<u64>,
    loaded: Arc<HashSet<(i32, i32)>>,
    staged_tiles: Vec<(TileKey, SectionMesh, Vec<(i32, i32)>)>,
    staged_gone: Vec<TileKey>,
    gpu_tile_cutouts: HashMap<TileKey, Vec<(i32, i32)>>,
    active_cutout: FxHashSet<(i32, i32)>,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// The settings, read once and kept up to date by the settings page.
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
    /// The LOD render distance in chunks while a session runs, else 0.
    static RADIUS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static NOTICE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn settings(game: &Game<'_>) -> Settings {
    SETTINGS.with_borrow_mut(|s| {
        *s.get_or_insert_with(|| Settings::read(|k| game.native_setting(dh::ID, k)))
    })
}

pub(super) fn set_settings(game: &Game<'_>, settings: Settings) {
    for (key, value) in settings.stored() {
        game.native_set_setting(dh::ID, key, &value);
    }
    SETTINGS.with_borrow_mut(|s| *s = Some(settings));
}

fn work_settings(game: &Game<'_>, settings: &Settings) -> WorkSettings {
    let dimension = game.dim.id();
    WorkSettings {
        render: RenderSettings {
            // `!levelWrapper.hasCeiling() && !isTheEnd()`.
            cave_culling: settings.cave_culling && dimension == "minecraft:overworld",
            cave_culling_height: MIN_Y + HEIGHT,
            min_y: MIN_Y,
        },
        generation: settings.generation,
        debug: settings.debug,
    }
}

/// The world generator: on the single player server thread when there is
/// one; a server the player joined has none here.
fn generator(game: &Game<'_>) -> Option<Arc<dyn LodSource>> {
    let dim = game.dim;
    match &game.integrated {
        Some(server) => server.call(move |g| g.lod_source_in(dim)).flatten(),
        None if game.remote.is_none() => Some(game.streaming.lod_source()),
        None => None,
    }
}

/// Keeps Distant Horizons' regenerable LOD cache in fast internal app storage (`HORIZON_PRIVATE`)
/// on Android so FUSE `/sdcard` writes don't trigger `MediaProvider` CPU spikes, while performing
/// a one-time copy of existing non-`.partial` `.dhs` files from `legacy_dir`.
fn resolve_dh_storage_dir(root: &std::path::Path, dimension: &str) -> std::path::PathBuf {
    let legacy_dir = root.join("data").join("DistantHorizons").join(dimension);
    #[cfg(target_os = "android")]
    if let Some(private) = std::env::var_os("HORIZON_PRIVATE").map(std::path::PathBuf::from) {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in root.to_string_lossy().as_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        let hint: String = root
            .to_string_lossy()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(24)
            .collect();
        let slug = format!("{hint}-{h:016x}");
        let private_dir = private.join("dh-cache").join(slug).join(dimension);
        if !private_dir.exists() && legacy_dir.exists() {
            copy_dh_dir(&legacy_dir, &private_dir);
        }
        return private_dir;
    }
    legacy_dir
}

#[cfg(target_os = "android")]
fn copy_dh_dir(src: &std::path::Path, dst: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(src) else {
        return;
    };
    let _ = std::fs::create_dir_all(dst);
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let target = dst.join(&name);
        if path.is_dir() {
            copy_dh_dir(&path, &target);
        } else if path.is_file() && !name.to_string_lossy().ends_with(".partial") {
            let _ = std::fs::copy(&path, &target);
        }
    }
}

fn start(game: &mut Game<'_>, key: WorldKey, settings: &Settings) -> Session {
    let facts = game.res.facts();
    let blocks = Arc::new(block_table(facts));
    let canonical = canonical_states(&game.pack.context.assets.registry, facts, &blocks);
    let fingerprint = fingerprint(facts);
    // The generator lives with the world: on the single player server
    // thread when there is one. A server the player joined has none here.
    let source = generator(game);
    // In single player the world's folder is the server's: the window's own
    // save folder is a placeholder.
    let root = match &game.integrated {
        Some(server) => server.call(|g| g.save.root.clone()),
        None if game.remote.is_none() => Some(key.root.clone()),
        None => None,
    };
    let dir = root.filter(|_| !key.remote).map(|root| {
        let dimension = key
            .dimension
            .strip_prefix("minecraft:")
            .unwrap_or(key.dimension);
        resolve_dh_storage_dir(&root, dimension)
    });
    let host = HorizonHost {
        context: game.pack.context.clone(),
        blocks,
        canonical: canonical.clone(),
        source: Mutex::new(source),
        styles: Mutex::new(HashMap::new()),
    };
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let engine = Engine::new(
        host,
        dir.clone(),
        fingerprint,
        work_settings(game, settings),
        dh::engine::worker_threads(cores),
    );
    match &dir {
        Some(dir) => log::info!(
            "Distant Horizons: started, keeping LODs in {}",
            dir.display()
        ),
        None => log::info!("Distant Horizons: started, keeping LODs in memory"),
    }
    Session {
        key,
        dir,
        engine,
        canonical,
        seen: HashMap::new(),
        last_generator: Instant::now(),
        pending: Vec::new(),
        last_scan: None,
        near_revision: None,
        loaded: Arc::new(HashSet::new()),
        staged_tiles: Vec::new(),
        staged_gone: Vec::new(),
        gpu_tile_cutouts: HashMap::new(),
        active_cutout: FxHashSet::default(),
    }
}

fn clear_session_lods(game: &mut Game<'_>, session: &Session) {
    if let Some(renderer) = &mut game.renderer {
        let removed: Vec<TileKey> = session.engine.shown_sections().map(tile).collect();
        renderer.queue_lods(Vec::new(), removed, Some(FxHashSet::default()));
        renderer.upload_lods(UPLOAD_BUDGET);
    }
    RADIUS.set(0);
}

/// Takes the session's tiles out of the renderer and ends it (its sections
/// are saved as the engine stops).
fn stop(game: &mut Game<'_>, session: Session) {
    clear_session_lods(game, &session);
    drop(session);
}

/// Clears rendered LOD tiles on the main thread and moves the blocking DH
/// worker shutdown & disk flush to a background thread.
pub(super) fn begin_async_stop(game: &mut Game<'_>) -> Option<std::thread::JoinHandle<()>> {
    let session = SESSION.with_borrow_mut(Option::take)?;
    clear_session_lods(game, &session);
    std::thread::Builder::new()
        .name("dh-save".into())
        .spawn(move || drop(session))
        .ok()
}

fn key_dist_sq(key: TileKey, cam_col: (i32, i32)) -> i64 {
    let [ox, _, oz] = key.origin();
    let half = key.size() / 2;
    let cx = (ox + half) >> 4;
    let cz = (oz + half) >> 4;
    let dx = i64::from(cx - cam_col.0);
    let dz = i64::from(cz - cam_col.1);
    dx * dx + dz * dz
}

fn compute_active_cutout(session: &Session, near: &Near) -> FxHashSet<(i32, i32)> {
    let mut candidates = FxHashSet::default();
    for &(cx, cz) in near.loaded.iter() {
        if near.covers(cx, cz) {
            candidates.insert((cx, cz));
        }
    }
    for cols in session.gpu_tile_cutouts.values() {
        for &(cx, cz) in cols {
            candidates.insert((cx, cz));
        }
    }
    for &(cx, cz) in &session.active_cutout {
        candidates.insert((cx, cz));
    }
    let max_level = dh::pos::ROOT_DETAIL_LEVEL - 4;
    let mut out = FxHashSet::default();
    for (cx, cz) in candidates {
        let mut covering = None;
        for level in 2..=max_level {
            let key = TileKey {
                x: cx >> level,
                z: cz >> level,
                level,
            };
            if let Some(cut) = session.gpu_tile_cutouts.get(&key) {
                covering = Some(cut);
                break;
            }
        }
        let keep = match covering {
            Some(cut) => cut.contains(&(cx, cz)),
            None => {
                near.covers(cx, cz)
                    || (session.active_cutout.contains(&(cx, cz))
                        && (cx - near.center.0).abs() <= near.radius + 8
                        && (cz - near.center.1).abs() <= near.radius + 8
                        && session.engine.waiting_for_tile_at(cx, cz))
            }
        };
        if keep {
            out.insert((cx, cz));
        }
    }
    out
}

/// Pause or resume Distant Horizons background LOD workers (e.g., while dual-screen lid is closed).
pub(super) fn set_paused(paused: bool) {
    SESSION.with_borrow(|s| {
        if let Some(session) = s.as_ref() {
            session.engine.set_paused(paused);
        }
    });
}

/// Once a frame (`native_frame`).
pub(super) fn frame(game: &mut Game<'_>) {
    if !game.native_window() {
        return;
    }
    add_commands(game);
    let settings = settings(game);
    let in_world = game.renderer.is_some() && game.player.is_some() && game.title.is_none();
    // Horizon's own far terrain (Video Settings > LOD Distance) draws in the
    // same place: Distant Horizons waits while it is on.
    let builtin = game.settings.horizon.lod_distance > 0;
    if builtin && in_world && settings.enabled && !NOTICE.get() {
        NOTICE.set(true);
        game.native_chat("Distant Horizons is paused while Video Settings > LOD Distance is on.");
    }
    let active = game.native_on(dh::ID) && settings.enabled && in_world && !builtin;
    let key = WorldKey {
        root: game.save.root.clone(),
        dimension: game.dim.id(),
        remote: game.remote.is_some() && game.integrated.is_none(),
    };
    let mut session = SESSION.take();
    if let Some(s) = session.take_if(|s| !active || s.key != key) {
        stop(game, s);
    }
    if !active {
        return;
    }
    let mut session = session.unwrap_or_else(|| start(game, key, &settings));
    session.engine.set_settings(work_settings(game, &settings));
    RADIUS.set(settings.radius_chunks());
    let needs_generator = settings.generation
        && session.engine.host().source.lock().unwrap().is_none()
        && session.last_generator.elapsed() > Duration::from_millis(250);
    if needs_generator {
        session.last_generator = Instant::now();
        if let Some(src) = generator(game) {
            *session.engine.host().source.lock().unwrap() = Some(src);
            session.engine.notify_generator_ready();
        }
    }
    scan_chunks(game, &mut session);
    let Some(player) = game.native_player() else {
        SESSION.set(Some(session));
        return;
    };
    let mut near_changed = false;
    if let Some(renderer) = game.renderer.as_ref() {
        let revision = renderer.lod_detail_revision();
        if session.near_revision != Some(revision) {
            session.near_revision = Some(revision);
            session.loaded = Arc::new(renderer.lod_detail_columns().collect());
            near_changed = true;
        }
    }
    let cam_col = (
        (player.x.floor() as i32) >> 4,
        (player.z.floor() as i32) >> 4,
    );
    let near = Near {
        center: cam_col,
        radius: game.streaming.radius,
        loaded: Arc::clone(&session.loaded),
    };
    let reduced = game.settings.horizon.far_detail == horizon_client::graphics::FarDetail::Reduced;
    let plan = DetailPlan::with_reduced(settings.quality, settings.radius_chunks(), reduced);
    let (meshes, removed) = session
        .engine
        .update([player.x, player.z], plan, near.clone());
    if let Some(renderer) = &mut game.renderer {
        for s in removed {
            let key = tile(s);
            session.staged_tiles.retain(|(k, _, _)| *k != key);
            if !session.staged_gone.contains(&key) {
                session.staged_gone.push(key);
            }
        }
        for m in meshes {
            let key = tile(m.section);
            match m.mesh {
                Some(mesh) => {
                    session.staged_gone.retain(|k| *k != key);
                    if let Some(pos) = session.staged_tiles.iter().position(|(k, _, _)| *k == key) {
                        session.staged_tiles[pos].1 = mesh;
                        session.staged_tiles[pos].2 = m.cutout;
                    } else {
                        session.staged_tiles.push((key, mesh, m.cutout));
                    }
                }
                None => {
                    session.staged_tiles.retain(|(k, _, _)| *k != key);
                    if !session.staged_gone.contains(&key) {
                        session.staged_gone.push(key);
                    }
                }
            }
        }
        if !renderer.has_pending_lods() {
            if !session.staged_tiles.is_empty() || !session.staged_gone.is_empty() {
                session
                    .staged_tiles
                    .sort_unstable_by_key(|(key, _, _)| (key.level, key_dist_sq(*key, cam_col)));
                let n = session.staged_tiles.len().min(16);
                let mut batch_tiles = Vec::with_capacity(n);
                for (key, mesh, cut) in session.staged_tiles.drain(..n) {
                    session.gpu_tile_cutouts.insert(key, cut);
                    batch_tiles.push((key, mesh));
                }
                let batch_gone = if session.staged_tiles.is_empty() {
                    let gone = std::mem::take(&mut session.staged_gone);
                    for k in &gone {
                        session.gpu_tile_cutouts.remove(k);
                    }
                    gone
                } else {
                    Vec::new()
                };
                let next_cutout = compute_active_cutout(&session, &near);
                let cutout_arg = (next_cutout != session.active_cutout).then(|| {
                    session.active_cutout = next_cutout.clone();
                    next_cutout
                });
                renderer.queue_lods(batch_tiles, batch_gone, cutout_arg);
            } else if near_changed || !session.active_cutout.is_empty() {
                let next_cutout = compute_active_cutout(&session, &near);
                if next_cutout != session.active_cutout {
                    session.active_cutout = next_cutout.clone();
                    renderer.queue_lods(Vec::new(), Vec::new(), Some(next_cutout));
                }
            }
        }
        renderer.upload_lods(UPLOAD_BUDGET);
    }
    SESSION.set(Some(session));
}

/// New and changed loaded chunks into the LODs, a few a frame.
fn scan_chunks(game: &mut Game<'_>, session: &mut Session) {
    let now = Instant::now();
    if session.pending.is_empty() && session.last_scan.is_none_or(|at| now - at >= SCAN_EVERY) {
        session.last_scan = Some(now);
        let chunks = &game.world.chunks;
        session.seen.retain(|k, _| chunks.contains_key(k));
        for (&key, chunk) in chunks.iter() {
            // Light must be in: the LODs keep each run's light.
            if !chunk.lit {
                continue;
            }
            let changed = match session.seen.get(&key) {
                None => true,
                // Read again when its blocks changed (the game remeshes
                // them), at most every few seconds. Its data's address also
                // moves with light updates alone: not a reason.
                Some(&at) => {
                    now - at >= REREAD_EVERY
                        && (0..chunk.sections.len() as i32)
                            .any(|sy| game.world.dirty_sections.contains(&(key.0, sy, key.1)))
                }
            };
            if changed {
                session.seen.insert(key, now);
                session.pending.push(key);
            }
        }
        // Nearest first.
        let (cx, cz) = game.streaming.center;
        session
            .pending
            .sort_unstable_by_key(|&(x, z)| std::cmp::Reverse((x - cx).pow(2) + (z - cz).pow(2)));
    }
    let air = game.res.facts().air.0;
    for _ in 0..CHUNKS_PER_FRAME {
        let Some(key) = session.pending.pop() else {
            break;
        };
        if let Some(chunk) = game.world.chunks.get(&key) {
            session.engine.add_chunk(Box::new(LoadedChunk {
                chunk: chunk.clone(),
                canonical: session.canonical.clone(),
                air,
            }));
        }
    }
}

/// `terrain_distance` as the renderer uses it: far enough for the LODs
/// while a session runs.
pub(super) fn terrain_distance(value: i32) -> i32 {
    value.max(RADIUS.get())
}

/// `/dh` is registered in `Game::native_commands` via `native_hooks.rs`.
fn add_commands(_game: &mut Game<'_>) {}

/// `tree` with `/dh` and its words (`distant_horizons::COMMANDS`), for the
/// chat's completion; the mod runs them before the tree is asked
/// (`native_command`). Unchanged when it has them already.
#[allow(dead_code)]
pub(super) fn with_commands(
    tree: &horizon_client::command::Dispatcher,
) -> Option<horizon_client::command::Dispatcher> {
    use horizon_client::command::MOD_COMMAND;
    if tree.find("dh").is_some() {
        return None;
    }
    let mut d = tree.clone();
    let root = d.literal(0, "dh");
    d.executes(root, MOD_COMMAND);
    for &(word, values) in dh::COMMANDS {
        let node = d.literal(root, word);
        if values.is_empty() {
            d.executes(node, MOD_COMMAND);
        }
        for value in values {
            let leaf = d.literal(node, value);
            d.executes(leaf, MOD_COMMAND);
        }
    }
    Some(d)
}

/// `/dh`: the status, or a setting changed.
pub(super) fn command(game: &mut Game<'_>, command: &str) -> bool {
    let mut words = command.split_whitespace();
    if !matches!(words.next(), Some("dh" | "distanthorizons")) {
        return false;
    }
    let args: Vec<&str> = words.collect();
    let mut settings = settings(game);
    let reply = match args.as_slice() {
        [] | ["status"] => {
            let session = SESSION.with_borrow(|s| {
                s.as_ref().map(|s| {
                    let generator = s.engine.host().source.lock().unwrap().is_some();
                    (s.engine.stats(), generator, s.dir.clone())
                })
            });
            match session {
                Some((s, generator, dir)) => format!(
                    "Distant Horizons {}: {} of {} sections drawn ({} faces), {} jobs queued, {} in memory, {} generated, {} chunks read; generator {}; near {} chunks around {:?}; LODs {}",
                    dh::VERSION,
                    s.shown,
                    s.wanted,
                    s.quads,
                    s.queued,
                    s.loaded,
                    s.generated,
                    s.chunks,
                    if generator { "on" } else { "none" },
                    game.streaming.radius,
                    game.streaming.center,
                    if dir.is_some() {
                        "saved in the world"
                    } else {
                        "in memory"
                    },
                ),
                None => format!("Distant Horizons {}: not running here", dh::VERSION),
            }
        }
        ["save"] => {
            SESSION.with_borrow(|s| s.as_ref().map(|s| s.engine.save()));
            "Distant Horizons: saved".to_owned()
        }
        ["clear"] => {
            SESSION.with_borrow_mut(|s| s.as_mut().map(|s| s.engine.wipe()));
            "Distant Horizons: LODs cleared; they are made again".to_owned()
        }
        _ => match settings.command(&args) {
            Some(reply) => {
                set_settings(game, settings);
                reply
            }
            None => format!(
                "/dh [{}]",
                dh::COMMANDS
                    .iter()
                    .map(|(w, v)| if v.is_empty() {
                        (*w).to_owned()
                    } else {
                        format!("{w} {}", v.join("|"))
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
        },
    };
    game.native_chat(&reply);
    true
}

/// The debug box: shown with the debug detail colours.
pub(super) fn hud(
    game: &mut Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    use horizon_client::extras::Corner;
    if !game.native_on(dh::ID)
        || !settings(game).debug
        || !game.native_corner_shown(Corner::BottomLeft)
    {
        return;
    }
    let Some(s) = SESSION.with_borrow(|s| s.as_ref().map(|s| s.engine.stats())) else {
        return;
    };
    let lines = [
        format!("DH {}: {}/{} sections", dh::VERSION, s.shown, s.wanted),
        format!(
            "{} queued, {} generated, {} chunks",
            s.queued, s.generated, s.chunks
        ),
    ];
    let w = lines
        .iter()
        .map(|l| game.native_text_width(l))
        .fold(0.0, f32::max)
        + 4.0;
    let h = 10.0 * lines.len() as f32 + 2.0;
    let (x, y) = corners.place(Corner::BottomLeft, w, h);
    game.native_fill(batch, [x, y, w, h], 0x7800_0000);
    for (i, line) in lines.iter().enumerate() {
        game.native_text(
            batch,
            line,
            [x + 2.0, y + 2.0 + 10.0 * i as f32],
            0xE0E0E0,
            true,
        );
    }
}
