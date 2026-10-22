use super::*;

impl Debugger {
    pub(super) fn build_primary_panels(
        &mut self,
        visible: &HashSet<String>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> (HashMap<String, AnyElement>, Vec<AnyElement>) {
        let this = cx.entity();
        let framebuffer = visible
            .contains(FramebufferWidget::ID)
            .then(|| {
                FramebufferWidget::new(self.framebuffer.clone())
                    .collapsed(self.is_collapsed(FramebufferWidget::ID))
                    .on_toggle(cx.listener(|this, _, _, cx| this.toggle(FramebufferWidget::ID, cx)))
            })
            .map(|widget| widget.into_any_element());

        let cartridge = visible.contains(CartridgeWidget::ID).then(|| {
            CartridgeWidget::new(self.cartridge.clone())
                .collapsed(self.is_collapsed(CartridgeWidget::ID))
                .on_toggle(cx.listener(|this, _, _, cx| this.toggle(CartridgeWidget::ID, cx)))
        });

        let controls_view = ControlsView {
            running: self.execution.is_running(),
            targets: self.step_targets.clone(),
        };
        let controls = ControlsWidget::new(controls_view)
            .collapsed(self.is_collapsed(ControlsWidget::ID))
            .on_toggle(cx.listener(|this, _, _, cx| this.toggle(ControlsWidget::ID, cx)))
            .on_run(cx.listener(|this, _, _, cx| this.set_running(true, cx)))
            .on_pause(cx.listener(|this, _, _, cx| this.set_running(false, cx)))
            .on_step(cx.listener(|this, _, _, cx| this.step(cx)))
            .on_run_until({
                let this = this.clone();
                std::rc::Rc::new(move |id: SharedString, _: &mut Window, cx: &mut App| {
                    this.update(cx, |debugger, cx| debugger.run_until(&id, cx));
                })
            })
            .on_reset(cx.listener(|this, _, _, cx| this.reset(cx)))
            .on_save_state(cx.listener(|this, _, _, cx| {
                let slot = this.active_slot();
                this.save_state(slot, cx);
            }))
            .on_load_state(cx.listener(|this, _, _, cx| {
                let slot = this.active_slot();
                this.load_state(slot, cx);
            }));

        let savestates = visible.contains(SavestatesWidget::ID).then(|| {
            let this = this.clone();
            SavestatesWidget::new((*self.savestates_view()).clone())
                .scroll(self.scroll_for(SavestatesWidget::ID))
                .collapsed(self.is_collapsed(SavestatesWidget::ID))
                .selected(self.active_slot().id())
                .on_toggle(cx.listener(|this, _, _, cx| this.toggle(SavestatesWidget::ID, cx)))
                .on_action(move |action, id, _, cx| {
                    this.update(cx, |debugger, cx| debugger.slot_action(action, &id, cx));
                })
        });

        let mmio_priority = self
            .registry
            .manifest()
            .workspaces
            .iter()
            .find(|w| w.id.as_ref() == self.workspace.as_ref())
            .and_then(|w| w.mmio_group.clone());
        let mmio = MmioWidget::new(self.mmio_view.clone())
            .scroll(self.mmio_scroll.clone())
            .priority(mmio_priority.map(SharedString::from))
            .expanded(self.expanded_set(MmioWidget::ID))
            .collapsed(self.is_collapsed(MmioWidget::ID))
            .on_toggle(cx.listener(|this, _, _, cx| this.toggle(MmioWidget::ID, cx)))
            .on_toggle_register({
                let this = this.clone();
                move |id, _, cx| this.update(cx, |d, cx| d.toggle_expanded(MmioWidget::ID, id, cx))
            });

        let palette = visible.contains(PaletteWidget::ID).then(|| {
            let resources = self.resources_of(VideoResourceKind::Palette);
            PaletteWidget::new(self.palette_view.clone())
                .resources(resources.clone(), self.palette_ui.index(&resources))
                .on_resource_select({
                    let this = this.clone();
                    move |index, _, cx| {
                        this.update(cx, |d, cx| {
                            d.palette_ui.resource = resources.get(index).map(|r| r.id.clone());
                            d.palette_selected = None;
                            d.refresh_palette();
                            d.refresh_tileset(cx);
                            cx.notify();
                        })
                    }
                })
                .scroll(self.scroll_for(PaletteWidget::ID))
                .selected(self.palette_selected)
                .collapsed(self.is_collapsed(PaletteWidget::ID))
                .on_toggle(cx.listener(|this, _, _, cx| this.toggle(PaletteWidget::ID, cx)))
                .on_select({
                    let this = this.clone();
                    move |palette, color, _, cx| {
                        this.update(cx, |d, cx| d.select_palette_color(palette, color, cx))
                    }
                })
        });

        let tileset = visible.contains(TilesetWidget::ID).then(|| {
            self.wire_grid(
                TilesetWidget::panel(self.tileset.clone()),
                TilesetWidget::ID,
                &self.tileset_ui,
                self.resources_of(VideoResourceKind::Tileset),
                |d| &mut d.tileset_ui,
                Self::refresh_tileset,
                cx,
            )
        });

        let tilemap = visible.contains(TilemapWidget::ID).then(|| {
            self.wire_grid(
                TilemapWidget::panel(self.tilemap.clone()),
                TilemapWidget::ID,
                &self.tilemap_ui,
                self.resources_of(VideoResourceKind::Tilemap),
                |d| &mut d.tilemap_ui,
                Self::refresh_tilemap,
                cx,
            )
        });

        let sprites = visible.contains(SpritesWidget::ID).then(|| {
            self.wire_grid(
                SpritesWidget::panel(self.sprites.clone()),
                SpritesWidget::ID,
                &self.sprites_ui,
                self.resources_of(VideoResourceKind::Sprites),
                |d| &mut d.sprites_ui,
                Self::refresh_sprites,
                cx,
            )
        });

        let bitmap = visible.contains(BitmapWidget::ID).then(|| {
            self.wire_grid(
                BitmapWidget::panel(self.bitmap.clone()),
                BitmapWidget::ID,
                &self.bitmap_ui,
                self.resources_of(VideoResourceKind::Bitmap),
                |d| &mut d.bitmap_ui,
                Self::refresh_bitmap,
                cx,
            )
        });

        let breakpoints =
            visible.contains(BreakpointsWidget::ID).then(|| {
                BreakpointsWidget::new(self.breakpoints.view(), self.breakpoints.draft_kind())
                    .scroll(self.scroll_for(BreakpointsWidget::ID))
                    .collapsed(self.is_collapsed(BreakpointsWidget::ID))
                    .scrollable(true)
                    .address_input(self.bp_input.clone())
                    .on_toggle(cx.listener(|this, _, _, cx| this.toggle(BreakpointsWidget::ID, cx)))
                    .on_add(cx.listener(|this, _, _, cx| this.bp_add(cx)))
                    .on_kind_open(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        this.bp_open_combo(ev.position(), cx)
                    }))
                    .on_remove({
                        let this = this.clone();
                        move |id, _, cx| this.update(cx, |d, cx| d.bp_remove(id, cx))
                    })
                    .on_toggle_entry({
                        let this = this.clone();
                        move |id, _, cx| this.update(cx, |d, cx| d.bp_toggle(id, cx))
                    })
            });

