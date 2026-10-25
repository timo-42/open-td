//! Colours, text helpers and button styling shared by the menu and HUD.
//! Buttons act on `PointerClick` (see `hud::on_action`), not the widget's
//! `Activate`, which misses a press and release within one frame.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::Button;

pub const BACKDROP: Color = Color::srgb(0.07, 0.07, 0.09);
pub const PANEL: Color = Color::srgba(0.08, 0.09, 0.11, 0.92);
pub const BUTTON: Color = Color::srgb(0.19, 0.21, 0.26);
pub const BUTTON_HOVER: Color = Color::srgb(0.27, 0.30, 0.37);
pub const BUTTON_PRESS: Color = Color::srgb(0.36, 0.40, 0.50);
pub const BUTTON_ACTIVE: Color = Color::srgb(0.25, 0.42, 0.30);
pub const BUTTON_DISABLED: Color = Color::srgb(0.14, 0.14, 0.16);
pub const TEXT: Color = Color::srgb(0.92, 0.92, 0.88);
pub const TEXT_DIM: Color = Color::srgb(0.62, 0.63, 0.66);
pub const GOOD: Color = Color::srgb(0.45, 0.85, 0.45);
pub const BAD: Color = Color::srgb(0.95, 0.40, 0.35);
pub const ACCENT: Color = Color::srgb(0.95, 0.80, 0.40);

pub fn plugin(app: &mut App) {
    app.add_systems(PostUpdate, style_buttons);
}

/// Parses `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    let n = |i: usize, len: usize| u8::from_str_radix(&s[i..i + len], 16).ok();
    let (r, g, b, a) = match s.len() {
        3 => (n(0, 1)? * 17, n(1, 1)? * 17, n(2, 1)? * 17, 255),
        6 => (n(0, 2)?, n(2, 2)?, n(4, 2)?, 255),
        8 => (n(0, 2)?, n(2, 2)?, n(4, 2)?, n(6, 2)?),
        _ => return None,
    };
    Some(Color::srgba_u8(r, g, b, a))
}

pub fn color_or(s: &Option<String>, fallback: Color) -> Color {
    s.as_deref().and_then(hex).unwrap_or(fallback)
}

/// A stable colour for anything without one, from its id.
pub fn id_color(id: &str) -> Color {
    let h = id
        .bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    Color::hsl((h % 360) as f32, 0.55, 0.55)
}

pub fn text(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s),
        TextFont::from_font_size(size),
        TextColor(color),
        Pickable::IGNORE,
    )
}

/// Marks a button as the current choice (selected tool, chosen map …).
#[derive(Component)]
pub struct Active;

/// A button that can't be used right now.
#[derive(Component)]
pub struct Disabled;

pub fn button_node() -> Node {
    Node {
        padding: UiRect::axes(px(10), px(6)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        flex_direction: FlexDirection::Column,
        border_radius: BorderRadius::all(px(4)),
        ..default()
    }
}

/// A button without content.
pub fn button_base() -> impl Bundle {
    (
        Button,
        Hovered::default(),
        button_node(),
        BackgroundColor(BUTTON),
    )
}

/// A button with a label; add a component describing what it does.
pub fn button(label: impl Into<String>, size: f32) -> impl Bundle {
    (button_base(), children![text(label, size, TEXT)])
}

fn style_buttons(
    mut q: Query<
        (
            &mut BackgroundColor,
            &Hovered,
            Has<Pressed>,
            Has<Active>,
            Has<Disabled>,
        ),
        With<Button>,
    >,
) {
    for (mut bg, hovered, pressed, active, disabled) in &mut q {
        let c = if disabled {
            BUTTON_DISABLED
        } else if pressed {
            BUTTON_PRESS
        } else if hovered.get() {
            BUTTON_HOVER
        } else if active {
            BUTTON_ACTIVE
        } else {
            BUTTON
        };
        if bg.0 != c {
            bg.0 = c;
        }
    }
}
