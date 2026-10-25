//! Main menu: pick a map or a generator, fill in the generator form, choose
//! the number of players (others are played by bots), start.

use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::picking::events::PointerClick;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use td_content::GeneratorDef;
use td_mod_api::Value;

use crate::assets::{LoadError, Packs};
use crate::session::{PendingStart, StartError};
use crate::style::{self, Active, PANEL, TEXT, TEXT_DIM, button, text};
use crate::{Options, Screen};

pub fn plugin(app: &mut App) {
    app.init_resource::<MenuState>()
        .init_resource::<StartError>()
        .add_observer(on_action)
        .add_systems(Startup, autostart)
        .add_systems(OnEnter(Screen::Menu), spawn_menu)
        .add_systems(
            Update,
            (details, mark_choice).run_if(in_state(Screen::Menu)),
        );
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Choice {
    Map(usize),
    Generator(usize),
}

#[derive(Resource)]
struct MenuState {
    choice: Choice,
    players: Option<u8>,
    /// Form values per generator.
    params: Vec<Value>,
    seed: u64,
}

impl Default for MenuState {
    fn default() -> Self {
        MenuState {
            choice: Choice::Map(0),
            players: None,
            params: Vec::new(),
            seed: 1,
        }
    }
}

#[derive(Component, Clone, Debug)]
enum MenuAction {
    Select(Choice),
    Players(i32),
    Param(String, f64),
    Cycle(String),
    RandomSeed,
    Play,
}

#[derive(Component, Default)]
struct Details(u64);

fn autostart(mut commands: Commands, options: Res<Options>, packs: Option<Res<Packs>>) {
    let (Some(id), Some(packs)) = (&options.map, packs) else {
        return;
    };
    match packs.lib.find_map(id) {
        Some(m) => commands.insert_resource(PendingStart {
            map: m.map.clone(),
            players: options.players,
            seed: options.seed,
        }),
        None => error!("no map `{id}`"),
    }
}

fn spawn_menu(
    mut commands: Commands,
    packs: Option<Res<Packs>>,
    error: Option<Res<LoadError>>,
    mut state: ResMut<MenuState>,
) {
    let root = commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::top(px(40)),
                row_gap: px(20),
                ..default()
            },
            DespawnOnExit(Screen::Menu),
        ))
        .id();
    commands.entity(root).with_children(|r| {
        r.spawn(text("open-td", 56.0, style::ACCENT));
        r.spawn(text("tower defense, rules as wasm", 16.0, TEXT_DIM));
    });
    let Some(packs) = packs else {
        let msg = error
            .map(|e| e.0.clone())
            .unwrap_or_else(|| "no packs".into());
        commands.entity(root).with_children(|r| {
            r.spawn(text(
                format!("Could not load the base pack:\n{msg}"),
                18.0,
                style::BAD,
            ));
        });
        return;
    };
    let lib = &packs.lib;
    if state.params.len() != lib.generators.len() {
        state.params = lib
            .generators
            .iter()
            .map(|g| g.def.resolve_params(&Value::Nil))
            .collect();
    }
    commands.entity(root).with_children(|r| {
        r.spawn(Node { column_gap: px(20), align_items: AlignItems::FlexStart, ..default() }).with_children(|row| {
            row.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                    padding: UiRect::all(px(14)),
                    width: px(280),
                    border_radius: BorderRadius::all(px(8)),
                    ..default()
                },
                BackgroundColor(PANEL),
            ))
            .with_children(|list| {
                list.spawn(text("Maps", 18.0, TEXT_DIM));
                for (i, m) in lib.maps.iter().enumerate() {
                    let label = if m.pack == td_content::pack::BASE_PACK { m.map.name.clone() } else { format!("{} ({})", m.map.name, m.pack) };
                    list.spawn((button(label, 16.0), MenuAction::Select(Choice::Map(i))));
                }
                if !lib.generators.is_empty() {
                    list.spawn(text("Generated", 18.0, TEXT_DIM));
                    for (i, g) in lib.generators.iter().enumerate() {
                        let label = g.def.name.clone().unwrap_or_else(|| g.def.id.clone());
                        list.spawn((button(label, 16.0), MenuAction::Select(Choice::Generator(i))));
                    }
                }
            });
            row.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(8),
                    padding: UiRect::all(px(16)),
                    width: px(420),
                    border_radius: BorderRadius::all(px(8)),
                    ..default()
                },
                BackgroundColor(PANEL),
                Details::default(),
            ));
        });
        r.spawn(text(
            "WASD / arrows / right-drag: pan   wheel: zoom   1-0: build   U: upgrade   Tab: targeting   Del: sell   Space: pause",
            14.0,
            TEXT_DIM,
        ));
    });
}

