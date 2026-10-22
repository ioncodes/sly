use super::*;

mod auxiliary;
mod primary;
mod registry;

fn accent_dot(
    t: &Theme,
    index: usize,
    selected: bool,
    cx: &mut Context<Debugger>,
) -> impl IntoElement {
    div()
        .id(("accent", index))
        .size(px(16.))
        .rounded_full()
        .bg(rgb(accent_at(index)))
        .border_2()
        .border_color(if selected {
            t.text_primary
        } else {
            gpui::transparent_black()
        })
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.set_accent(index, cx)))
}

pub(super) fn themed(mode: ThemeMode, accent: usize) -> Theme {
    let base = Theme::new(mode);
    if accent == 0 {
        base
    } else {
        base.with_accent(cirsium::theme::hex(accent_at(accent)))
    }
}

fn workspace_icon(icon: WorkspaceIcon) -> &'static str {
    match icon {
        WorkspaceIcon::Cpu => icon::CPU,
        WorkspaceIcon::Video => icon::MONITOR,
        WorkspaceIcon::Chip => icon::ZAP,
        WorkspaceIcon::Sound => icon::AUDIO,
        WorkspaceIcon::Script => icon::CODE,
    }
}

fn clipboard_listener(
    cx: &mut Context<Debugger>,
    payload: SharedString,
    close: fn(&mut Debugger, &mut Context<Debugger>),
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, _, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(payload.to_string()));
        close(this, cx);
    })
}

impl Render for Debugger {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx);
        let t = &*theme;

        self.sync_game_focus(window, cx);

        self.logs.follow_tail();

        let mut graph_animating = false;
        for pane in &mut self.panes {
            graph_animating |= pane.disasm.graph_animating();
        }
        if graph_animating {
            window.request_animation_frame();
        }

        #[cfg(target_family = "wasm")]
        if self.execution.is_running() && !self.web_tick_armed {
            self.web_tick_armed = true;

            let entity = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                entity
                    .update(cx, |this, cx| {
                        this.web_tick_armed = false;
                        if this.execution.is_running() {
                            this.run_tick(Duration::ZERO, cx);
                        }
                    })
                    .ok();
            });
        }

        let this = cx.entity();

        let manifest = self.registry.manifest();
        let active_workspace = self.workspace.clone();
        let mut visible: HashSet<String> = manifest
            .in_workspace(&active_workspace)
            .map(|p| p.id.to_string())
            .collect();
        if !self.show_log {
            visible.remove(LogWidget::ID);
        }

        let workspace_switcher = (manifest.workspaces.len() > 1).then(|| {
            let mut switcher = Segmented::new("workspace").selected(active_workspace.clone());

            for ws in &manifest.workspaces {
                switcher = switcher.option(
                    SharedString::from(ws.id.clone()),
                    self::workspace_icon(ws.icon),
                    SharedString::from(ws.title.clone()),
                );
            }

            switcher.on_select({
                let this = this.clone();
                move |id, _, cx| {
                    let id = SharedString::from(id.to_string());
                    this.update(cx, |d, cx| d.set_workspace(id, cx));
                }
            })
        });

        let settings_button = icon_button(
            t,
            "settings",
            t.bg_panel_alt,
            icon::SETTINGS,
            cx.listener(|this, ev: &ClickEvent, _, cx| this.open_settings(ev.position(), cx)),
        );
        let theme_toggle = icon_button(
            t,
            "theme-toggle",
            t.bg_panel_alt,
            icon::THEME,
            cx.listener(|this, _, _, cx| {
                let next = Theme::global(cx).mode.toggled();
                set_theme(cx, self::themed(next, this.accent));
                this.save_preferences(cx);
                cx.notify();
            }),
        );
        let titlebar_leading = div().flex().items_center().h_full().children(
            workspace_switcher.map(|switcher| div().flex().items_center().child(switcher)),
        );
        let titlebar_trailing = div()
            .flex()
            .items_center()
            .gap_2()
            .h_full()
            .child(settings_button)
            .child(theme_toggle);

        let (mut panels, mut overlays) = self.build_primary_panels(&visible, t, cx);
        overlays.extend(self.build_auxiliary_panels(&visible, t, &mut panels, cx));
        self.insert_generic_panels(&active_workspace, &visible, &mut panels, cx);
        self.panel_frames.replace(panels);
        let dock = self
            .registry
            .workspace_dock(&active_workspace, &visible, |id| self.is_collapsed(id));

        div()
            .track_focus(&self.game_focus)
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                window.focus(&this.game_focus, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.game_key(&event.keystroke, true, window, cx);
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, window, cx| {
                this.game_key(&event.keystroke, false, window, cx);
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(t.bg_root)
            .font_family(t.ui_font)
            .text_color(t.text_primary)
            .text_sm()
            .child(title_bar(
                t,
                "Sly Debugger",
                titlebar_leading,
                titlebar_trailing,
            ))
            .child(dock)
            .child(status_bar(t, &self.status_with_perf()))
            .children(overlays)
    }
}
