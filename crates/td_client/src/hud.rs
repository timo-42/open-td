//! In-game HUD: resources, wave, messages, build palette, module actions
//! (next wave, buy builder, send creeps), the selection panel and the
//! game-over screen. Panels whose content changes are rebuilt when a key
//! derived from that content changes.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use bevy::picking::events::PointerClick;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use td_core::content::{EffectKind, StructureKind};
use td_mod_api::{Command, Scope, StructureId, TICK_RATE, Value};

use crate::Screen;
use crate::input::{Selection, Tool, UiState, next_targeting};
use crate::session::{PendingStart, Session};
use crate::style::{self, Active, Disabled, PANEL, TEXT, TEXT_DIM, button, text};

pub fn plugin(app: &mut App) {
    app.init_resource::<Palette>()
        .add_observer(on_action)
        .add_systems(OnEnter(Screen::Playing), spawn_hud)
        .add_systems(
            Update,
            (
                update_palette,
                top_bar,
                log_box,
                palette_bar,
                actions_bar,
                info_panel,
                end_overlay,
                mark_buttons,
            )
                .after(crate::session::Advance)
                .run_if(in_state(Screen::Playing)),
        );
}

/// Structure kinds the local player may build directly, in palette order.
#[derive(Resource, Default)]
pub struct Palette(pub Vec<usize>);

#[derive(Component, Clone, Debug)]
pub enum Action {
    Tool(usize),
    Pause,
    Speed(f32),
    Menu,
    Restart,
    Request(String, Value),
    Upgrade(StructureId, String),
    Sell(StructureId),
    Targeting(StructureId),
}

#[derive(Component)]
struct ResourcesText;
#[derive(Component)]
struct WaveText;
#[derive(Component)]
struct ClockText;
#[derive(Component, Default)]
struct LogBox(u64);
#[derive(Component, Default)]
struct PaletteBar(u64);
#[derive(Component, Default)]
struct ActionsBar(u64);
#[derive(Component, Default)]
struct InfoPanel(u64);
#[derive(Component, Default)]
struct EndOverlay(bool);

fn key(v: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn amounts(cost: &BTreeMap<String, i64>) -> String {
    if cost.is_empty() {
        return "free".into();
    }
    cost.iter()
        .map(|(k, v)| format!("{v} {k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn kind_name(k: &StructureKind) -> &str {
    k.name.as_deref().unwrap_or(&k.id)
}

fn spawn_hud(mut commands: Commands, s: Res<Session>) {
    let root = commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            DespawnOnExit(Screen::Playing),
        ))
        .id();

    // Top bar.
    commands.entity(root).with_children(|r| {
        r.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                left: px(0),
                right: px(0),
                height: px(40),
                padding: UiRect::horizontal(px(12)),
                column_gap: px(18),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_children(|bar| {
            bar.spawn(text(s.map.name.clone(), 18.0, style::ACCENT));
            bar.spawn((text("", 16.0, TEXT), ResourcesText));
            bar.spawn((text("", 16.0, TEXT), WaveText));
            bar.spawn((
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
                Pickable::IGNORE,
            ));
            bar.spawn((text("", 15.0, TEXT_DIM), ClockText));
            bar.spawn((button("Pause", 14.0), Action::Pause));
            for speed in [1.0, 2.0, 4.0] {
                bar.spawn((button(format!("{speed}x"), 14.0), Action::Speed(speed)));
            }
            bar.spawn((button("Menu", 14.0), Action::Menu));
        });

        // Message log.
        r.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(48),
                left: px(12),
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                ..default()
            },
            Pickable::IGNORE,
            LogBox::default(),
        ));

        // Right: selection / tool info.
        r.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(48),
                right: px(8),
                width: px(270),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(10)),
                border_radius: BorderRadius::all(px(6)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(PANEL),
            InfoPanel::default(),
        ));

        // Bottom: palette and actions.
        r.spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: px(0),
                left: px(0),
                right: px(0),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(8)),
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_children(|b| {
            b.spawn((
                Node {
                    column_gap: px(6),
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: px(6),
                    ..default()
                },
                PaletteBar::default(),
            ));
            b.spawn((
                Node {
                    column_gap: px(6),
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: px(6),
                    ..default()
                },
                ActionsBar::default(),
            ));
        });

        // Game over.
        r.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            GlobalZIndex(10),
            EndOverlay::default(),
        ));
    });
}