        let mut cpu_panels: Vec<(String, AnyElement)> = Vec::with_capacity(self.panes.len());
        let mut disasm_panels: Vec<(String, AnyElement)> = Vec::with_capacity(self.panes.len());
        let mut call_stack_panels: Vec<(String, AnyElement)> = Vec::with_capacity(self.panes.len());
        let mut disasm_menus = Vec::new();

        for (pane_ix, pane) in self.panes.iter().enumerate() {
            if visible.contains(pane.id.as_str()) {
                let cpu_id = SharedString::from(pane.id.arc());
                let cpu_widget = CpuWidget::new(pane.cpu_view.clone())
                    .id(cpu_id.clone())
                    .title(pane.title.clone())
                    .collapsed(self.is_collapsed(&cpu_id))
                    .on_toggle({
                        let id = cpu_id.clone();
                        cx.listener(move |this, _, _, cx| this.toggle(id.clone(), cx))
                    });
                cpu_panels.push((
                    pane.id.as_str().to_string(),
                    div().flex_none().child(cpu_widget).into_any_element(),
                ));
            }

            let disasm_panel_id = format!("{}.disasm", pane.id.as_str());
            if !visible.contains(disasm_panel_id.as_str()) {
                continue;
            }

            let disasm_id = SharedString::from(disasm_panel_id.clone());
            let disasm_title = SharedString::from(format!("{} Disassembly", pane.title));
            let disassembly =
                DisassemblyWidget::new(pane.disasm.view.clone(), pane.disasm.scroll.clone())
                    .id(disasm_id.clone())
                    .title(disasm_title)
                    .current(pane.disasm.current_ix())
                    .pipeline(pane.disasm.pipeline_ix())
                    .current_note(pane.disasm.current_note())
                    .selected(pane.disasm.selected_ix())
                    .breakpoints(pane.disasm.breakpoint_rows())
                    .layout_cache(pane.disasm.layout_cache.clone())
                    .collapsed(self.is_collapsed(&disasm_id))
                    .on_toggle({
                        let id = disasm_id.clone();
                        cx.listener(move |this, _, _, cx| this.toggle(id.clone(), cx))
                    })
                    .on_select({
                        let this = this.clone();
                        move |ix, _, cx| this.update(cx, |d, cx| d.disasm_select(pane_ix, ix, cx))
                    })
                    .on_context({
                        let this = this.clone();
                        move |ix, pos, _, cx| {
                            this.update(cx, |d, cx| d.disasm_open_menu(pane_ix, ix, pos, cx))
                        }
                    })
                    .decode_modes(pane.disasm.decode_modes(), pane.disasm.decode_mode(), {
                        let this = this.clone();
                        move |mode, _, cx| {
                            this.update(cx, |d, cx| {
                                d.panes[pane_ix]
                                    .disasm
                                    .set_decode_mode(mode, d.target.as_ref());
                                cx.notify();
                            })
                        }
                    })
                    .view_mode(pane.disasm.view_mode())
                    .graph_offset(pane.disasm.graph_offset())
                    .on_set_view({
                        let this = this.clone();
                        move |mode, _, cx| {
                            this.update(cx, |d, cx| d.disasm_set_view(pane_ix, mode, cx))
                        }
                    })
                    .on_pan_begin({
                        let this = this.clone();
                        move |pos, _, cx| this.update(cx, |d, _cx| d.disasm_pan_begin(pane_ix, pos))
                    })
                    .on_pan_drag({
                        let this = this.clone();
                        move |pos, _, cx| {
                            this.update(cx, |d, cx| d.disasm_pan_drag(pane_ix, pos, cx))
                        }
                    })
                    .follow(pane.disasm.follow_pc())
                    .graph_viewport(pane.disasm.graph_viewport())
                    .on_viewport({
                        let this = this.clone();
                        move |viewport, _, cx| {
                            this.update(cx, |d, cx| d.disasm_set_viewport(pane_ix, viewport, cx))
                        }
                    })
                    .hovered_edge(pane.disasm.hovered_edge())
                    .on_edge_hover({
                        let this = this.clone();
                        move |edge, _, cx| {
                            this.update(cx, |d, cx| d.disasm_hover_edge(pane_ix, edge, cx))
                        }
                    })
                    .on_jump({
                        let this = this.clone();
                        move |from, to, _, cx| {
                            this.update(cx, |d, cx| d.disasm_jump(pane_ix, from, to, cx))
                        }
                    });

            let cs_id = SharedString::from(format!("{}.callstack", pane.id.as_str()));
            let cs_title = SharedString::from(format!("{} Call Stack", pane.title));
            let callstack = CallStackWidget::new(pane.call_stack.clone())
                .id(cs_id.clone())
                .title(cs_title)
                .scrollable(true)
                .scroll(self.scroll_for(&cs_id))
                .collapsed(self.is_collapsed(&cs_id))
                .on_toggle({
                    let id = cs_id.clone();
                    cx.listener(move |this, _, _, cx| this.toggle(id.clone(), cx))
                });

            disasm_panels.push((
                disasm_panel_id,
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .child(disassembly)
                    .into_any_element(),
            ));
            call_stack_panels.push((cs_id.to_string(), callstack.into_any_element()));

            if let Some(menu) = pane.disasm.menu.as_ref() {
                let addr = menu.addr;
                let len = menu.len;

                let mut items = vec![
                    MenuItem::new(
                        "Copy Address",
                        super::clipboard_listener(cx, menu.address.clone(), Debugger::close_menu),
                    ),
                    MenuItem::new(
                        "Copy Bytes",
                        super::clipboard_listener(cx, menu.bytes.clone(), Debugger::close_menu),
                    ),
                    MenuItem::new(
                        "Copy Disassembly",
                        super::clipboard_listener(cx, menu.text.clone(), Debugger::close_menu),
                    ),
                    MenuItem::new(
                        "Set PC to here",
                        cx.listener(move |this, _, _, cx| this.set_pc_to(pane_ix, addr, cx)),
                    ),
                    MenuItem::new(
                        "Add execute breakpoint",
                        cx.listener(move |this, _, _, cx| {
                            this.bp_add_at(pane_ix, addr, len, BreakpointKind::Execute, cx)
                        }),
                    ),
                ];

                if pane_ix == 0 {
                    items.push(MenuItem::new(
                        "Add read breakpoint",
                        cx.listener(move |this, _, _, cx| {
                            this.bp_add_at(pane_ix, addr, len, BreakpointKind::Read, cx)
                        }),
                    ));
                    items.push(MenuItem::new(
                        "Add write breakpoint",
                        cx.listener(move |this, _, _, cx| {
                            this.bp_add_at(pane_ix, addr, len, BreakpointKind::Write, cx)
                        }),
                    ));
                }

                disasm_menus.push(context_menu(
                    t,
                    menu.position,
                    items,
                    cx.listener(|this, _event: &MouseDownEvent, _window, cx| this.close_menu(cx)),
                ));
            }
        }

