//! A small CPU particle system, sprite-batched. Presets come from the
//! pack's `data/vfx.toml`; the simulation's [`Fx`] hints say where to play
//! them. Purely cosmetic: nothing here feeds back into the game.

use bevy::prelude::*;
use td_content::VfxPreset;
use td_core::Fx;
use td_mod_api::TICK_RATE;

use crate::Screen;
use crate::assets::Packs;
use crate::session::Session;
use crate::style::hex;

const MAX_PARTICLES: usize = 2500;
const Z_PARTICLE: f32 = 5.0;

pub fn plugin(app: &mut App) {
    app.init_resource::<Particles>()
        .add_systems(OnExit(Screen::Playing), |mut p: ResMut<Particles>| {
            *p = Particles::default()
        })
        .add_systems(
            Update,
            (emit, simulate)
                .chain()
                .after(crate::session::Advance)
                .run_if(in_state(Screen::Playing)),
        );
}

struct Particle {
    entity: Entity,
    pos: Vec2,
    vel: Vec2,
    age: f32,
    life: f32,
    size: (f32, f32),
    color: (LinearRgba, LinearRgba),
    gravity: f32,
    drag: f32,
}

#[derive(Resource, Default)]
struct Particles {
    list: Vec<Particle>,
    rng: u32,
    /// Seconds since the last effect-aura emission pass.
    aura_clock: f32,
}

