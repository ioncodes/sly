use super::*;

impl Debugger {
    pub(super) fn refresh_pane_view(
        target: &dyn DebugCore,
        pane: &mut ProcessorPane,
        mode: CpuRefresh,
    ) {
        pane.cpu_view = Arc::new(
            Self::track_changes(target.cpu_view(&pane.id), &mut pane.reg_changes, mode).into(),
        );
        pane.disasm.recompute(target);
        pane.call_stack = Arc::new(target.call_stack(&pane.id).into());
    }

    fn selected_palette(&self) -> Option<PaletteSelection> {
        let (bank, color) = self.palette_selected?;
        Some(PaletteSelection {
            resource: self.palette_ui.resource.clone()?,
            color: self.palette_view.color_index(bank, color)?,
        })
    }

    pub(super) fn resources_of(&self, kind: VideoResourceKind) -> Vec<VideoResource> {
        self.video_resources
            .iter()
            .filter(|r| r.kind == kind)
            .cloned()
            .collect()
    }

    fn video_key(
        &self,
        resource: Option<&VideoResourceId>,
        palette: Option<PaletteSelection>,
    ) -> Option<VideoCacheKey> {
        VideoCacheKey::read(self.target.video()?, resource?, palette)
    }

    pub(super) fn invalidate_video_keys(&mut self) {
        self.palette_key = None;
        self.tileset_key = None;
        self.tilemap_key = None;
        self.tilemap_overlay_key = None;
        self.sprites_key = None;
        self.bitmap_key = None;
    }

    pub(super) fn drop_grid_image(cx: &mut Context<Self>, image: Option<Arc<RenderImage>>) {
        if let Some(image) = image {
            cx.defer(move |cx| cx.drop_image(image, None));
        }
    }

    pub(super) fn refresh_palette(&mut self) {
        let key = self.video_key(self.palette_ui.resource.as_ref(), None);
        if key.is_some() && key == self.palette_key {
            return;
        }

        self.palette_key = key;
        self.palette_view = Arc::new(
            self.target
                .video()
                .and_then(|v| v.palette(self.palette_ui.resource.as_ref()?))
                .unwrap_or_default(),
        );

        if self
            .palette_selected
            .is_some_and(|(p, c)| self.palette_view.color_index(p, c).is_none())
        {
            self.palette_selected = None;
        }
    }

    pub(super) fn refresh_tileset(&mut self, cx: &mut Context<Self>) {
        let palette = self.selected_palette();
        let key = self.video_key(self.tileset_ui.resource.as_ref(), palette.clone());
        if key.is_some() && key == self.tileset_key && self.tileset.is_some() {
            return;
        }

        self.tileset_key = key;
        Self::drop_grid_image(cx, self.tileset.take().and_then(|v| v.image));
        self.tileset = self
            .target
            .video()
            .and_then(|v| v.tileset(self.tileset_ui.resource.as_ref()?, palette.as_ref()))
            .map(Into::into);
    }

    pub(super) fn refresh_tilemap(&mut self, cx: &mut Context<Self>) {
        let overlay_key = self.video_key(self.tilemap_ui.resource.as_ref(), None);
        let key = overlay_key.clone().map(|mut key| {
            key.revision.overlay = 0;
            key
        });

        if key.is_some() && key == self.tilemap_key && self.tilemap.is_some() {
            self.apply_tilemap_overlay(overlay_key);
            return;
        }

        self.tilemap_key = key;
        Self::drop_grid_image(cx, self.tilemap.take().and_then(BackgroundLayerView::image));
        self.tilemap = self
            .target
            .video()
            .and_then(|v| BackgroundLayerView::load(v, self.tilemap_ui.resource.as_ref()?));

        self.tilemap_overlay_key = None;
        self.apply_tilemap_overlay(overlay_key);
    }

    fn refresh_tilemap_overlay(&mut self) {
        let key = self.video_key(self.tilemap_ui.resource.as_ref(), None);
        self.apply_tilemap_overlay(key);
    }

    fn apply_tilemap_overlay(&mut self, key: Option<VideoCacheKey>) {
        let Some(video) = self.target.video() else {
            return;
        };

        let Some(resource) = self.tilemap_ui.resource.as_ref() else {
            return;
        };

        if let Some(BackgroundLayerView::Tilemap(view)) = &mut self.tilemap {
            let viewport = video.tilemap_viewport(resource);
            view.viewport = viewport.filter(|v| v.transform.is_none()).map(Into::into);
            view.footprint = viewport.filter(|v| v.transform.is_some());
            if key.is_none() || key != self.tilemap_overlay_key {
                view.set_overlays(video.tilemap_overlays(resource));
            }
        }

        self.tilemap_overlay_key = key;
    }