        let mut panels = HashMap::new();
        panels.insert(
            ControlsWidget::ID.into(),
            div().flex_none().child(controls).into_any_element(),
        );
        if let Some(savestates) = savestates {
            panels.insert(
                SavestatesWidget::ID.into(),
                div().flex_none().child(savestates).into_any_element(),
            );
        }
        if let Some(breakpoints) = breakpoints {
            panels.insert(BreakpointsWidget::ID.into(), breakpoints.into_any_element());
        }

        if let Some(palette) = palette {
            panels.insert(PaletteWidget::ID.into(), palette.into_any_element());
        }

        if let Some(tileset) = tileset {
            panels.insert(TilesetWidget::ID.into(), tileset.into_any_element());
        }

        if let Some(tilemap) = tilemap {
            panels.insert(TilemapWidget::ID.into(), tilemap.into_any_element());
        }

        if let Some(cartridge) = cartridge {
            panels.insert(
                CartridgeWidget::ID.into(),
                div().flex_none().child(cartridge).into_any_element(),
            );
        }
        panels.insert(MmioWidget::ID.into(), mmio.into_any_element());
        if let Some(sprites) = sprites {
            panels.insert(SpritesWidget::ID.into(), sprites.into_any_element());
        }

        if let Some(bitmap) = bitmap {
            panels.insert(BitmapWidget::ID.into(), bitmap.into_any_element());
        }

        if let Some(screen) = framebuffer {
            panels.insert(FramebufferWidget::ID.into(), screen);
        }

        panels.extend(cpu_panels);
        panels.extend(disasm_panels);
        panels.extend(call_stack_panels);

        let overlays = disasm_menus
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect();
        (panels, overlays)
    }
}