impl Particles {
    fn rand(&mut self) -> f32 {
        // xorshift32; cosmetic only.
        if self.rng == 0 {
            self.rng = 0x9e37_79b9;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

fn spawn(
    commands: &mut Commands,
    parts: &mut Particles,
    packs: &Packs,
    preset: &VfxPreset,
    at: Vec2,
    dir: Option<Vec2>,
    scale: f32,
) {
    let c0 = hex(&preset.color).unwrap_or(Color::WHITE);
    let c1 = preset
        .color_end
        .as_deref()
        .and_then(hex)
        .unwrap_or(c0.with_alpha(0.0));
    let count = ((preset.count as f32) * scale).round().max(1.0) as usize;
    let base_angle = dir.map(|d| d.y.atan2(d.x)).unwrap_or(0.0);
    let image = packs
        .sprite(&preset.sprite)
        .unwrap_or_else(|| packs.dot.clone());
    for _ in 0..count {
        if parts.list.len() >= MAX_PARTICLES {
            return;
        }
        let spread = preset.spread.to_radians() as f32;
        let angle = base_angle + (parts.rand() - 0.5) * spread;
        let speed = preset.speed as f32 * (0.5 + parts.rand());
        let life = preset.lifetime as f32 * (0.7 + 0.6 * parts.rand());
        let size0 = preset.size as f32 * (0.8 + 0.4 * parts.rand());
        let size1 = preset.size_end.map(|s| s as f32).unwrap_or(size0);
        let entity = commands
            .spawn((
                Sprite {
                    image: image.clone(),
                    color: c0,
                    custom_size: Some(Vec2::splat(size0 * 2.0)),
                    ..default()
                },
                Transform::from_xyz(at.x, at.y, Z_PARTICLE),
                DespawnOnExit(Screen::Playing),
            ))
            .id();
        parts.list.push(Particle {
            entity,
            pos: at,
            vel: Vec2::from_angle(angle) * speed,
            age: 0.0,
            life,
            size: (size0, size1),
            color: (c0.to_linear(), c1.to_linear()),
            gravity: preset.gravity as f32,
            drag: preset.drag as f32,
        });
    }
}

/// A beam: particles along a jagged line, for lightning.
fn beam(
    commands: &mut Commands,
    parts: &mut Particles,
    packs: &Packs,
    preset: &VfxPreset,
    from: Vec2,
    to: Vec2,
) {
    let n = (from.distance(to) * 6.0).ceil().max(2.0) as usize;
    let normal = (to - from).perp().normalize_or_zero();
    let mut still = preset.clone();
    still.speed = 0.05;
    still.count = 1;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let jitter = if i == 0 || i == n {
            0.0
        } else {
            (parts.rand() - 0.5) * 0.35
        };
        spawn(
            commands,
            parts,
            packs,
            &still,
            from.lerp(to, t) + normal * jitter,
            None,
            1.0,
        );
    }
}

fn emit(
    mut commands: Commands,
    time: Res<Time>,
    mut s: ResMut<Session>,
    packs: Res<Packs>,
    mut parts: ResMut<Particles>,
) {
    let lib = packs.lib.clone();
    let content = s.game.sim.content.clone();
    let preset = |id: &Option<String>| id.as_ref().and_then(|i| lib.vfx.get(i));
    let named = |id: &str| lib.vfx.get(id);
    let fx: Vec<Fx> = std::mem::take(&mut s.fx);
    for f in fx {
        match f {
            Fx::Attack {
                kind,
                lane,
                from,
                to,
                ..
            } => {
                let k = &content.structures[kind];
                if let Some(p) = preset(&k.vfx.as_ref().and_then(|v| v.attack.clone())) {
                    let (a, b) = (
                        s.to_world(lane, from.0, from.1),
                        s.to_world(lane, to.0, to.1),
                    );
                    if p.style == "beam" {
                        beam(&mut commands, &mut parts, &packs, p, a, b);
                    } else {
                        spawn(&mut commands, &mut parts, &packs, p, a, Some(b - a), 1.0);
                    }
                }
            }
            Fx::Hit {
                kind,
                lane,
                at,
                splash,
            } => {
                let k = &content.structures[kind];
                if let Some(p) = preset(&k.vfx.as_ref().and_then(|v| v.hit.clone())) {
                    let scale = 1.0 + splash as f32;
                    spawn(
                        &mut commands,
                        &mut parts,
                        &packs,
                        p,
                        s.to_world(lane, at.0, at.1),
                        None,
                        scale,
                    );
                }
            }
            Fx::Bounce {
                kind,
                lane,
                from,
                to,
            } => {
                let k = &content.structures[kind];
                let id = k
                    .vfx
                    .as_ref()
                    .and_then(|v| v.attack.clone().or(v.hit.clone()));
                if let Some(p) = preset(&id) {
                    let (a, b) = (
                        s.to_world(lane, from.0, from.1),
                        s.to_world(lane, to.0, to.1),
                    );
                    if p.style == "beam" {
                        beam(&mut commands, &mut parts, &packs, p, a, b);
                    } else {
                        spawn(&mut commands, &mut parts, &packs, p, b, None, 1.0);
                    }
                }
            }
            Fx::UnitAttack {
                unit,
                lane,
                from,
                to,
            } => {
                let id = s
                    .game
                    .sim
                    .units
                    .get(&unit)
                    .and_then(|u| content.units[u.kind].vfx.as_ref())
                    .and_then(|v| v.attack.clone());
                if let Some(p) = preset(&id).or_else(|| named("debris")) {
                    let (a, b) = (
                        s.to_world(lane, from.0, from.1),
                        s.to_world(lane, to.0, to.1),
                    );
                    spawn(&mut commands, &mut parts, &packs, p, b, Some(a - b), 0.4);
                }
            }
            Fx::UnitDied { lane, at, .. } => {
                if let Some(p) = named("death") {
                    spawn(
                        &mut commands,
                        &mut parts,
                        &packs,
                        p,
                        s.to_world(lane, at.0, at.1),
                        None,
                        1.0,
                    );
                }
            }
            Fx::UnitLeaked { lane, at, .. } => {
                if let Some(p) = named("leak") {
                    spawn(
                        &mut commands,
                        &mut parts,
                        &packs,
                        p,
                        s.to_world(lane, at.0, at.1),
                        None,
                        1.0,
                    );
                }
            }
            Fx::StructureRemoved {
                kind,
                lane,
                at,
                destroyed,
            } => {
                let size = content.structures[kind].size as f64 / 2.0;
                let p = if destroyed {
                    named("debris")
                } else {
                    named("smoke")
                };
                if let Some(p) = p {
                    spawn(
                        &mut commands,
                        &mut parts,
                        &packs,
                        p,
                        s.to_world(lane, at.0 as f64 + size, at.1 as f64 + size),
                        None,
                        1.5,
                    );
                }
            }
            Fx::StructurePlaced { kind, lane, at } => {
                let size = content.structures[kind].size as f64 / 2.0;
                if let Some(p) = named("smoke") {
                    spawn(
                        &mut commands,
                        &mut parts,
                        &packs,
                        p,
                        s.to_world(lane, at.0 as f64 + size, at.1 as f64 + size),
                        None,
                        0.8,
                    );
                }
            }
        }
    }

    // Trails behind projectiles.
    for p in &s.game.sim.projectiles {
        let k = &content.structures[p.kind];
        if let Some(preset) = preset(&k.vfx.as_ref().and_then(|v| v.projectile.clone())) {
            let pos = s.to_world(p.lane, p.x, p.y);
            let rate = preset.count as f32 * time.delta_secs() * TICK_RATE as f32 * 0.5;
            if parts.rand() < rate.fract() || rate >= 1.0 {
                spawn(
                    &mut commands,
                    &mut parts,
                    &packs,
                    preset,
                    pos,
                    None,
                    rate.max(1.0) / preset.count.max(1) as f32,
                );
            }
        }
    }

    // Effects with a vfx shimmer on the units they affect.
    parts.aura_clock += time.delta_secs();
    if parts.aura_clock >= 0.2 {
        parts.aura_clock = 0.0;
        for u in s.game.sim.units.values() {
            for e in &u.effects {
                if let Some(p) = preset(&e.def.vfx) {
                    let pos = s.to_world(u.lane, u.x, u.y);
                    spawn(&mut commands, &mut parts, &packs, p, pos, None, 0.35);
                }
            }
        }
    }
}

fn simulate(
    mut commands: Commands,
    time: Res<Time>,
    s: Res<Session>,
    mut parts: ResMut<Particles>,
    mut q: Query<(&mut Transform, &mut Sprite)>,
) {
    let dt = if s.paused {
        0.0
    } else {
        time.delta_secs() * s.speed.min(2.0)
    };
    parts.list.retain_mut(|p| {
        p.age += dt;
        if p.age >= p.life {
            commands.entity(p.entity).despawn();
            return false;
        }
        p.vel *= 1.0 / (1.0 + p.drag * dt);
        // Positive gravity pulls down the screen.
        p.vel.y -= p.gravity * dt;
        p.pos += p.vel * dt;
        let t = p.age / p.life;
        if let Ok((mut tr, mut sprite)) = q.get_mut(p.entity) {
            tr.translation.x = p.pos.x;
            tr.translation.y = p.pos.y;
            let size = p.size.0 + (p.size.1 - p.size.0) * t;
            sprite.custom_size = Some(Vec2::splat(size * 2.0));
            let c = p.color.0.mix(&p.color.1, t);
            sprite.color = c.into();
        }
        true
    });
}