    pub(super) fn refresh_sprites(&mut self, cx: &mut Context<Self>) {
        if self.is_collapsed(SpritesWidget::ID) {
            return;
        }

        let key = self.video_key(self.sprites_ui.resource.as_ref(), None);
        if key.is_some() && key == self.sprites_key && self.sprites.is_some() {
            return;
        }

        self.sprites_key = key;
        Self::drop_grid_image(cx, self.sprites.take().and_then(|v| v.image));
        self.sprites = self
            .target
            .video()
            .and_then(|v| v.sprites(self.sprites_ui.resource.as_ref()?))
            .map(Into::into);
    }

    pub(super) fn refresh_bitmap(&mut self, cx: &mut Context<Self>) {
        let key = self.video_key(self.bitmap_ui.resource.as_ref(), None);
        if key.is_some() && key == self.bitmap_key && self.bitmap.is_some() {
            return;
        }

        self.bitmap_key = key;
        Self::drop_grid_image(cx, self.bitmap.take().and_then(|v| v.image));
        self.bitmap = self
            .target
            .video()
            .and_then(|v| v.bitmap(self.bitmap_ui.resource.as_ref()?))
            .map(Into::into);
    }

    pub(super) fn refresh_framebuffer(&mut self) {
        if self.is_collapsed(FramebufferWidget::ID) {
            return;
        }

        let fb = self.target.software_frame();

        if let (Some(view), Some(fb)) = (&mut self.framebuffer, fb)
            && view.update(fb)
        {
            return;
        }

        self.framebuffer = fb.map(Into::into);
    }

    pub(super) fn should_refresh(&self, panel: &PanelDescriptor, live: bool) -> bool {
        !live || self.live_refresh || panel.refresh == RefreshPolicy::Continuous
    }

    pub(super) fn wants_refresh(&self, live: bool, kind: impl Fn(&PanelKind) -> bool) -> bool {
        self.registry
            .manifest()
            .in_workspace(&self.workspace)
            .any(|panel| kind(&panel.kind) && self.should_refresh(panel, live))
    }

    pub(super) fn refresh_panes(&mut self, mode: CpuRefresh, live: bool, cx: &mut Context<Self>) {
        for ix in 0..self.panes.len() {
            let id = self.panes[ix].id.clone();
            let visible = self.wants_refresh(live, |kind| {
                matches!(
                    kind,
                    PanelKind::Registers { cpu } | PanelKind::Disassembly { cpu }
                        if **cpu == *id.as_str()
                )
            });

            if visible {
                Self::refresh_pane_view(self.target.as_ref(), &mut self.panes[ix], mode);
            }
        }

        if self.wants_refresh(live, |kind| matches!(kind, PanelKind::Mmio)) {
            self.mmio_view = Arc::new(self.target.mmio_view().into());
        }

        if self.wants_refresh(live, |kind| matches!(kind, PanelKind::Framebuffer)) {
            self.refresh_framebuffer();
        }

        if self.wants_refresh(live, |kind| matches!(kind, PanelKind::Memory)) {
            self.memory.refresh_cached(self.target.as_ref());
        }

        self.refresh_video_panes(live, cx);
        self.refresh_generic_panels(live);
        if self.show_log && self.wants_refresh(live, |kind| matches!(kind, PanelKind::Log)) {
            self.logs.refresh();
        }
        self.registry.refresh_visible(
            &self.workspace,
            cirsium::PanelRefresh { live },
            self.live_refresh,
            cx,
        );
    }

    pub(super) fn refresh_video_panes(&mut self, live: bool, cx: &mut Context<Self>) {
        self.video_resources = self
            .target
            .video()
            .map(|v| v.resources())
            .unwrap_or_default();

        self.palette_ui
            .sync(&self.video_resources, VideoResourceKind::Palette);
        self.tileset_ui
            .sync(&self.video_resources, VideoResourceKind::Tileset);
        self.tilemap_ui
            .sync(&self.video_resources, VideoResourceKind::Tilemap);
        self.sprites_ui
            .sync(&self.video_resources, VideoResourceKind::Sprites);
        self.bitmap_ui
            .sync(&self.video_resources, VideoResourceKind::Bitmap);
        if self.wants_refresh(live, |k| matches!(k, PanelKind::Palette)) {
            self.refresh_palette();
        }

        if self.wants_refresh(live, |k| matches!(k, PanelKind::Tileset)) {
            self.refresh_tileset(cx);
        }

        if self.wants_refresh(live, |k| matches!(k, PanelKind::Tilemap)) {
            self.refresh_tilemap(cx);
        } else if live && self.tilemap_ui.show_window_overlays && self.tilemap.is_some() {
            self.refresh_tilemap_overlay();
        }

        if self.wants_refresh(live, |k| matches!(k, PanelKind::Sprites)) {
            self.refresh_sprites(cx);
        }

        if self.wants_refresh(live, |k| matches!(k, PanelKind::Bitmap)) {
            self.refresh_bitmap(cx);
        }
    }

