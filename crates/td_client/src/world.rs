//! Draws the simulation: tiles, structures, units and projectiles as
//! sprites (coloured shapes unless the pack ships a PNG), ranges, paths
//! and the placement preview as gizmo overlays. One world unit is one tile.

use std::collections::HashMap;

use bevy::prelude::*;
use td_core::content::EffectKind;
use td_core::lane::UNREACHABLE;
use td_core::{Lane, TileFlags};
use td_mod_api::{Cell, StructureId, UnitId};

use crate::Screen;
use crate::assets::Packs;
use crate::input::{Selection, Tool, UiState};
use crate::session::Session;
use crate::style::{self, color_or, id_color};

const Z_TILE: f32 = 0.0;
const Z_STRUCTURE: f32 = 1.0;
const Z_UNIT: f32 = 2.0;
const Z_PROJECTILE: f32 = 3.0;
const Z_BAR: f32 = 0.5;

pub fn plugin(app: &mut App) {
    app.init_resource::<Views>()
        .init_resource::<PlacementCache>()
        .add_systems(OnEnter(Screen::Playing), spawn_lanes)
        .add_systems(OnExit(Screen::Playing), |mut v: ResMut<Views>| {
            *v = Views::default()
        })
        .add_systems(
            Update,
            (sync_structures, sync_units, sync_projectiles, overlays)
                .chain()
                .after(crate::session::Advance)
                .run_if(in_state(Screen::Playing)),
        );
}

#[derive(Resource, Default)]
struct Views {
    units: HashMap<UnitId, (Entity, (Entity, Entity))>,
    structures: HashMap<StructureId, (Entity, (Entity, Entity), usize)>,
    projectiles: HashMap<u32, Entity>,
}

#[derive(Resource, Default)]
struct PlacementCache {
    key: Option<(Cell, usize, u64)>,
    ok: bool,
}

fn spawn_lanes(mut commands: Commands, s: Res<Session>, packs: Res<Packs>) {
    let content = &s.game.sim.content;
    for lane in &s.game.sim.lanes {
        let o = s.layout[lane.id as usize];
        let (w, h) = (lane.width as f32, lane.height as f32);
        commands.spawn((
            Sprite::from_color(Color::srgb(0.03, 0.03, 0.04), Vec2::new(w + 0.3, h + 0.3)),
            Transform::from_xyz(o.x + w / 2.0, o.y - h / 2.0, Z_TILE - 0.1),
            DespawnOnExit(Screen::Playing),
        ));
        for y in 0..lane.height as i32 {
            for x in 0..lane.width as i32 {
                let kind = &content.tiles[lane.tiles[lane.idx(x, y)]];
                let p = s.tile_center(lane.id, x, y);
                let sprite = match packs.sprite(&kind.sprite) {
                    Some(image) => Sprite {
                        image,
                        custom_size: Some(Vec2::ONE),
                        ..default()
                    },
                    None => {
                        // A slightly smaller square shows the grid.
                        let c = color_or(&kind.color, id_color(&kind.id));
                        Sprite::from_color(c, Vec2::splat(0.96))
                    }
                };
                commands.spawn((
                    sprite,
                    Transform::from_xyz(p.x, p.y, Z_TILE),
                    DespawnOnExit(Screen::Playing),
                ));
            }
        }
    }
}

/// A health/progress bar: a dark background with the fill as its child,
/// so hiding the background hides both. Returns (background, fill).
fn bar(commands: &mut ChildSpawnerCommands, width: f32, y: f32) -> (Entity, Entity) {
    let mut fill = Entity::PLACEHOLDER;
    let bg = commands
        .spawn((
            Sprite::from_color(
                Color::srgba(0.0, 0.0, 0.0, 0.7),
                Vec2::new(width + 0.04, 0.09),
            ),
            Transform::from_xyz(0.0, y, Z_BAR),
            Visibility::Hidden,
        ))
        .with_children(|c| {
            fill = c
                .spawn((
                    Sprite::from_color(style::GOOD, Vec2::new(width, 0.06)),
                    Transform::from_xyz(0.0, 0.0, 0.01),
                ))
                .id();
        })
        .id();
    (bg, fill)
}

type ViewQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Sprite,
        &'static mut Visibility,
    ),
>;

fn set_bar(
    q: &mut ViewQuery,
    (bg, fill): (Entity, Entity),
    frac: f32,
    width: f32,
    show: bool,
    color: Option<Color>,
) {
    if let Ok(mut b) = q.get_mut(bg) {
        let v = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *b.2 != v {
            *b.2 = v;
        }
    }
    if !show {
        return;
    }
    if let Ok(mut f) = q.get_mut(fill) {
        let frac = frac.clamp(0.0, 1.0);
        f.0.scale.x = frac.max(0.001);
        f.0.translation.x = -width * (1.0 - frac) / 2.0;
        f.1.color = color.unwrap_or(if frac > 0.5 {
            style::GOOD
        } else if frac > 0.25 {
            style::ACCENT
        } else {
            style::BAD
        });
    }
}

fn sync_structures(
    mut commands: Commands,
    s: Res<Session>,
    packs: Res<Packs>,
    mut views: ResMut<Views>,
    mut q: ViewQuery,
    children: Query<&Children>,
) {
    let content = &s.game.sim.content;
    let views = &mut *views;
    views.structures.retain(|id, (e, _, _)| {
        let keep = s.game.sim.structures.contains_key(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    for st in s.game.sim.structures.values() {
        let k = &content.structures[st.kind];
        let size = k.size as f32;
        let entry = views.structures.get(&st.id).copied();
        let needs_spawn = entry.is_none_or(|(_, _, kind)| kind != st.kind);
        if needs_spawn {
            if let Some((e, _, _)) = entry {
                commands.entity(e).despawn();
            }
            let center = s.to_world(
                st.lane,
                st.x as f64 + size as f64 / 2.0,
                st.y as f64 + size as f64 / 2.0,
            );
            let color = color_or(&k.color, id_color(&k.id));
            let body = match packs.sprite(&k.sprite) {
                Some(image) => Sprite {
                    image,
                    custom_size: Some(Vec2::splat(size * 0.92)),
                    ..default()
                },
                None => Sprite::from_color(color.darker(0.12), Vec2::splat(size * 0.86)),
            };
            let mut fill = (Entity::PLACEHOLDER, Entity::PLACEHOLDER);
            let root = commands
                .spawn((
                    body,
                    Transform::from_xyz(center.x, center.y, Z_STRUCTURE),
                    DespawnOnExit(Screen::Playing),
                ))
                .with_children(|c| {
                    if packs.sprite(&k.sprite).is_none() {
                        if k.can_attack() {
                            // A turret disc marks towers; walls stay plain blocks.
                            c.spawn((
                                Sprite {
                                    image: packs.circle.clone(),
                                    color: color.lighter(0.1),
                                    custom_size: Some(Vec2::splat(size * 0.55)),
                                    ..default()
                                },
                                Transform::from_xyz(0.0, 0.0, 0.05),
                            ));
                        } else if k.aura.is_some() {
                            c.spawn((
                                Sprite {
                                    image: packs.circle.clone(),
                                    color: color.with_alpha(0.6),
                                    custom_size: Some(Vec2::splat(size * 0.4)),
                                    ..default()
                                },
                                Transform::from_xyz(0.0, 0.0, 0.05),
                            ));
                        }
                    }
                    fill = bar(c, size * 0.8, size * 0.5 - 0.08);
                })
                .id();
            views.structures.insert(st.id, (root, fill, st.kind));
            continue;
        }
        let (root, fill, _) = entry.unwrap();
        // Under construction: translucent, with a progress bar.
        let building = st.constructing > 0;
        if let Ok(mut v) = q.get_mut(root) {
            let alpha = if building { 0.45 } else { 1.0 };
            if v.1.color.alpha() != alpha {
                v.1.color.set_alpha(alpha);
                if let Ok(kids) = children.get(root) {
                    for kid in kids {
                        if let Ok(mut kv) = q.get_mut(*kid)
                            && kv.1.image == packs.circle
                        {
                            kv.1.color.set_alpha(alpha * 0.9);
                        }
                    }
                }
            }
        }
        if building {
            let frac = 1.0 - st.constructing as f32 / st.construct_total.max(1) as f32;
            set_bar(
                &mut q,
                fill,
                frac,
                size * 0.8,
                true,
                Some(Color::srgb(0.5, 0.7, 1.0)),
            );
        } else {
            let (hp, max) = (st.hp.unwrap_or(1.0), st.max_hp.unwrap_or(1.0));
            set_bar(
                &mut q,
                fill,
                (hp / max) as f32,
                size * 0.8,
                st.hp.is_some() && hp < max,
                None,
            );
        }
    }
}

fn sync_units(
    mut commands: Commands,
    s: Res<Session>,
    packs: Res<Packs>,
    mut views: ResMut<Views>,
    mut q: ViewQuery,
) {
    let content = &s.game.sim.content;
    let views = &mut *views;
    views.units.retain(|id, (e, _)| {
        let keep = s.game.sim.units.contains_key(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    for u in s.game.sim.units.values() {
        let k = &content.units[u.kind];
        let r = k.size.radius() as f32;
        let cur = Vec2::new(u.x as f32, u.y as f32);
        let pos = s
            .prev_units
            .get(&u.id)
            .map(|p| p.lerp(cur, s.alpha))
            .unwrap_or(cur);
        let world = s.to_world(u.lane, pos.x as f64, pos.y as f64);
        let base = color_or(&k.color, id_color(&k.id));
        // Effects tint the body: frost blue, poison green, stun yellow.
        let tint = if u.effect(EffectKind::Stun).is_some() {
            base.mix(&Color::srgb(1.0, 0.95, 0.4), 0.6)
        } else if u.effect(EffectKind::Slow).is_some() {
            base.mix(&Color::srgb(0.55, 0.8, 1.0), 0.45)
        } else if u.effect(EffectKind::Dot).is_some() {
            base.mix(&Color::srgb(0.5, 0.95, 0.3), 0.35)
        } else {
            base
        };
        let Some(&(root, fill)) = views.units.get(&u.id) else {
            let sprite = match packs.sprite(&k.sprite) {
                Some(image) => Sprite {
                    image,
                    custom_size: Some(Vec2::splat(r * 2.4)),
                    ..default()
                },
                None => Sprite {
                    image: packs.circle.clone(),
                    color: tint,
                    custom_size: Some(Vec2::splat(r * 2.0)),
                    ..default()
                },
            };
            let z = Z_UNIT
                + if k.moves_on.contains(TileFlags::AIR) {
                    0.3
                } else {
                    0.0
                };
            let mut fill = (Entity::PLACEHOLDER, Entity::PLACEHOLDER);
            let root = commands
                .spawn((
                    sprite,
                    Transform::from_xyz(world.x, world.y, z),
                    DespawnOnExit(Screen::Playing),
                ))
                .with_children(|c| fill = bar(c, (r * 2.0).max(0.4), r + 0.1))
                .id();
            views.units.insert(u.id, (root, fill));
            continue;
        };
        if let Ok(mut v) = q.get_mut(root) {
            v.0.translation.x = world.x;
            v.0.translation.y = world.y;
            if packs.sprite(&k.sprite).is_none() {
                v.1.color = tint;
            } else {
                v.1.flip_x = u.facing.0 < 0.0;
            }
        }
        set_bar(
            &mut q,
            fill,
            (u.hp / u.max_hp) as f32,
            (r * 2.0).max(0.4),
            u.hp < u.max_hp,
            None,
        );
    }
}

fn sync_projectiles(
    mut commands: Commands,
    s: Res<Session>,
    packs: Res<Packs>,
    mut views: ResMut<Views>,
    mut q: Query<&mut Transform>,
) {
    let content = &s.game.sim.content;
    let alive: std::collections::HashSet<u32> =
        s.game.sim.projectiles.iter().map(|p| p.id).collect();
    views.projectiles.retain(|id, e| {
        let keep = alive.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    for p in &s.game.sim.projectiles {
        let cur = Vec2::new(p.x as f32, p.y as f32);
        let pos = s
            .prev_projectiles
            .get(&p.id)
            .map(|q| q.lerp(cur, s.alpha))
            .unwrap_or(cur);
        let world = s.to_world(p.lane, pos.x as f64, pos.y as f64);
        match views.projectiles.get(&p.id) {
            Some(&e) => {
                if let Ok(mut t) = q.get_mut(e) {
                    t.translation.x = world.x;
                    t.translation.y = world.y;
                }
            }
            None => {
                let k = &content.structures[p.kind];
                let color = color_or(&k.color, id_color(&k.id)).lighter(0.25);
                let e = commands
                    .spawn((
                        Sprite {
                            image: packs.circle.clone(),
                            color,
                            custom_size: Some(Vec2::splat(0.16)),
                            ..default()
                        },
                        Transform::from_xyz(world.x, world.y, Z_PROJECTILE),
                        DespawnOnExit(Screen::Playing),
                    ))
                    .id();
                views.projectiles.insert(p.id, e);
            }
        }
    }
}

fn route(lane: &Lane, dist: &[u32], mask: TileFlags, from: (i32, i32)) -> Vec<(i32, i32)> {
    let mut out = vec![from];
    let (mut x, mut y) = from;
    for _ in 0..lane.base.len() {
        if lane.is_exit(x, y) || dist[lane.idx(x, y)] == UNREACHABLE {
            break;
        }
        match lane.next_step(dist, mask, x, y) {
            Some(n) => {
                (x, y) = n;
                out.push(n);
            }
            None => break,
        }
    }
    out
}

fn overlays(
    mut gizmos: Gizmos,
    mut s: ResMut<Session>,
    ui: Res<UiState>,
    mut cache: ResMut<PlacementCache>,
) {
    let content = s.game.sim.content.clone();
    // Current routes from every spawn, per movement mask in play.
    for lane in &s.game.sim.lanes {
        for f in lane
            .fields
            .iter()
            .filter(|f| f.guarded && !f.mask.contains(TileFlags::UNIT))
        {
            let color = if f.mask.contains(TileFlags::GROUND) {
                Color::srgba(1.0, 1.0, 0.9, 0.22)
            } else {
                Color::srgba(0.6, 0.85, 1.0, 0.18)
            };
            for &sp in &lane.spawns {
                let pts: Vec<Vec2> = route(lane, &f.dist, f.mask, sp)
                    .into_iter()
                    .map(|(x, y)| s.tile_center(lane.id, x, y))
                    .collect();
                gizmos.linestrip_2d(pts, color);
            }
        }
        for &(x, y) in &lane.spawns {
            gizmos.circle_2d(
                Isometry2d::from_translation(s.tile_center(lane.id, x, y)),
                0.42,
                Color::srgb(0.9, 0.3, 0.3),
            );
        }
        for &(x, y) in &lane.exits {
            gizmos.rect_2d(
                Isometry2d::from_translation(s.tile_center(lane.id, x, y)),
                Vec2::splat(0.84),
                Color::srgb(0.95, 0.85, 0.3),
            );
        }
    }

    match ui.selection {
        Selection::Structure(id) => {
            if let Some(st) = s.game.sim.structures.get(&id) {
                let k = &content.structures[st.kind];
                let size = k.size as f64;
                let c = s.to_world(st.lane, st.x as f64 + size / 2.0, st.y as f64 + size / 2.0);
                gizmos.rect_2d(
                    Isometry2d::from_translation(c),
                    Vec2::splat(k.size as f32),
                    style::ACCENT,
                );
                if k.range > 0.0 {
                    gizmos
                        .circle_2d(
                            Isometry2d::from_translation(c),
                            k.range as f32,
                            Color::srgba(1.0, 1.0, 1.0, 0.5),
                        )
                        .resolution(64);
                }
                if let Some(a) = &k.aura {
                    gizmos
                        .circle_2d(
                            Isometry2d::from_translation(c),
                            a.radius as f32,
                            Color::srgba(0.6, 1.0, 0.6, 0.5),
                        )
                        .resolution(64);
                }
            }
        }
        Selection::Unit(id) => {
            if let Some(u) = s.game.sim.units.get(&id) {
                let r = content.units[u.kind].size.radius() as f32;
                let p = s.to_world(u.lane, u.x, u.y);
                gizmos.circle_2d(Isometry2d::from_translation(p), r + 0.12, style::ACCENT);
                if let td_core::sim::UnitMode::Moving { path, step, .. } = &u.mode {
                    let mut pts = vec![p];
                    pts.extend(
                        path[*step..]
                            .iter()
                            .map(|&(x, y)| s.tile_center(u.lane, x, y)),
                    );
                    gizmos.linestrip_2d(pts, Color::srgba(0.95, 0.8, 0.4, 0.6));
                }
            }
        }
        Selection::None => {}
    }

    // Sites my builders are walking to or have queued.
    for u in s.game.sim.units.values().filter(|u| u.owner == s.me) {
        let current = match &u.mode {
            td_core::sim::UnitMode::Moving { job: Some(j), .. } => Some(j),
            _ => None,
        };
        for job in current.into_iter().chain(u.jobs.iter()) {
            let size = content.structures[job.kind].size as f64;
            let c = s.to_world(
                job.at.lane,
                job.at.x as f64 + size / 2.0,
                job.at.y as f64 + size / 2.0,
            );
            gizmos.rect_2d(
                Isometry2d::from_translation(c),
                Vec2::splat(size as f32 * 0.8),
                Color::srgba(0.6, 0.85, 1.0, 0.8),
            );
        }
    }

    let Some(cell) = ui.hover else { return };
    match ui.tool {
        Tool::Build(kind) => {
            let me = s.me;
            let key = (cell, kind, s.game.sim.tick);
            if cache.key != Some(key) {
                cache.ok = s.game.sim.check_placement(kind, cell, me).is_ok();
                cache.key = Some(key);
            }
            let k = &content.structures[kind];
            let size = k.size as f64;
            let c = s.to_world(
                cell.lane,
                cell.x as f64 + size / 2.0,
                cell.y as f64 + size / 2.0,
            );
            let affordable = s
                .game
                .sim
                .can_afford(td_mod_api::Scope::Player(me), &k.cost);
            let color = if cache.ok && affordable {
                style::GOOD
            } else {
                style::BAD
            };
            gizmos.rect_2d(
                Isometry2d::from_translation(c),
                Vec2::splat(k.size as f32 * 0.96),
                color,
            );
            if k.range > 0.0 {
                gizmos
                    .circle_2d(
                        Isometry2d::from_translation(c),
                        k.range as f32,
                        color.with_alpha(0.6),
                    )
                    .resolution(64);
            }
            if let Some(a) = &k.aura {
                gizmos
                    .circle_2d(
                        Isometry2d::from_translation(c),
                        a.radius as f32,
                        Color::srgba(0.6, 1.0, 0.6, 0.5),
                    )
                    .resolution(64);
            }
        }
        Tool::None => {
            gizmos.rect_2d(
                Isometry2d::from_translation(s.tile_center(cell.lane, cell.x, cell.y)),
                Vec2::splat(1.0),
                Color::srgba(1.0, 1.0, 1.0, 0.35),
            );
        }
    }
}