fn update_palette(s: Res<Session>, mut palette: ResMut<Palette>) {
    let content = &s.game.sim.content;
    let allowed = s.game.sim.allowed(s.me);
    let mut kinds: Vec<usize> = (0..content.structures.len())
        .filter(|&k| allowed.is_none_or(|a| a.contains(&k)))
        // Kinds reachable by upgrading are not built from scratch.
        .filter(|&k| {
            !content
                .structures
                .iter()
                .any(|o| o.upgrades_to.contains(&content.structures[k].id))
        })
        .collect();
    kinds.sort_by_key(|&k| {
        let st = &content.structures[k];
        (
            !st.can_attack(),
            st.aura.is_some(),
            st.cost.values().sum::<i64>(),
            k,
        )
    });
    if palette.0 != kinds {
        palette.0 = kinds;
    }
}

fn top_bar(
    s: Res<Session>,
    mut res: Query<&mut Text, (With<ResourcesText>, Without<WaveText>, Without<ClockText>)>,
    mut wave: Query<&mut Text, (With<WaveText>, Without<ResourcesText>, Without<ClockText>)>,
    mut clock: Query<&mut Text, (With<ClockText>, Without<ResourcesText>, Without<WaveText>)>,
) {
    let sim = &s.game.sim;
    let mut parts: Vec<String> = sim
        .resources(Scope::Player(s.me))
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    let team: Vec<String> = sim
        .resources(Scope::Team(sim.team_of(s.me)))
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    if !team.is_empty() {
        parts.push(format!("team: {}", team.join("  ")));
    }
    if let Ok(mut t) = res.single_mut() {
        let v = parts.join("   ");
        if t.0 != v {
            t.0 = v;
        }
    }
    if let Ok(mut t) = wave.single_mut() {
        let v = match s.wave {
            Some((n, total)) if total > 0 => format!("Wave {n}/{total}"),
            Some((n, _)) => format!("Wave {n}"),
            None => String::new(),
        };
        if t.0 != v {
            t.0 = v;
        }
    }
    if let Ok(mut t) = clock.single_mut() {
        let secs = sim.tick / TICK_RATE as u64;
        let state = if s.paused {
            "  paused".to_string()
        } else if s.speed != 1.0 {
            format!("  {}x", s.speed)
        } else {
            String::new()
        };
        let v = format!("{:02}:{:02}{state}", secs / 60, secs % 60);
        if t.0 != v {
            t.0 = v;
        }
    }
}