    pub(super) fn refresh_generic_panels(&mut self, live: bool) {
        let generic = self.visible_generic_panels(live);

        for id in generic.tables {
            let view = Arc::new(self.target.table_view(id.as_ref()));
            self.table_views.insert(id, view);
        }

        for (id, selection) in generic.structured {
            let view = Arc::new(self.target.structured_view(id.as_ref(), selection));
            self.structured_views.insert(id, view);
        }

        for id in generic.scopes {
            let view = self.target.scopes_view(id.as_ref());
            self.scopes_views.insert(id, view);
        }
    }

    pub(super) fn visible_generic_panels(&self, live: bool) -> GenericRefresh {
        let mut refresh = GenericRefresh::default();

        for panel in self.registry.manifest().in_workspace(&self.workspace) {
            if !self.should_refresh(panel, live) || self.is_collapsed(panel.id.as_ref()) {
                continue;
            }

            let id = SharedString::from(panel.id.clone());
            match panel.kind {
                PanelKind::Table => refresh.tables.push(id),
                PanelKind::Structured => {
                    let selection = panel
                        .selection_from
                        .as_ref()
                        .and_then(|source| self.panel_selection(source.as_ref()));

                    refresh.structured.push((id, selection));
                }
                PanelKind::ChannelScopes => refresh.scopes.push(id),
                _ => {}
            }
        }

        refresh
    }

    fn panel_selection(&self, id: &str) -> Option<usize> {
        match id {
            TilesetWidget::ID => self.tileset_ui.selected(),
            TilemapWidget::ID => self.tilemap_ui.selected(),
            SpritesWidget::ID => self.sprites_ui.selected(),
            BitmapWidget::ID => self.bitmap_ui.selected(),
            _ => self.table_selection.get(id).copied().or(Some(0)),
        }
    }

    pub(super) fn table_selected(&self, id: &str) -> usize {
        self.table_selection.get(id).copied().unwrap_or(0)
    }

    pub(super) fn push_scope(buffer: &mut Vec<f32>, samples: &[i16], channels: u8) {
        const SCOPE_LEN: usize = 512;

        if channels == 0 {
            return;
        }

        for frame in samples.chunks_exact(channels as usize) {
            let mono = frame.iter().map(|&s| s as f32).sum::<f32>() / channels as f32 / 32768.0;
            buffer.push(mono);
        }

        let len = buffer.len();
        if len > SCOPE_LEN {
            buffer.drain(..len - SCOPE_LEN);
        }
    }

    pub(super) fn track_changes(
        mut cpu: CpuView,
        changes: &mut ChangeTracker<Arc<str>>,
        mode: CpuRefresh,
    ) -> CpuView {
        if mode == CpuRefresh::Baseline {
            changes.clear();
        }

        for reg in &mut cpu.registers {
            let (key, value) = (reg.name.clone(), reg.value as u64);
            let highlight = changes.highlight(key, value);
            if matches!(reg.highlight, Highlight::Execution | Highlight::Pipeline) {
                continue;
            }
            reg.highlight = match mode {
                CpuRefresh::Highlight => highlight,
                CpuRefresh::Baseline => Highlight::None,
            };
        }
        cpu
    }

    pub(super) fn is_collapsed(&self, id: &str) -> bool {
        self.collapsed.contains(id)
            || self
                .band_host(id)
                .is_some_and(|h| self.collapsed.contains(h))
    }

    fn band_host(&self, id: &str) -> Option<&str> {
        self.registry
            .manifest()
            .host_of(self.workspace.as_ref(), id)
    }

    pub(super) fn toggle_in(set: &mut HashSet<SharedString>, id: SharedString) {
        if !set.remove(&id) {
            set.insert(id);
        }
    }

