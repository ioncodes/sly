use super::*;

impl Debugger {
    pub(super) fn set_pc_to(&mut self, pane: usize, addr: u32, cx: &mut Context<Self>) {
        let id = self.panes[pane].id.clone();
        self.panes[pane].disasm.close_menu();
        self.target.set_pc(&id, addr);
        Self::refresh_pane_view(
            self.target.as_ref(),
            &mut self.panes[pane],
            CpuRefresh::Highlight,
        );
        self.set_state("pc set");
        cx.notify();
    }

    pub(super) fn disasm_select(&mut self, pane: usize, ix: usize, cx: &mut Context<Self>) {
        self.panes[pane].disasm.select(ix);
        cx.notify();
    }

    pub(super) fn disasm_open_menu(
        &mut self,
        pane: usize,
        ix: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.panes[pane].disasm.open_menu(ix, position);
        cx.notify();
    }

    pub(super) fn disasm_set_view(&mut self, pane: usize, mode: ViewMode, cx: &mut Context<Self>) {
        self.panes[pane]
            .disasm
            .set_view_mode(mode, self.target.as_ref());
        cx.notify();
    }

    pub(super) fn disasm_pan_begin(&mut self, pane: usize, pos: Point<Pixels>) {
        self.panes[pane].disasm.pan_begin(pos);
    }

    pub(super) fn disasm_pan_drag(
        &mut self,
        pane: usize,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.panes[pane].disasm.pan_drag(pos);
        cx.notify();
    }

    pub(super) fn disasm_set_viewport(
        &mut self,
        pane: usize,
        viewport: Size<f32>,
        cx: &mut Context<Self>,
    ) {
        if self.panes[pane].disasm.set_graph_viewport(viewport) {
            cx.notify();
        }
    }

    pub(super) fn disasm_hover_edge(
        &mut self,
        pane: usize,
        edge: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if self.panes[pane].disasm.set_hovered_edge(edge) {
            cx.notify();
        }
    }