fn param_row(c: &mut ChildSpawnerCommands, def: &GeneratorDef, values: &Value) {
    for p in &def.params {
        let label = p.label.clone().unwrap_or_else(|| p.name.clone());
        let v = values.get(&p.name);
        c.spawn(Node {
            column_gap: px(8),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                text(label, 15.0, TEXT),
                Node {
                    width: px(90),
                    ..default()
                },
            ));
            match p.kind.as_str() {
                "choice" | "bool" => {
                    let shown = match v {
                        Value::Bool(b) => if *b { "yes" } else { "no" }.to_string(),
                        other => other.str_or("").to_string(),
                    };
                    row.spawn((button(shown, 15.0), MenuAction::Cycle(p.name.clone())));
                }
                _ => {
                    let big = if p.max.unwrap_or(100.0) - p.min.unwrap_or(0.0) > 50.0 {
                        10.0
                    } else {
                        5.0
                    };
                    row.spawn((
                        button(format!("-{big}"), 14.0),
                        MenuAction::Param(p.name.clone(), -big),
                    ));
                    row.spawn((button("-", 14.0), MenuAction::Param(p.name.clone(), -1.0)));
                    let shown = match v {
                        Value::Float(f) => format!("{f:.1}"),
                        other => other.i64_or(0).to_string(),
                    };
                    row.spawn((
                        text(shown, 16.0, style::ACCENT),
                        Node {
                            width: px(56),
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ));
                    row.spawn((button("+", 14.0), MenuAction::Param(p.name.clone(), 1.0)));
                    row.spawn((
                        button(format!("+{big}"), 14.0),
                        MenuAction::Param(p.name.clone(), big),
                    ));
                }
            }
        });
    }
}

fn details(
    mut commands: Commands,
    packs: Option<Res<Packs>>,
    state: Res<MenuState>,
    error: Res<StartError>,
    mut q: Query<(Entity, &mut Details)>,
) {
    let (Some(packs), Ok((e, mut d))) = (packs, q.single_mut()) else {
        return;
    };
    let mut h = DefaultHasher::new();
    (
        state.choice,
        state.players,
        format!("{:?}", state.params),
        &error.0,
    )
        .hash(&mut h);
    let k = h.finish();
    if d.0 == k {
        return;
    }
    d.0 = k;
    let lib = &packs.lib;
    commands.entity(e).despawn_children().with_children(|c| {
        let (name, default_players, modules) = match state.choice {
            Choice::Map(i) => {
                let m = &lib.maps[i].map;
                let modules = m
                    .modules
                    .clone()
                    .unwrap_or_else(|| lib.defaults.modules.clone());
                let waves = m.extra.get("wave").map(|w| w.as_array().len()).unwrap_or(0);
                c.spawn(text(m.name.clone(), 24.0, style::ACCENT));
                let lanes = m.lanes.len();
                c.spawn(text(
                    format!("{lanes} lane template(s), {waves} waves"),
                    15.0,
                    TEXT_DIM,
                ));
                (m.name.clone(), m.players, modules)
            }
            Choice::Generator(i) => {
                let g = &lib.generators[i];
                let name = g.def.name.clone().unwrap_or_else(|| g.def.id.clone());
                c.spawn(text(name.clone(), 24.0, style::ACCENT));
                param_row(c, &g.def, &state.params[i]);
                c.spawn((button("Random seed", 14.0), MenuAction::RandomSeed));
                let players = state.params[i].get("lanes").i64_or(1) as u8;
                (name, players, lib.defaults.modules.clone())
            }
        };
        let _ = name;
        c.spawn(text(
            format!("rules: {}", modules.join(", ")),
            15.0,
            TEXT_DIM,
        ));
        let players = state.players.unwrap_or(default_players);
        c.spawn(Node {
            column_gap: px(8),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                text("players", 15.0, TEXT),
                Node {
                    width: px(90),
                    ..default()
                },
            ));
            row.spawn((button("-", 14.0), MenuAction::Players(-1)));
            row.spawn((
                text(players.to_string(), 16.0, style::ACCENT),
                Node {
                    width: px(40),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
            ));
            row.spawn((button("+", 14.0), MenuAction::Players(1)));
        });
        if players > 1 {
            c.spawn(text(
                "You are player 1; bots play the others.",
                14.0,
                TEXT_DIM,
            ));
        }
        c.spawn((button("Play", 22.0), MenuAction::Play))
            .insert(Node {
                margin: UiRect::top(px(10)),
                padding: UiRect::axes(px(30), px(10)),
                ..style::button_node()
            });
        if let Some(err) = &error.0 {
            c.spawn(text(err.clone(), 15.0, style::BAD));
        }
    });
}

