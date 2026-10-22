use std::sync::Arc;

use gpui::{App, Global, Hsla, rgb, rgba};

pub const UI_FONT: &str = "IBM Plex Sans";
pub const MONO_FONT: &str = "IBM Plex Mono";

pub const ACCENT_PRESETS: [u32; 8] = [
    0xA55CFF, // purple (default)
    0xFF8FC7, // pink
    0x7DB9FF, // blue
    0x5FD0C5, // teal
    0x9EDE8F, // green
    0xF2D06B, // yellow
    0xFFB27A, // orange
    0xFF8B8B, // red
];

// Special people :)
pub const GUEST_COLORS: &[(&str, u32)] = &[
    ("Nadine", 0x78866B),
    ("Blody", 0xD7B4F3),
    ("Chicho", 0x93C572),
];

pub fn accent_at(index: usize) -> u32 {
    let presets = ACCENT_PRESETS.len();
    if index < presets {
        ACCENT_PRESETS[index]
    } else {
        GUEST_COLORS[index - presets].1
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

impl ThemeMode {
    pub fn toggled(self) -> Self {
        match self {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        }
    }
}

#[derive(Clone)]
pub struct Theme {
    pub mode: ThemeMode,

    pub ui_font: &'static str,
    pub mono_font: &'static str,

    pub bg_root: Hsla,
    pub bg_panel: Hsla,
    pub bg_panel_alt: Hsla,
    pub bg_header: Hsla,
    pub border_subtle: Hsla,
    pub border_strong: Hsla,

    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_muted: Hsla,
    pub text_dim: Hsla,

    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_faint: Hsla,
    pub accent_hover: Hsla,
    pub on_accent: Hsla,
    pub selection: Hsla,
    pub selection_border: Hsla,
    pub logo: Hsla,

    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub danger_strong: Hsla,
    pub info: Hsla,

    pub read_fill: Hsla,
    pub read_border: Hsla,
    pub write_fill: Hsla,
    pub write_border: Hsla,
    pub exec_fill: Hsla,
    pub exec_border: Hsla,

    pub code_opcode: Hsla,
    pub code_address: Hsla,
    pub code_comment: Hsla,
    pub code_number: Hsla,

    pub code_flow: Hsla,
    pub code_memory: Hsla,
    pub code_alu: Hsla,
    pub code_transfer: Hsla,
    pub code_system: Hsla,
    pub code_register: Hsla,
    pub code_immediate: Hsla,
    pub code_sigil: Hsla,
    pub code_symbol: Hsla,
    pub code_punctuation: Hsla,
}

impl Theme {
    pub fn new(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    pub fn global(cx: &App) -> Arc<Theme> {
        cx.global::<GlobalTheme>().0.clone()
    }

    pub fn with_accent(mut self, accent: Hsla) -> Self {
        self.accent = accent;
        self.accent_soft = with_alpha(accent, 0.23);
        self.accent_faint = with_alpha(accent, 0.07);
        self.accent_hover = lighten(accent, 0.12);
        self.on_accent = if accent.lightness > 0.62 {
            hex(0x101014)
        } else {
            hex(0xffffff)
        };
        self.selection = with_alpha(accent, 0.30);
        self.selection_border = with_alpha(accent, 0.65);
        self.logo = accent;
        self
    }

    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,

            ui_font: UI_FONT,
            mono_font: MONO_FONT,

            bg_root: hex(0x000000),
            bg_panel: hex(0x0a0a0c),
            bg_panel_alt: hex(0x111114),
            bg_header: hex(0x0e0e11),
            border_subtle: hex(0x232328),
            border_strong: hex(0x3a3a42),

            text_primary: hex(0xf0f0f3),
            text_secondary: hex(0xababb2),
            text_muted: hex(0x6c6c74),
            text_dim: hex(0x4a4a52),

            accent: hex(0xa55cff),
            accent_soft: hexa(0xa55cff3a),
            accent_faint: hexa(0xa55cff12),
            accent_hover: hex(0xb985ff),
            on_accent: hex(0xffffff),
            selection: hex(0x4a2f7a),
            selection_border: hex(0x7d5bb8),
            logo: hex(0xa55cff),

            success: hex(0x6cc98a),
            warning: hex(0xd9b46a),
            danger: hex(0xee8a86),
            danger_strong: hex(0xe5484d),
            info: hex(0x7bb8f4),

            read_fill: hex(0x1d3a5c),
            read_border: hex(0x3f6fa8),
            write_fill: hex(0x1e3d2b),
            write_border: hex(0x4f9e6a),
            exec_fill: hex(0x5c2329),
            exec_border: hex(0xa84f54),

            code_opcode: hex(0x86c98f),
            code_address: hex(0x86868f),
            code_comment: hex(0x64645f),
            code_number: hex(0xd7b87e),

            code_flow: hex(0xe6a0c0),
            code_memory: hex(0x8fbde6),
            code_alu: hex(0xe6b58f),
            code_transfer: hex(0xc6a8ec),
            code_system: hex(0x8fe0cf),
            code_register: hex(0x7fd4d4),
            code_immediate: hex(0xa79bcf),
            code_sigil: hex(0x9c8a5e),
            code_symbol: hex(0xf0a995),
            code_punctuation: hex(0x6c6c74),
        }
    }

    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,

            ui_font: UI_FONT,
            mono_font: MONO_FONT,

            bg_root: hex(0xeaeaec),
            bg_panel: hex(0xfbfbfc),
            bg_panel_alt: hex(0xf1f1f3),
            bg_header: hex(0xf3f3f5),
            border_subtle: hex(0xdadade),
            border_strong: hex(0xbcbcc2),

            text_primary: hex(0x18181c),
            text_secondary: hex(0x4c4c54),
            text_muted: hex(0x8a8a92),
            text_dim: hex(0xa6a6ae),

            accent: hex(0x7c3aed),
            accent_soft: hexa(0x7c3aed26),
            accent_faint: hexa(0x7c3aed12),
            accent_hover: hex(0x6a28d9),
            on_accent: hex(0xffffff),
            selection: hex(0xcdb4f5),
            selection_border: hex(0x9d7fd0),
            logo: hex(0x7c3aed),

            success: hex(0x1f9d57),
            warning: hex(0xb5800f),
            danger: hex(0xd23b39),
            danger_strong: hex(0xdc2626),
            info: hex(0x356fa8),

            read_fill: hex(0xc3dbf5),
            read_border: hex(0x5a8fc0),
            write_fill: hex(0xc8ecd0),
            write_border: hex(0x4f9e63),
            exec_fill: hex(0xf3c7c4),
            exec_border: hex(0xc2706c),

            code_opcode: hex(0x1f8a3b),
            code_address: hex(0x6c6c74),
            code_comment: hex(0x82826a),
            code_number: hex(0x9a5a12),

            code_flow: hex(0xb83a78),
            code_memory: hex(0x2f6fb0),
            code_alu: hex(0xb5651f),
            code_transfer: hex(0x7a3ac4),
            code_system: hex(0x12806a),
            code_register: hex(0x1f8a8a),
            code_immediate: hex(0x5f4fa0),
            code_sigil: hex(0x8a7340),
            code_symbol: hex(0xc26a4f),
            code_punctuation: hex(0x8a8a92),
        }
    }
}

pub fn hex(color: u32) -> Hsla {
    gpui::rgb_to_hsla(rgb(color))
}

pub fn hexa(color: u32) -> Hsla {
    gpui::rgb_to_hsla(rgba(color))
}

pub fn with_alpha(mut color: Hsla, alpha: f32) -> Hsla {
    color.alpha = alpha;
    color
}

pub fn lighten(mut color: Hsla, amount: f32) -> Hsla {
    color.lightness = (color.lightness + amount).min(1.0);
    color
}

pub struct GlobalTheme(pub Arc<Theme>);

impl Global for GlobalTheme {}

pub fn set_theme(cx: &mut App, theme: Theme) {
    cx.set_global(GlobalTheme(Arc::new(theme)));
}