    pub(super) fn disasm_jump(
        &mut self,
        pane: usize,
        from: Point<Pixels>,
        to: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.panes[pane].disasm.jump(from, to);
        cx.notify();
    }

    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for pane in &mut self.panes {
            changed |= pane.disasm.close_menu();
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn memory_set_region(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.memory.set_region(&id);
        self.memory.load_page(0, self.target.as_ref());
        cx.notify();
    }

    pub(super) fn memory_open_combo(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.memory.open_combo(position);
        cx.notify();
    }

    pub(super) fn memory_close_combo(&mut self, cx: &mut Context<Self>) {
        if self.memory.close_combo() {
            cx.notify();
        }
    }

    pub(super) fn memory_byte_down(&mut self, addr: u32, shift: bool, cx: &mut Context<Self>) {
        self.memory.begin(addr, shift);
        cx.notify();
    }

    pub(super) fn memory_byte_drag(&mut self, addr: u32, cx: &mut Context<Self>) {
        if self.memory.drag(addr) {
            cx.notify();
        }
    }

    pub(super) fn memory_open_menu(
        &mut self,
        addr: u32,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.memory.open_menu(addr, position, self.target.as_ref());
        cx.notify();
    }

    pub(super) fn memory_close_menu(&mut self, cx: &mut Context<Self>) {
        if self.memory.close_menu() {
            cx.notify();
        }
    }

    pub(super) fn memory_dump(&mut self, cx: &mut Context<Self>) {
        if let Some((name, bytes)) = self.memory.dump(self.target.as_ref()) {
            crate::save::save_bin(name, bytes, cx);
        }
    }

    pub(super) fn memory_toggle_follow(&mut self, cx: &mut Context<Self>) {
        self.memory.toggle_follow();
        self.follow_memory();
        cx.notify();
    }

    pub(super) fn follow_memory(&mut self) {
        if self.memory.follow_enabled()
            && let Some((region, addr)) = self.target.latest_access()
        {
            self.memory.follow_to(&region, addr);
        }
    }

    pub(super) fn on_jump_event(
        &mut self,
        _input: Entity<TextInput>,
        event: &TextInputEvent,
        cx: &mut Context<Self>,
    ) {
        if let TextInputEvent::Submit = event {
            self.jump_submit(cx);
        }
    }

    pub(super) fn jump_submit(&mut self, cx: &mut Context<Self>) {
        let text = self.jump_input.read(cx).content().trim().to_string();
        if let Ok(addr) = u32::from_str_radix(&text, 16) {
            self.memory.jump_to(addr);
        }
        self.jump_input.update(cx, |input, cx| input.clear(cx));
        cx.notify();
    }

    pub(super) fn on_bp_event(
        &mut self,
        _input: Entity<TextInput>,
        event: &TextInputEvent,
        cx: &mut Context<Self>,
    ) {
        if let TextInputEvent::Submit = event {
            self.bp_add(cx);
        }
    }

    fn supports_watchpoint(&mut self, kind: BreakpointKind, region: &str) -> bool {
        if kind == BreakpointKind::Execute
            || self
                .target
                .memory_regions()
                .iter()
                .any(|r| r.id.as_ref() == region && !r.watch_sources.is_empty())
        {
            return true;
        }

        self.set_state("data watchpoints unsupported for this region");
        false
    }

    pub(super) fn bp_add(&mut self, cx: &mut Context<Self>) {
        let text = self.bp_input.read(cx).content().trim().to_string();
        if let Ok(addr) = u32::from_str_radix(&text, 16) {
            let (cpu, region) = self.pane_space(self.workspace_pane());
            if !self.supports_watchpoint(self.breakpoints.draft_kind(), &region) {
                cx.notify();
                return;
            }

            self.breakpoints
                .add(addr, cpu, region, self.target.as_ref());
            self.bp_input.update(cx, |input, cx| input.clear(cx));
        }
        self.sync_bp_markers();
        cx.notify();
    }

    pub(super) fn bp_add_at(
        &mut self,
        pane: usize,
        addr: u32,
        len: u32,
        kind: BreakpointKind,
        cx: &mut Context<Self>,
    ) {
        let (cpu, region) = self.pane_space(pane);
        if !self.supports_watchpoint(kind, &region) {
            cx.notify();
            return;
        }

        self.panes[pane].disasm.close_menu();
        let end = addr.saturating_add(len.saturating_sub(1));
        self.breakpoints
            .add_kind(addr, end, kind, cpu, region, self.target.as_ref());
        self.sync_bp_markers();
        cx.notify();
    }

    pub(super) fn bp_add_mem(
        &mut self,
        start: u32,
        end: u32,
        kind: BreakpointKind,
        label: SharedString,
        cx: &mut Context<Self>,
    ) {
        self.memory.close_menu();
        let (cpu, _) = self.pane_space(self.workspace_pane());
        let region: Arc<str> = self.memory.region_id().into();
        if !self.supports_watchpoint(kind, &region) {
            cx.notify();
            return;
        }

        let note = self.target.annotate(start).into();
        self.breakpoints
            .add_labeled(start, end, kind, cpu, region, label, note);
        self.sync_bp_markers();
        cx.notify();
    }

    /// A processor's id and address space, used to tag breakpoints raised from
    /// that pane.
    pub(super) fn pane_space(&self, pane: usize) -> (CpuId, Arc<str>) {
        (self.panes[pane].id.clone(), self.panes[pane].space.clone())
    }

    fn workspace_pane(&self) -> usize {
        let cpu = self
            .registry
            .manifest()
            .panels
            .iter()
            .find_map(|panel| match &panel.kind {
                PanelKind::Registers { cpu } if panel.visible_in(&self.workspace) => {
                    Some(cpu.as_ref())
                }
                _ => None,
            });

        cpu.and_then(|cpu| self.panes.iter().position(|pane| pane.id.as_str() == cpu))
            .unwrap_or(0)
    }

    pub(super) fn bp_remove(&mut self, id: usize, cx: &mut Context<Self>) {
        self.breakpoints.remove(id);
        self.sync_bp_markers();
        cx.notify();
    }

    pub(super) fn bp_toggle(&mut self, id: usize, cx: &mut Context<Self>) {
        self.breakpoints.toggle(id);
        self.sync_bp_markers();
        cx.notify();
    }

    pub(super) fn sync_bp_markers(&mut self) {
        let marks = self.breakpoints.marks();
        for pane in &mut self.panes {
            pane.disasm.set_breakpoints(marks);
        }

        self.memory.set_breakpoints(marks.to_vec());
    }

    pub(super) fn bp_set_kind(&mut self, id: SharedString, cx: &mut Context<Self>) {
        if let Some(kind) = BreakpointKind::from_id(&id) {
            self.breakpoints.set_draft_kind(kind);
        }
        cx.notify();
    }

    pub(super) fn bp_open_combo(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.breakpoints.open_combo(position);
        cx.notify();
    }

    pub(super) fn bp_close_combo(&mut self, cx: &mut Context<Self>) {
        if self.breakpoints.close_combo() {
            cx.notify();
        }
    }

    pub(super) fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        for pane in &mut self.panes {
            pane.disasm.toggle_follow(self.target.as_ref());
        }
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn toggle_bytes(&mut self, cx: &mut Context<Self>) {
        for pane in &mut self.panes {
            pane.disasm.toggle_bytes();
        }
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        match settings::api_key() {
            Ok(key) => self
                .artwork_key_input
                .update(cx, |input, cx| input.set_content(&key, cx)),
            Err(error) => tracing::error!("{error}"),
        }

        self.settings = Some(position);
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn clear_logs(&mut self, cx: &mut Context<Self>) {
        self.logs.clear();
        cx.notify();
    }

    pub(super) fn log_toggle_dedup(&mut self, cx: &mut Context<Self>) {
        self.logs.toggle_dedup();
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn toggle_show_log(&mut self, cx: &mut Context<Self>) {
        self.show_log = !self.show_log;
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn toggle_live_refresh(&mut self, cx: &mut Context<Self>) {
        self.live_refresh = !self.live_refresh;
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn toggle_grid_inline_details(&mut self, cx: &mut Context<Self>) {
        self.grid_inline_details = !self.grid_inline_details;
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn toggle_grid_hover_details(&mut self, cx: &mut Context<Self>) {
        self.grid_hover_details = !self.grid_hover_details;
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn toggle_grid_pin_popup(&mut self, cx: &mut Context<Self>) {
        self.grid_pin_popup = !self.grid_pin_popup;
        if !self.grid_pin_popup {
            for state in [
                &mut self.palette_ui,
                &mut self.tileset_ui,
                &mut self.tilemap_ui,
                &mut self.sprites_ui,
                &mut self.bitmap_ui,
            ] {
                state.pinned = None;
            }
        }
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn toggle_grid_fill_width(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.grid_fill_width.remove(id) {
            self.grid_fill_width.insert(id.to_owned());
        }
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn on_log_filter_event(
        &mut self,
        input: Entity<TextInput>,
        _event: &TextInputEvent,
        cx: &mut Context<Self>,
    ) {
        let text = input.read(cx).content().to_string();
        self.logs.set_message(text);
        cx.notify();
    }

    pub(super) fn on_builtin_filter_event(
        &mut self,
        input: Entity<TextInput>,
        _event: &TextInputEvent,
        cx: &mut Context<Self>,
    ) {
        self.builtin_filter = input.read(cx).content().to_string().into();
        cx.notify();
    }

    pub(super) fn on_script_editor_event(
        &mut self,
        _editor: Entity<cirsium::components::CodeEditor>,
        event: &cirsium::components::CodeEditorEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            cirsium::components::CodeEditorEvent::Run => self.script_run(cx),
            cirsium::components::CodeEditorEvent::Save => self.script_save(cx),
            cirsium::components::CodeEditorEvent::SaveAs => self.script_save_as(cx),
            cirsium::components::CodeEditorEvent::Changed => cx.notify(),
        }
    }

    pub(super) fn script_run(&mut self, cx: &mut Context<Self>) {
        let source = self.script_editor.read(cx).text().to_string();
        self.script.run(&source, self.target.as_mut());

        let line = self.script.error_line();
        self.script_editor
            .update(cx, |editor, cx| editor.set_error_line(line, cx));

        if let Err(err) = self.session.write_script(&source) {
            tracing::warn!("could not persist script: {err}");
        }

        if let Some(paused) = self.script.take_pause_request() {
            self.set_running(!paused, cx);
        }

        cx.notify();
    }

    pub(super) fn script_stop(&mut self, cx: &mut Context<Self>) {
        self.script.stop();
        self.script_editor
            .update(cx, |editor, cx| editor.set_error_line(None, cx));
        cx.notify();
    }

    pub(super) fn script_revert(&mut self, cx: &mut Context<Self>) {
        self.script.stop();
        let template = crate::script::default_script(self.session.game().system);
        self.script_editor.update(cx, |editor, cx| {
            editor.set_text(template, cx);
            editor.set_error_line(None, cx);
        });
        self.script.note("editor reset to the starter script");
        cx.notify();
    }

    pub(super) fn script_clear_console(&mut self, cx: &mut Context<Self>) {
        self.script.clear_console();
        cx.notify();
    }

    pub(super) fn script_insert(
        &mut self,
        snippet: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.script_editor
            .update(cx, |editor, cx| editor.insert(&snippet, window, cx));
        cx.notify();
    }

    pub(crate) fn load_script_source(
        &mut self,
        name: String,
        source: String,
        cx: &mut Context<Self>,
    ) {
        self.script_editor.update(cx, |editor, cx| {
            editor.set_text(source, cx);
            editor.set_error_line(None, cx);
        });
        self.script_file_name = name.clone().into();
        #[cfg(target_family = "wasm")]
        {
            self.script_external = true;
        }
        #[cfg(not(target_family = "wasm"))]
        {
            self.script_path = None;
        }
        self.script.note(format!("loaded {name}"));
        cx.notify();
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn load_script_file(
        &mut self,
        path: std::path::PathBuf,
        source: String,
        cx: &mut Context<Self>,
    ) {
        let name = crate::file_name_of(&path);
        self.load_script_source(name, source, cx);
        self.script_path = Some(path);
    }

    pub(super) fn script_save(&mut self, cx: &mut Context<Self>) {
        let source = self.script_editor.read(cx).text().to_string();

        #[cfg(not(target_family = "wasm"))]
        if let Some(path) = self.script_path.clone() {
            self.write_script_file(path, source, cx);
            return;
        }

        #[cfg(target_family = "wasm")]
        if self.script_external {
            crate::save::save_bin(self.script_file_name.to_string(), source.into_bytes(), cx);
            self.script.note(format!("saved {}", self.script_file_name));
            cx.notify();
            return;
        }

        match self.session.write_script(&source) {
            Ok(()) => {
                self.script_file_name = storage::SCRIPT_FILE.into();
                #[cfg(target_family = "wasm")]
                {
                    self.script_external = false;
                }
                self.script.note("saved script.lua for this game");
            }
            Err(err) => {
                tracing::warn!("could not persist script: {err}");
                self.script.note(format!("save failed: {err}"));
            }
        }
        cx.notify();
    }

    pub(super) fn script_save_as(&mut self, cx: &mut Context<Self>) {
        let source = self.script_editor.read(cx).text().to_string();
        let suggested_name = if self.script_file_name.ends_with(".lua") {
            self.script_file_name.to_string()
        } else {
            storage::SCRIPT_FILE.to_string()
        };

        #[cfg(not(target_family = "wasm"))]
        {
            let directory = self
                .script_path
                .as_deref()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let prompt = cx.prompt_for_new_path(&directory, Some(&suggested_name));
            cx.spawn(async move |this, cx| {
                let path = match prompt.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Err(err)) => {
                        tracing::error!("save dialog failed: {err}");
                        return;
                    }
                    _ => return,
                };
                let _ = this.update(cx, |this, cx| this.write_script_file(path, source, cx));
            })
            .detach();
        }

        #[cfg(target_family = "wasm")]
        {
            crate::save::save_bin(suggested_name.clone(), source.into_bytes(), cx);
            self.script_file_name = suggested_name.into();
            self.script_external = true;
            self.script.note(format!("saved {}", self.script_file_name));
            cx.notify();
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn write_script_file(
        &mut self,
        path: std::path::PathBuf,
        source: String,
        cx: &mut Context<Self>,
    ) {
        crate::save::write_file(
            path,
            source.into_bytes(),
            cx,
            move |path, result, this, cx| {
                let name = crate::file_name_of(&path);
                match result {
                    Ok(()) => {
                        this.script_file_name = name.clone().into();
                        this.script_path = Some(path);
                        this.script.note(format!("saved {name}"));
                    }
                    Err(err) => {
                        tracing::error!("failed to write {}: {err}", path.display());
                        this.script.note(format!("save failed: {err}"));
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn script_open_from_disk(&mut self, cx: &mut Context<Self>) {
        crate::save::browse_for_file(cx, |path, this, cx| {
            #[cfg(not(target_family = "wasm"))]
            match std::fs::read_to_string(path) {
                Ok(source) => this.load_script_file(path.to_path_buf(), source, cx),
                Err(err) => tracing::error!("reading {}: {err}", path.display()),
            }
            #[cfg(target_family = "wasm")]
            let _ = (path, this, cx);
        });
    }

    pub(super) fn restore_script(&mut self, cx: &mut Context<Self>) {
        let template = crate::script::default_script(self.session.game().system);
        if self.script_editor.read(cx).text() != template {
            return;
        }

        match self.session.read_script() {
            Ok(Some(source)) if !source.trim().is_empty() => {
                self.script_editor
                    .update(cx, |editor, cx| editor.set_text(source, cx));
                self.script_file_name = storage::SCRIPT_FILE.into();
                #[cfg(target_family = "wasm")]
                {
                    self.script_external = false;
                }
                #[cfg(not(target_family = "wasm"))]
                {
                    self.script_path = None;
                }
                self.script.note("restored the script saved for this game");
            }
            Ok(_) => {}
            Err(err) => tracing::warn!("could not read stored script: {err}"),
        }
    }

    pub(super) fn log_open_target(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.logs.open_target_combo(position);
        cx.notify();
    }

    pub(super) fn log_close_target(&mut self, cx: &mut Context<Self>) {
        if self.logs.close_target_combo() {
            cx.notify();
        }
    }

    pub(super) fn log_set_target(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.logs.set_target(&id);
        cx.notify();
    }

    pub(super) fn log_open_level(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.logs.open_level_combo(position);
        cx.notify();
    }

    pub(super) fn log_close_level(&mut self, cx: &mut Context<Self>) {
        if self.logs.close_level_combo() {
            cx.notify();
        }
    }

    pub(super) fn log_set_level(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.logs.set_level(&id);
        cx.notify();
    }
}

impl Debugger {
    pub(super) fn slot_action(
        &mut self,
        action: cirsium::widgets::SlotAction,
        id: &str,
        cx: &mut Context<Self>,
    ) {
        use cirsium::widgets::SlotAction;

        let Some(slot) = storage::Slot::from_id(id) else {
            return;
        };

        match action {
            SlotAction::Select => self.select_slot(slot, cx),
            SlotAction::Save => self.save_state(slot, cx),
            SlotAction::Load => self.load_state(slot, cx),
            SlotAction::Delete => self.delete_slot(slot, cx),
        }
    }

    pub(super) fn save_state(&mut self, slot: storage::Slot, cx: &mut Context<Self>) {
        match self.session.save_state(&*self.target, slot) {
            Ok(bytes) => {
                self.set_state(&format!("saved {}", slot.label().to_lowercase()));
                self.refresh_slot(slot, Some(&bytes));
            }
            Err(err) => {
                self.set_state("save state failed");
                tracing::error!("save state: {err}");
            }
        }

        self.active_slot = slot;
        cx.notify();
    }

    pub(super) fn load_state(&mut self, slot: storage::Slot, cx: &mut Context<Self>) {
        match self.session.load_state(&mut *self.target, slot) {
            Ok(true) => {
                self.after_state_load(&slot.label().to_lowercase());
                self.active_slot = slot;
            }
            Ok(false) => self.set_state(&format!("{} is empty", slot.label().to_lowercase())),
            Err(err) => {
                self.set_state("load state failed");
                tracing::error!("load state: {err}");
            }
        }

        cx.notify();
    }

    pub(crate) fn apply_state(&mut self, bytes: &[u8], what: &str, cx: &mut Context<Self>) {
        match self.target.load_state(bytes) {
            Ok(()) => self.after_state_load(what),
            Err(err) => {
                self.set_state("load state failed");
                tracing::error!("loading state: {err}");
            }
        }

        cx.notify();
    }

    fn after_state_load(&mut self, what: &str) {
        self.invalidate_video_keys();
        self.refresh_framebuffer();
        self.follow_memory();
        self.set_state(&format!("loaded {what}"));
    }

    fn select_slot(&mut self, slot: storage::Slot, cx: &mut Context<Self>) {
        self.active_slot = slot;
        cx.notify();
    }

    pub(super) fn cycle_slot(&mut self, forward: bool, cx: &mut Context<Self>) {
        let slots = storage::Slot::ALL;
        let at = slots
            .iter()
            .position(|slot| *slot == self.active_slot)
            .unwrap_or(0);
        let next = if forward {
            (at + 1) % slots.len()
        } else {
            (at + slots.len() - 1) % slots.len()
        };

        self.active_slot = slots[next];
        self.set_state(&format!("{} selected", self.active_slot.label()));
        cx.notify();
    }

    fn delete_slot(&mut self, slot: storage::Slot, cx: &mut Context<Self>) {
        if let Err(err) = self.session.remove_state(slot) {
            tracing::error!("deleting state: {err}");
            self.set_state("delete failed");
        } else {
            self.set_state(&format!("cleared {}", slot.label().to_lowercase()));
            self.refresh_slot(slot, None);
        }

        cx.notify();
    }

    pub(crate) fn active_slot(&self) -> storage::Slot {
        self.active_slot
    }

    pub(crate) fn hydrate_saves(
        &self,
    ) -> impl std::future::Future<Output = Result<(), storage::StoreError>> + 'static {
        self.session.hydrate()
    }

    pub(crate) fn saves_loaded(&mut self, cx: &mut Context<Self>) {
        self.restore_save_ram();
        self.refresh_savestates();
        self.restore_script(cx);
        self.saves_ready = true;
        cx.notify();
    }

    fn restore_save_ram(&mut self) {
        if let Err(err) = self.session.restore_save_ram(&mut *self.target) {
            tracing::warn!("restoring save ram: {err}");
        }
    }

    pub(crate) fn persist_save_ram(&self) {
        if !self.saves_ready {
            return;
        }

        if let Err(err) = self.session.persist_save_ram(&*self.target) {
            tracing::error!("writing save ram: {err}");
        }
    }

    pub(crate) fn refresh_savestates(&mut self) {
        let occupied: std::collections::HashMap<storage::Slot, u64> = self
            .session
            .slots()
            .unwrap_or_else(|err| {
                tracing::error!("listing slots: {err}");
                Vec::new()
            })
            .into_iter()
            .map(|info| (info.slot, info.len))
            .collect();

        let slots = storage::Slot::ALL
            .into_iter()
            .map(|slot| match occupied.get(&slot).copied() {
                Some(len) => {
                    let bytes = self.session.read_state(slot).ok().flatten();
                    Self::slot_row(slot, Some(len), bytes.as_deref())
                }
                None => Self::slot_row(slot, None, None),
            })
            .collect();

        let screen = self.screen_size();
        self.savestates_view =
            std::sync::Arc::new(cirsium::widgets::SavestatesView { slots, screen });
    }

    fn refresh_slot(&mut self, slot: storage::Slot, bytes: Option<&[u8]>) {
        let row = Self::slot_row(slot, bytes.map(|b| b.len() as u64), bytes);
        let screen = self.screen_size();
        let mut view = (*self.savestates_view).clone();
        if let Some(existing) = view.slots.iter_mut().find(|slot| slot.id == row.id) {
            *existing = row;
        }
        view.screen = screen;
        self.savestates_view = std::sync::Arc::new(view);
    }

    fn screen_size(&self) -> Option<(u32, u32)> {
        self.target
            .software_frame()
            .filter(core_model::SoftwareFrame::is_valid)
            .map(|frame| (frame.width, frame.height))
    }

    fn slot_row(
        slot: storage::Slot,
        len: Option<u64>,
        bytes: Option<&[u8]>,
    ) -> cirsium::widgets::SavestateSlot {
        let thumbnail = bytes
            .and_then(core_model::state::peek_thumbnail)
            .and_then(|thumb| {
                cirsium::widgets::thumbnail_image(thumb.width, thumb.height, thumb.pixels)
            });

        cirsium::widgets::SavestateSlot {
            id: slot.id().into(),
            label: slot.label().into(),
            detail: match len {
                Some(len) => format!("{} KiB", len / 1024).into(),
                None => "empty".into(),
            },
            occupied: len.is_some(),
            thumbnail,
        }
    }

    pub(crate) fn savestates_view(&self) -> std::sync::Arc<cirsium::widgets::SavestatesView> {
        self.savestates_view.clone()
    }
}