fn mark_choice(
    mut commands: Commands,
    state: Res<MenuState>,
    q: Query<(Entity, &MenuAction, Has<Active>)>,
) {
    for (e, a, active) in &q {
        let want = matches!(a, MenuAction::Select(c) if *c == state.choice);
        if want != active {
            if want {
                commands.entity(e).insert(Active);
            } else {
                commands.entity(e).remove::<Active>();
            }
        }
    }
}

fn on_action(
    ev: On<PointerClick>,
    actions: Query<&MenuAction>,
    mut state: ResMut<MenuState>,
    packs: Option<Res<Packs>>,
    mut commands: Commands,
    mut error: ResMut<StartError>,
) {
    if ev.button != PointerButton::Primary {
        return;
    }
    let (Ok(action), Some(packs)) = (actions.get(ev.entity), packs) else {
        return;
    };
    let lib = &packs.lib;
    match action.clone() {
        MenuAction::Select(c) => {
            state.choice = c;
            state.players = None;
            error.0 = None;
        }
        MenuAction::Players(d) => {
            let base = state.players.unwrap_or(match state.choice {
                Choice::Map(i) => lib.maps[i].map.players,
                Choice::Generator(i) => state.params[i].get("lanes").i64_or(1) as u8,
            });
            state.players = Some((base as i32 + d).clamp(1, 8) as u8);
        }
        MenuAction::Param(name, d) => {
            if let Choice::Generator(i) = state.choice {
                let def = &lib.generators[i].def;
                let v = state.params[i].get(&name).f64_or(0.0) + d;
                let mut given = state.params[i].clone();
                given.insert(
                    name.clone(),
                    if def
                        .params
                        .iter()
                        .any(|p| p.name == name && p.kind == "float")
                    {
                        Value::Float(v)
                    } else {
                        Value::Int(v as i64)
                    },
                );
                state.params[i] = def.resolve_params(&given);
                if name == "lanes" {
                    state.players = None;
                }
            }
        }
        MenuAction::Cycle(name) => {
            if let Choice::Generator(i) = state.choice {
                let def = &lib.generators[i].def;
                if let Some(p) = def.params.iter().find(|p| p.name == name) {
                    let cur = state.params[i].get(&name).clone();
                    let next = match (&cur, p.kind.as_str()) {
                        (Value::Bool(b), _) => Value::Bool(!b),
                        (Value::Str(s), _) => {
                            let at = p
                                .options
                                .iter()
                                .position(|o| o == s)
                                .map(|i| i + 1)
                                .unwrap_or(0);
                            Value::Str(
                                p.options
                                    .get(at % p.options.len().max(1))
                                    .cloned()
                                    .unwrap_or_default(),
                            )
                        }
                        _ => cur,
                    };
                    state.params[i].insert(name, next);
                }
            }
        }
        MenuAction::RandomSeed => {
            if let Choice::Generator(i) = state.choice {
                state.seed = state
                    .seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let seed = ((state.seed >> 33) % 1_000_000) as i64;
                state.params[i].insert("seed", Value::Int(seed));
            }
        }
        MenuAction::Play => {
            let map = match state.choice {
                Choice::Map(i) => Ok(lib.maps[i].map.clone()),
                Choice::Generator(i) => lib.generate(&lib.generators[i].def.id, &state.params[i]),
            };
            match map {
                Ok(map) => commands.insert_resource(PendingStart {
                    map,
                    players: state.players,
                    seed: state.seed,
                }),
                Err(e) => error.0 = Some(e),
            }
        }
    }
}
