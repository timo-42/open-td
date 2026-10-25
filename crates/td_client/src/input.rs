//! Camera, mouse and keyboard. Everything the player does becomes a
//! `Command` queued on the session; nothing here touches the simulation
//! state directly.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use td_core::TileFlags;
use td_core::sim::UnitMode;
use td_mod_api::{Cell, Command, StructureId, Targeting, UnitId};

use crate::Screen;
use crate::session::Session;

pub fn plugin(app: &mut App) {
    app.init_resource::<UiState>()
        .add_systems(Startup, spawn_camera)
        .add_systems(OnEnter(Screen::Playing), (reset, fit_camera))
        .add_systems(
            Update,
            (camera_controls, hover, clicks, hotkeys)
                .chain()
                .before(crate::session::Advance)
                .run_if(in_state(Screen::Playing)),
        );
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    #[default]
    None,
    /// Place this structure kind.
    Build(usize),
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    #[default]
    None,
    Structure(StructureId),
    Unit(UnitId),
}

#[derive(Resource, Default)]
pub struct UiState {
    pub tool: Tool,
    pub selection: Selection,
    pub hover: Option<Cell>,
    pub cursor_world: Option<Vec2>,
    /// Right-button drag distance, to tell a click from a pan.
    drag: f32,
}

fn reset(mut ui: ResMut<UiState>) {
    *ui = UiState::default();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scale: 1.0 / 40.0,
            ..OrthographicProjection::default_2d()
        }),
    ));
}

fn fit_camera(
    s: Res<Session>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cam: Single<(&mut Transform, &mut Projection), With<Camera2d>>,
) {
    // Frame the player's own lane; the rest is a scroll away.
    let (min, max) = match s.my_lane() {
        Some(l) => {
            let lane = &s.game.sim.lanes[l as usize];
            let o = s.layout[l as usize];
            (
                Vec2::new(o.x, o.y - lane.height as f32),
                Vec2::new(o.x + lane.width as f32, o.y),
            )
        }
        None => s.bounds(),
    };
    let size = (max - min) + Vec2::new(2.0, 6.0);
    let scale = (size.x / window.width()).max(size.y / window.height());
    let center = (min + max) / 2.0;
    cam.0.translation = Vec3::new(center.x, center.y - 0.8, cam.0.translation.z);
    if let Projection::Orthographic(o) = &mut *cam.1 {
        o.scale = scale;
    }
}

/// Whether the mouse is over a HUD panel or button.
pub fn pointer_over_ui(hover: &HoverMap, nodes: &Query<(), With<Node>>) -> bool {
    hover
        .get(&PointerId::Mouse)
        .is_some_and(|hits| hits.keys().any(|e| nodes.contains(*e)))
}

#[allow(clippy::too_many_arguments)]
fn camera_controls(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    motion: Res<AccumulatedMouseMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cam: Single<(&mut Transform, &mut Projection, &Camera, &GlobalTransform), With<Camera2d>>,
    hover: Res<HoverMap>,
    nodes: Query<(), With<Node>>,
    mut ui: ResMut<UiState>,
) {
    let (ref mut transform, ref mut projection, camera, global) = *cam;
    let Projection::Orthographic(ortho) = &mut **projection else {
        return;
    };

    let mut pan = Vec2::ZERO;
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        pan.x -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        pan.x += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        pan.y += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        pan.y -= 1.0;
    }
    transform.translation += (pan * 700.0 * ortho.scale * time.delta_secs()).extend(0.0);

    if mouse.pressed(MouseButton::Middle) || mouse.pressed(MouseButton::Right) {
        let d = motion.delta;
        ui.drag += d.length();
        if mouse.pressed(MouseButton::Middle) || ui.drag > 6.0 {
            transform.translation += Vec3::new(-d.x, d.y, 0.0) * ortho.scale;
        }
    }
    if mouse.just_pressed(MouseButton::Right) {
        ui.drag = 0.0;
    }

    if scroll.delta.y != 0.0 && !pointer_over_ui(&hover, &nodes) {
        let steps = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        let factor = 0.88f32.powf(steps);
        let before = window
            .cursor_position()
            .and_then(|c| camera.viewport_to_world_2d(global, c).ok());
        let new_scale = (ortho.scale * factor).clamp(1.0 / 160.0, 1.0 / 6.0);
        // Zoom toward the cursor.
        if let Some(p) = before {
            let t = Vec2::new(transform.translation.x, transform.translation.y);
            let moved = p + (t - p) * (new_scale / ortho.scale);
            transform.translation.x = moved.x;
            transform.translation.y = moved.y;
        }
        ortho.scale = new_scale;
    }
}

fn hover(
    s: Res<Session>,
    window: Single<&Window, With<PrimaryWindow>>,
    cam: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    hover: Res<HoverMap>,
    nodes: Query<(), With<Node>>,
    mut ui: ResMut<UiState>,
) {
    let world = window
        .cursor_position()
        .and_then(|c| cam.0.viewport_to_world_2d(cam.1, c).ok());
    ui.cursor_world = world;
    ui.hover = if pointer_over_ui(&hover, &nodes) {
        None
    } else {
        world.and_then(|p| s.to_cell(p))
    };
    // Forget selections that no longer exist.
    let gone = match ui.selection {
        Selection::Structure(id) => !s.game.sim.structures.contains_key(&id),
        Selection::Unit(id) => !s.game.sim.units.contains_key(&id),
        Selection::None => false,
    };
    if gone {
        ui.selection = Selection::None;
    }
}

