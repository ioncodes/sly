mod code_editor;
mod combobox;
mod copyable;
mod header_options;
mod line_geometry;
mod menu;
mod panel;
mod primitives;
mod rows;
mod scroll_shadow;
mod segmented;
mod status_bar;
mod text_input;
mod text_offsets;
mod window;

pub use code_editor::{CodeEditor, CodeEditorEvent, Completion, register_code_editor_keys};
pub use combobox::{ComboOption, combobox_button, combobox_menu};
pub use copyable::Copyable;
pub use menu::{MenuItem, context_menu, popover};
pub use panel::{HEADER_HEIGHT, Panel, PanelChrome};
pub use primitives::{
    CheckboxSize, badge, bool_chip, checkbox, empty_state, empty_state_centered, field_button,
    field_text_button, header_rule, hex, label_text, meter_bar, meter_bar_scaled, mono_text,
    section_header, separator, status_flag,
};

pub use rows::{
    DETAIL_INDENT, FOOTER_ROW_H, INDENT_STEP, RowCallback, chevron_slot, detail_row,
    footer_placeholder, footer_row, footer_row_el, row_toggle,
};
pub use scroll_shadow::{uniform_vertical_scroll_area, vertical_scroll_area};
pub use segmented::Segmented;
pub use status_bar::{StatusView, console_pill, status_bar};
pub use text_input::{TextInput, TextInputEvent, register_text_input_keys};
pub use window::{TITLE_BAR_HEIGHT, icon_button, title_bar, traffic_light_position, window_button};

use gpui::{App, ClickEvent, Window};

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

pub const CONTROL_HEIGHT: f32 = 24.;