fn log_box(mut commands: Commands, s: Res<Session>, mut q: Query<(Entity, &mut LogBox)>) {
    let Ok((e, mut lb)) = q.single_mut() else {
        return;
    };
    // Lines fade out after a while; rebuild once a second at most.
    let recent: Vec<&str> = s
        .log
        .iter()
        .filter(|l| s.clock - l.at < 14.0)
        .map(|l| l.text.as_str())
        .collect();
    let k = key(&recent);
    if lb.0 == k {
        return;
    }
    lb.0 = k;
    commands.entity(e).despawn_children().with_children(|c| {
        for line in recent {
            c.spawn((
                Node {
                    padding: UiRect::axes(px(6), px(2)),
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                Pickable::IGNORE,
                children![text(line, 15.0, TEXT)],
            ));
        }
    });
}

fn palette_bar(
    mut commands: Commands,
    s: Res<Session>,
    palette: Res<Palette>,
    mut q: Query<(Entity, &mut PaletteBar)>,
) {
    let Ok((e, mut bar)) = q.single_mut() else {
        return;
    };
    let k = key(&palette.0);
    if bar.0 == k {
        return;
    }
    bar.0 = k;
    let content = s.game.sim.content.clone();
    commands.entity(e).despawn_children().with_children(|c| {
        for (i, &kind) in palette.0.iter().enumerate() {
            let st = &content.structures[kind];
            let hotkey = if i < 10 {
                format!("{} ", (i + 1) % 10)
            } else {
                String::new()
            };
            c.spawn((style::button_base(), Action::Tool(kind)))
                .with_children(|b| {
                    b.spawn(text(format!("{hotkey}{}", kind_name(st)), 14.0, TEXT));
                    b.spawn(text(amounts(&st.cost), 12.0, TEXT_DIM));
                });
        }
    });
}

fn actions_bar(mut commands: Commands, s: Res<Session>, mut q: Query<(Entity, &mut ActionsBar)>) {
    let Ok((e, mut bar)) = q.single_mut() else {
        return;
    };
    let params = &s.params;
    let mut actions: Vec<(String, Action)> = Vec::new();
    let waves_left = s.wave.is_none_or(|(n, total)| n < total);
    if s.has_module("waves") && waves_left {
        actions.push((
            "Next wave now".into(),
            Action::Request("next_wave".into(), Value::Nil),
        ));
    }
    if s.has_module("builders") {
        let cost = params.path("builders.cost").as_amounts();
        actions.push((
            format!("Buy builder ({})", amounts(&cost)),
            Action::Request("buy_builder".into(), Value::Nil),
        ));
    }
    if s.has_module("versus")
        && let Some(units) = params.path("versus.units").as_map()
    {
        for (unit, cost) in units {
            let mut data = Value::map();
            data.insert("unit", Value::from(unit.as_str()));
            actions.push((
                format!("Send {unit} ({})", amounts(&cost.as_amounts())),
                Action::Request("send_monster".into(), data),
            ));
        }
    }
    let k = key(actions.iter().map(|(l, _)| l).collect::<Vec<_>>());
    if bar.0 == k {
        return;
    }
    bar.0 = k;
    commands.entity(e).despawn_children().with_children(|c| {
        for (label, action) in actions {
            c.spawn((button(label, 13.0), action));
        }
    });
}

fn effect_text(e: &td_core::content::EffectDef) -> String {
    match e.kind {
        EffectKind::Slow => format!("slows {}% for {}s", e.percent, e.duration),
        EffectKind::Dot => format!("{} damage/s for {}s", e.dps, e.duration),
        EffectKind::Stun => format!("stuns for {}s", e.duration),
        EffectKind::Armor => format!("{:+} armor for {}s", e.amount, e.duration),
        EffectKind::Bounce => format!("bounces to {} more ({}% each)", e.count, e.falloff),
        EffectKind::Damage => format!("+{}% damage", e.percent),
        EffectKind::Haste => format!("+{}% attack speed", e.percent),
    }
}

fn structure_lines(k: &StructureKind) -> Vec<String> {
    let mut lines = Vec::new();
    if k.can_attack() {
        lines.push(format!(
            "{} {} damage every {}s",
            k.damage, k.damage_type, k.cooldown
        ));
        let targets = k.targets.names().join(" + ");
        lines.push(format!("range {}, hits {targets}", k.range));
        if let Some(r) = k.splash {
            lines.push(format!("splash {r} tiles"));
        }
    }
    for e in &k.on_hit {
        lines.push(format!("on hit: {}", effect_text(e)));
    }
    if let Some(a) = &k.aura {
        let who = match a.filter {
            td_core::content::AuraFilter::Allies => "towers",
            td_core::content::AuraFilter::Enemies => "enemies",
        };
        for e in &a.effects {
            lines.push(format!("aura {}: {who} {}", a.radius, effect_text(e)));
        }
    }
    if let Some(hp) = k.hp {
        lines.push(format!("{hp} hp, {} armor ({})", k.armor, k.armor_type));
    }
    if k.size > 1 {
        lines.push(format!("{0}x{0} tiles", k.size));
    }
    if k.build_time > 0.0 {
        lines.push(format!("builds in {}s", k.build_time));
    }
    let blocks = k.blocks.names();
    if !blocks.is_empty() {
        lines.push(format!("blocks {}", blocks.join(" + ")));
    }
    lines
}

fn info_panel(
    mut commands: Commands,
    s: Res<Session>,
    ui: Res<UiState>,
    mut q: Query<(Entity, &mut InfoPanel, &mut Node)>,
) {
    let Ok((e, mut panel, mut node)) = q.single_mut() else {
        return;
    };
    let sim = &s.game.sim;
    let content = sim.content.clone();
    let mut title = String::new();
    let mut lines: Vec<String> = Vec::new();
    let mut buttons: Vec<(String, Action)> = Vec::new();
    match (ui.selection, ui.tool) {
        (Selection::Structure(id), _) if sim.structures.contains_key(&id) => {
            let st = &sim.structures[&id];
            let k = &content.structures[st.kind];
            title = kind_name(k).to_string();
            lines.push(format!(
                "owner: {}",
                sim.players
                    .get(st.owner as usize)
                    .map(|p| p.name.as_str())
                    .unwrap_or("?")
            ));
            if st.constructing > 0 {
                let pct = 100 - st.constructing * 100 / st.construct_total.max(1);
                lines.push(format!("under construction {pct}%"));
            }
            if let (Some(hp), Some(max)) = (st.hp, st.max_hp) {
                lines.push(format!("hp {:.0}/{max}", hp.max(0.0)));
            }
            lines.extend(structure_lines(k));
            if st.owner == s.me {
                if k.can_attack() {
                    buttons.push((
                        format!("Targeting: {} (Tab)", st.targeting.name()),
                        Action::Targeting(id),
                    ));
                }
                for up in &k.upgrades_to {
                    if let Some(u) = content.structure(up) {
                        let uk = &content.structures[u];
                        buttons.push((
                            format!("Upgrade: {} ({})", kind_name(uk), amounts(&uk.cost)),
                            Action::Upgrade(id, up.clone()),
                        ));
                    }
                }
                buttons.push((
                    format!("Sell (+{})", amounts(&sim.sell_refund(st))),
                    Action::Sell(id),
                ));
            }
        }
        (Selection::Unit(id), _) if sim.units.contains_key(&id) => {
            let u = &sim.units[&id];
            let k = &content.units[u.kind];
            title = k.name.clone().unwrap_or_else(|| k.id.clone());
            let owner = sim
                .players
                .get(u.owner as usize)
                .map(|p| p.name.as_str())
                .unwrap_or("creeps");
            lines.push(format!("owner: {owner}"));
            lines.push(format!(
                "hp {:.0}/{:.0}, armor {} ({})",
                u.hp.max(0.0),
                u.max_hp,
                k.armor,
                k.armor_type
            ));
            lines.push(format!(
                "speed {}, moves on {}",
                k.speed,
                k.moves_on.names().join(" + ")
            ));
            for eff in &u.effects {
                lines.push(format!(
                    "{}  ({:.1}s)",
                    effect_text(&eff.def),
                    eff.remaining as f32 / TICK_RATE as f32
                ));
            }
            if u.owner == s.me && k.moves_on.contains(td_core::TileFlags::UNIT) {
                lines.push("Right-click to walk. Pick a building below to build here.".into());
            }
        }
        (_, Tool::Build(kind)) => {
            let k = &content.structures[kind];
            title = kind_name(k).to_string();
            lines.push(format!("cost {}", amounts(&k.cost)));
            lines.extend(structure_lines(k));
            lines.push("Click to place, Shift-click to place more, right-click to cancel.".into());
        }
        _ => {}
    }
    let visible = !title.is_empty();
    let k = key((
        &title,
        &lines,
        buttons.iter().map(|(l, _)| l).collect::<Vec<_>>(),
    ));
    if panel.0 == k {
        return;
    }
    panel.0 = k;
    node.display = if visible {
        Display::Flex
    } else {
        Display::None
    };
    commands.entity(e).despawn_children().with_children(|c| {
        c.spawn(text(title, 18.0, style::ACCENT));
        for l in lines {
            c.spawn(text(l, 14.0, TEXT));
        }
        for (label, action) in buttons {
            c.spawn((button(label, 13.0), action));
        }
    });
}

fn end_overlay(
    mut commands: Commands,
    s: Res<Session>,
    mut q: Query<(Entity, &mut EndOverlay, &mut Node)>,
) {
    let Ok((e, mut shown, mut node)) = q.single_mut() else {
        return;
    };
    let Some(result) = s.game.sim.result() else {
        return;
    };
    if shown.0 {
        return;
    }
    shown.0 = true;
    node.display = Display::Flex;
    let won = result.winners.contains(&s.me);
    let (headline, color) = if won {
        ("Victory", style::GOOD)
    } else {
        ("Defeat", style::BAD)
    };
    let detail = result.text.clone();
    commands.entity(e).with_children(|c| {
        c.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(14),
                padding: UiRect::all(px(28)),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_children(|p| {
            p.spawn(text(headline, 48.0, color));
            if !detail.is_empty() && detail != headline {
                p.spawn(text(detail, 18.0, TEXT));
            }
            p.spawn(Node {
                column_gap: px(10),
                ..default()
            })
            .with_children(|row| {
                row.spawn((button("Play again", 16.0), Action::Restart));
                row.spawn((button("Back to menu", 16.0), Action::Menu));
            });
        });
    });
}

fn mark_buttons(
    mut commands: Commands,
    s: Res<Session>,
    ui: Res<UiState>,
    q: Query<(Entity, &Action, Has<Active>, Has<Disabled>)>,
) {
    let me = Scope::Player(s.me);
    let content = &s.game.sim.content;
    for (e, action, active, disabled) in &q {
        let (want_active, want_disabled) = match action {
            Action::Tool(k) => (
                ui.tool == Tool::Build(*k),
                !s.game.sim.can_afford(me, &content.structures[*k].cost),
            ),
            Action::Pause => (s.paused, false),
            Action::Speed(x) => (!s.paused && s.speed == *x, false),
            Action::Upgrade(_, to) => {
                let cost = content.structure(to).map(|k| &content.structures[k].cost);
                (false, cost.is_some_and(|c| !s.game.sim.can_afford(me, c)))
            }
            _ => (false, false),
        };
        if want_active != active {
            if want_active {
                commands.entity(e).insert(Active);
            } else {
                commands.entity(e).remove::<Active>();
            }
        }
        if want_disabled != disabled {
            if want_disabled {
                commands.entity(e).insert(Disabled);
            } else {
                commands.entity(e).remove::<Disabled>();
            }
        }
    }
}

fn on_action(
    ev: On<PointerClick>,
    actions: Query<&Action>,
    session: Option<ResMut<Session>>,
    mut ui: ResMut<UiState>,
    mut next: ResMut<NextState<Screen>>,
    mut commands: Commands,
) {
    if ev.button != PointerButton::Primary {
        return;
    }
    let Ok(action) = actions.get(ev.entity) else {
        return;
    };
    let Some(mut s) = session else { return };
    match action.clone() {
        Action::Tool(k) => {
            ui.tool = if ui.tool == Tool::Build(k) {
                Tool::None
            } else {
                Tool::Build(k)
            };
            if let Selection::Structure(_) = ui.selection {
                ui.selection = Selection::None;
            }
        }
        Action::Pause => s.paused = !s.paused,
        Action::Speed(x) => {
            s.speed = x;
            s.paused = false;
        }
        Action::Menu => next.set(Screen::Menu),
        Action::Restart => {
            // Through the menu state, so the world and HUD are rebuilt.
            commands.insert_resource(PendingStart {
                map: s.map.clone(),
                players: Some(s.game.sim.players.len() as u8),
                seed: s.seed,
            });
            next.set(Screen::Menu);
        }
        Action::Request(request, data) => s.queue(Command::Request { request, data }),
        Action::Upgrade(id, to) => s.queue(Command::Upgrade { structure: id, to }),
        Action::Sell(id) => {
            s.queue(Command::Sell { structure: id });
            ui.selection = Selection::None;
        }
        Action::Targeting(id) => {
            if let Some(st) = s.game.sim.structures.get(&id) {
                let mode = next_targeting(st.targeting);
                s.queue(Command::SetTargeting {
                    structure: id,
                    mode,
                });
            }
        }
    }
}