/// An idle builder of `me` closest to `cell`, or any of their builders.
pub fn pick_builder(s: &Session, cell: Cell, selected: Selection) -> Option<UnitId> {
    let sim = &s.game.sim;
    let mine = |u: &&td_core::sim::Unit| {
        u.owner == s.me && sim.content.units[u.kind].moves_on.contains(TileFlags::UNIT)
    };
    if let Selection::Unit(id) = selected
        && let Some(u) = sim.units.get(&id)
        && mine(&u)
        && u.lane == cell.lane
    {
        return Some(id);
    }
    let d = |u: &td_core::sim::Unit| {
        let (dx, dy) = (u.x - cell.x as f64, u.y - cell.y as f64);
        dx * dx + dy * dy
    };
    let candidates: Vec<&td_core::sim::Unit> = sim
        .units
        .values()
        .filter(mine)
        .filter(|u| u.lane == cell.lane)
        .collect();
    candidates
        .iter()
        .filter(|u| matches!(u.mode, UnitMode::Idle))
        .min_by(|a, b| d(a).total_cmp(&d(b)))
        .or_else(|| candidates.iter().min_by(|a, b| d(a).total_cmp(&d(b))))
        .map(|u| u.id)
}

fn clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut s: ResMut<Session>,
    mut ui: ResMut<UiState>,
    hover: Res<HoverMap>,
    nodes: Query<(), With<Node>>,
) {
    if pointer_over_ui(&hover, &nodes) || s.game.is_over() {
        return;
    }
    if mouse.just_released(MouseButton::Right) && ui.drag <= 6.0 {
        // Right click: move the selected builder, else cancel.
        if let (Selection::Unit(id), Some(cell)) = (ui.selection, ui.hover)
            && s.game.sim.units.get(&id).is_some_and(|u| u.owner == s.me)
            && ui.tool == Tool::None
        {
            s.queue(Command::OrderMove { unit: id, to: cell });
        } else {
            ui.tool = Tool::None;
            ui.selection = Selection::None;
        }
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(cell) = ui.hover else {
        ui.selection = Selection::None;
        return;
    };
    match ui.tool {
        Tool::Build(kind) => {
            let me = s.me;
            let builder = if s.game.sim.requires_builder(me) {
                pick_builder(&s, cell, ui.selection)
            } else {
                None
            };
            if s.game.sim.requires_builder(me) && builder.is_none() {
                s.say("no builder on this lane");
                return;
            }
            let id = s.game.sim.content.structures[kind].id.clone();
            s.queue(Command::Build {
                kind: id,
                at: cell,
                owner: me,
                builder,
            });
            if !keys.pressed(KeyCode::ShiftLeft) && !keys.pressed(KeyCode::ShiftRight) {
                ui.tool = Tool::None;
            }
        }
        Tool::None => {
            let p = ui.cursor_world.unwrap_or_default();
            // Units under the cursor first (builders), then structures.
            let unit = s.game.sim.units.values().find(|u| {
                let r = s.game.sim.content.units[u.kind].size.radius() as f32 + 0.1;
                u.lane == cell.lane && s.to_world(u.lane, u.x, u.y).distance(p) <= r
            });
            ui.selection = match (unit, s.game.sim.structure_at(cell)) {
                (Some(u), _) => Selection::Unit(u.id),
                (None, Some(st)) => Selection::Structure(st.id),
                (None, None) => Selection::None,
            };
        }
    }
}

pub fn next_targeting(t: Targeting) -> Targeting {
    let i = Targeting::ALL.iter().position(|x| *x == t).unwrap_or(0);
    Targeting::ALL[(i + 1) % Targeting::ALL.len()]
}

fn hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    mut s: ResMut<Session>,
    mut ui: ResMut<UiState>,
    palette: Res<crate::hud::Palette>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        ui.tool = Tool::None;
        ui.selection = Selection::None;
    }
    if keys.just_pressed(KeyCode::Space) {
        s.paused = !s.paused;
    }
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        s.speed = (s.speed * 2.0).min(8.0);
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        s.speed = (s.speed / 2.0).max(0.25);
    }
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
        KeyCode::Digit0,
    ];
    for (i, key) in digits.iter().enumerate() {
        if keys.just_pressed(*key)
            && let Some(&kind) = palette.0.get(i)
        {
            ui.tool = Tool::Build(kind);
        }
    }
    if let Selection::Structure(id) = ui.selection
        && let Some(st) = s.game.sim.structures.get(&id)
        && st.owner == s.me
    {
        let upgrade = s.game.sim.content.structures[st.kind]
            .upgrades_to
            .first()
            .cloned();
        let mode = next_targeting(st.targeting);
        if keys.just_pressed(KeyCode::KeyU)
            && let Some(to) = upgrade
        {
            s.queue(Command::Upgrade { structure: id, to });
        }
        if keys.just_pressed(KeyCode::Tab) {
            s.queue(Command::SetTargeting {
                structure: id,
                mode,
            });
        }
        if keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace) {
            s.queue(Command::Sell { structure: id });
            ui.selection = Selection::None;
        }
    }
}