    pub(super) fn toggle(&mut self, id: impl Into<SharedString>, cx: &mut Context<Self>) {
        let id = id.into();
        let host = self.band_host(&id).map(SharedString::from);
        let id = host.unwrap_or(id);
        Self::toggle_in(&mut self.collapsed, id.clone());

        if !self.collapsed.contains(&id) {
            self.refresh_framebuffer();
            self.refresh_video_panes(false, cx);
            self.refresh_generic_panels(false);
        }

        cx.notify();
    }

    pub(super) fn expanded_set(&self, widget: &str) -> HashSet<SharedString> {
        self.expanded.get(widget).cloned().unwrap_or_default()
    }

    pub(super) fn toggle_expanded(
        &mut self,
        widget: impl Into<SharedString>,
        id: SharedString,
        cx: &mut Context<Self>,
    ) {
        Self::toggle_in(self.expanded.entry(widget.into()).or_default(), id);
        cx.notify();
    }

    pub(super) fn select_palette_color(
        &mut self,
        palette: usize,
        color: usize,
        cx: &mut Context<Self>,
    ) {
        let selected = Some((palette, color));
        self.palette_selected = (self.palette_selected != selected).then_some((palette, color));
        self.refresh_tileset(cx);
        cx.notify();
    }

    pub(super) fn select_table_row(
        &mut self,
        table_id: SharedString,
        row: usize,
        cx: &mut Context<Self>,
    ) {
        self.table_selection.insert(table_id.clone(), row);

        let linked: Vec<SharedString> = self
            .registry
            .manifest()
            .in_workspace(&self.workspace)
            .filter(|p| p.selection_from.as_deref() == Some(table_id.as_ref()))
            .map(|p| SharedString::from(p.id.clone()))
            .collect();
        for id in linked {
            let view = Arc::new(self.target.structured_view(id.as_ref(), Some(row)));
            self.structured_views.insert(id, view);
        }

        cx.notify();
    }

    pub(super) fn wire_grid(
        &self,
        mut panel: GridPanel,
        id: &'static str,
        state: &GridState,
        resources: Vec<VideoResource>,
        ui: fn(&mut Self) -> &mut GridState,
        refresh: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> GridPanel {
        let this = cx.entity();
        if let Some(descriptor) = self.registry.descriptor(id) {
            panel = panel.title(descriptor.title.clone());
        }

        panel
            .layer(state.index(&resources))
            .layers(resources.clone())
            .selected(state.selected())
            .hovered(state.hovered)
            .pinned(state.pinned)
            .inline_details(self.grid_inline_details)
            .hover_details(self.grid_hover_details)
            .fill_width(self.grid_fill_width.contains(id))
            .on_toggle_fill_width({
                let this = this.clone();
                move |_, cx| this.update(cx, |d, cx| d.toggle_grid_fill_width(id, cx))
            })
            .viewport(state.viewport)
            .window_overlays_visible(state.show_window_overlays)
            .collapsed(self.is_collapsed(id))
            .on_toggle(cx.listener(move |this, _, _, cx| this.toggle(id, cx)))
            .on_layer_select({
                let this = this.clone();
                move |layer, _, cx| {
                    this.update(cx, |d, cx| {
                        ui(d).resource = resources.get(layer).map(|r| r.id.clone());
                        ui(d).hovered = None;
                        ui(d).pinned = None;
                        refresh(d, cx);
                        d.refresh_generic_panels(false);
                        cx.notify();
                    })
                }
            })
            .on_select({
                let this = this.clone();
                move |index, _, cx| {
                    this.update(cx, |d, cx| {
                        let pin = d.grid_pin_popup;
                        ui(d).toggle_selected(index, pin);
                        d.refresh_generic_panels(false);
                        cx.notify();
                    })
                }
            })
            .on_hover({
                let this = this.clone();
                move |hovered, _, cx| {
                    this.update(cx, |d, cx| {
                        if ui(d).set_hovered(hovered) {
                            cx.notify();
                        }
                    })
                }
            })
            .on_viewport({
                let this = this.clone();
                move |viewport, _, cx| {
                    this.update(cx, |d, cx| {
                        if ui(d).set_viewport(viewport) {
                            cx.notify();
                        }
                    })
                }
            })
            .on_toggle_window_overlays({
                let this = this.clone();
                move |_, cx| {
                    this.update(cx, |d, cx| {
                        let state = ui(d);
                        state.show_window_overlays = !state.show_window_overlays;
                        cx.notify();
                    })
                }
            })
    }
}
