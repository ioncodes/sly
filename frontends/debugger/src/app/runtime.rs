use super::*;

impl Debugger {
    pub(super) fn set_workspace(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.workspace = id;
        self.refresh_panes(CpuRefresh::Highlight, false, cx);
        cx.notify();
    }

    pub(super) fn set_accent(&mut self, index: usize, cx: &mut Context<Self>) {
        self.accent = index;
        let mode = Theme::global(cx).mode;
        set_theme(cx, super::render::themed(mode, index));
        self.save_preferences(cx);
        cx.notify();
    }

    pub(super) fn set_state(&mut self, label: &str) {
        self.status.state = self.target.caption(label).into();
    }

    pub(super) fn status_with_perf(&self) -> StatusView {
        let mut status = self.status.clone();
        status.segments.extend(self.perf.iter().cloned());
        status
    }

    pub(super) fn run_frame_interval(&self) -> Duration {
        self.target
            .frame_interval()
            .unwrap_or(DEFAULT_FRAME_INTERVAL)
    }

    #[cfg(not(target_family = "wasm"))]
    pub(super) fn next_run_wait(next_frame: &mut Instant, interval: Duration) -> Duration {
        *next_frame += interval;

        let now = Instant::now();
        let wait = next_frame.saturating_duration_since(now);
        if wait.is_zero() {
            if now.duration_since(*next_frame) >= interval {
                *next_frame = now;
            }
            FRAME_PACE_YIELD
        } else {
            wait
        }
    }

    pub(super) fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        let execution = if running {
            ExecutionState::Running
        } else {
            ExecutionState::Paused
        };
        if self.execution == execution {
            return;
        }

        self.execution = execution;
        self.set_state(if running { "running" } else { "paused" });

        if running {
            self.emu_clock = Instant::now();

            #[cfg(not(target_family = "wasm"))]
            {
                self.run_task = Some(cx.spawn(async move |this, cx| {
                    let mut next_frame = Instant::now();
                    let mut wake_late = Duration::ZERO;

                    loop {
                        let interval = this
                            .update(cx, |this, cx| {
                                this.run_tick(wake_late, cx)
                                    .then(|| this.run_frame_interval())
                            })
                            .unwrap_or(None);
                        let Some(interval) = interval else {
                            break;
                        };

                        let wait = Self::next_run_wait(&mut next_frame, interval);
                        let before = Instant::now();
                        cx.background_executor().timer(wait).await;
                        wake_late = before.elapsed().saturating_sub(wait);
                    }
                }));
            }
        } else {
            self.run_task = None;
            self.stats.reset();
            self.perf.clear();
            self.invalidate_video_keys();
            self.refresh_panes(CpuRefresh::Highlight, false, cx);
        }

        cx.notify();
    }

    pub(super) fn scope_visible(&self) -> bool {
        self.registry
            .manifest()
            .in_workspace(&self.workspace)
            .any(|p| matches!(p.kind, PanelKind::AudioScope) && !self.is_collapsed(p.id.as_ref()))
    }

    fn update_access_tracking(&mut self) {
        let visible = self
            .registry
            .manifest()
            .in_workspace(&self.workspace)
            .any(|p| {
                matches!(p.kind, PanelKind::Memory | PanelKind::Mmio)
                    && !self.is_collapsed(p.id.as_ref())
            });

        let tracking = if visible {
            debugger_model::AccessTracking::Highlights
        } else if self.memory.follow_enabled() {
            debugger_model::AccessTracking::Latest
        } else {
            debugger_model::AccessTracking::Off
        };

        self.target.track_accesses(tracking);
    }

    fn run_frame_hooked(&mut self) -> RunOutcome {
        self.update_access_tracking();
        if !self.script.is_active() {
            self.target.watch_events(&[]);
            return self.target.debug_run_frame(self.breakpoints.marks());
        }

        let mut marks = std::mem::take(&mut self.hook_marks);
        marks.clear();
        marks.extend_from_slice(self.breakpoints.marks());
        marks.extend_from_slice(self.script.marks());

        let outcome = self.run_hook_stops(&mut marks);
        self.hook_marks = marks;
        outcome
    }

    fn run_hook_stops(&mut self, marks: &mut Vec<BreakpointMark>) -> RunOutcome {
        const MAX_HOOK_STOPS: usize = 4096;

        for _ in 0..MAX_HOOK_STOPS {
            self.target.watch_events(self.script.event_watches());
            let outcome = self.target.debug_run_frame(marks);

            let hits = self.target.take_hits();
            let events = self.target.take_events();

            let mut user_break = false;
            let mut script_stop = !events.is_empty();
            for hit in &hits {
                let marks = self.breakpoints.marks();
                if marks.iter().any(|mark| mark.covers_hit(hit)) {
                    user_break = true;
                } else {
                    script_stop = true;
                }
            }

            self.script.dispatch(self.target.as_mut(), &hits);
            self.script.dispatch_events(self.target.as_mut(), &events);

            marks.clear();
            marks.extend_from_slice(self.breakpoints.marks());
            marks.extend_from_slice(self.script.marks());

            if self.script.take_pause_request() == Some(true) {
                return RunOutcome::Breakpoint;
            }

            if outcome == RunOutcome::FrameComplete {
                return RunOutcome::FrameComplete;
            }

            if user_break {
                return RunOutcome::Breakpoint;
            }

            if !script_stop {
                return outcome;
            }
        }

        self.script
            .note("hook budget for this frame exhausted, pausing");
        RunOutcome::BudgetExhausted
    }

    pub(super) fn emulate_frame(&mut self, scope_visible: bool) -> RunOutcome {
        let emu_start = Instant::now();
        let outcome = self.run_frame_hooked();
        if outcome == RunOutcome::FrameComplete {
            self.script.on_frame(self.target.as_mut());
        }

        self.stats.record_frame(emu_start.elapsed());
        self.emu_clock += self.run_frame_interval();

        self.audio_samples.clear();
        self.target.drain_audio(&mut self.audio_samples);
        self.audio.queue(&self.audio_samples);

        if scope_visible {
            Self::push_scope(
                &mut self.scope_samples,
                &self.audio_samples,
                self.target.audio_spec().map_or(0, |s| s.channels),
            );

            self.scope_view = AudioScopeView {
                samples: self.scope_samples.as_slice().into(),
            };
        }

        outcome
    }

    pub(super) fn frames_due(&mut self) -> u32 {
        let interval = self.run_frame_interval();
        let now = Instant::now();

        let max_debt = interval * MAX_CATCHUP_FRAMES;
        if now.saturating_duration_since(self.emu_clock) > max_debt {
            self.emu_clock = now - max_debt;
        }

        let deficit = now.saturating_duration_since(self.emu_clock);
        let mut due = (deficit.as_nanos() / interval.as_nanos().max(1)) as u32;

        match self.audio.pacing() {
            Some((buffered, band)) if buffered >= band.end => {
                due = due.saturating_sub(1);

                self.emu_clock += interval;
                if self.emu_clock > now {
                    self.emu_clock = now;
                }
            }
            Some((buffered, band)) if buffered < band.start => {
                due += 1;

                self.emu_clock = self
                    .emu_clock
                    .checked_sub(interval)
                    .unwrap_or(self.emu_clock);
            }
            _ => {}
        }

        due.min(MAX_CATCHUP_FRAMES)
    }

    pub(super) fn run_tick(&mut self, wake_late: Duration, cx: &mut Context<Self>) -> bool {
        if !self.execution.is_running() {
            return false;
        }

        let mut outcome = RunOutcome::FrameComplete;
        let scope_visible = self.scope_visible();
        for _ in 0..self.frames_due() {
            outcome = self.emulate_frame(scope_visible);
            if outcome != RunOutcome::FrameComplete {
                break;
            }
        }

        if self.script.take_pause_request() == Some(true) {
            outcome = RunOutcome::Breakpoint;
        }

        if let Some(segments) = self.stats.record_tick(wake_late, self.audio.buffered()) {
            self.perf = segments.into_iter().map(Into::into).collect();
        }

        self.flush_save_ram();

        let keep_going = outcome == RunOutcome::FrameComplete;
        if !keep_going {
            self.invalidate_video_keys();
        }

        self.refresh_panes(CpuRefresh::Highlight, keep_going, cx);
        self.follow_memory();

        if !keep_going {
            self.execution = ExecutionState::Paused;
            self.stats.reset();
            self.perf.clear();
        }

        self.set_state(match outcome {
            RunOutcome::FrameComplete => "running",
            RunOutcome::Breakpoint => "breakpoint",
            RunOutcome::Event => "event",
            RunOutcome::TargetReached => "target reached",
            RunOutcome::BudgetExhausted => "execution budget exhausted",
            RunOutcome::Unsupported => "unsupported operation",
        });

        cx.notify();
        keep_going
    }

    fn flush_save_ram(&mut self) {
        if self.sram_flushed.elapsed() < SAVE_RAM_FLUSH {
            return;
        }

        self.sram_flushed = Instant::now();
        self.persist_save_ram();
    }

    pub(super) fn step(&mut self, cx: &mut Context<Self>) {
        self.update_access_tracking();
        self.target.debug_step();
        self.finish_manual_step("paused", cx);
    }

    fn finish_manual_step(&mut self, state: &str, cx: &mut Context<Self>) {
        self.invalidate_video_keys();

        self.refresh_panes(CpuRefresh::Highlight, false, cx);
        self.set_state(state);
        self.execution = ExecutionState::Paused;
        self.run_task = None;
        self.follow_memory();

        cx.notify();
    }

    pub(super) fn run_until(&mut self, target: &str, cx: &mut Context<Self>) {
        self.update_access_tracking();
        let outcome = self.target.run_until(target);
        self.finish_manual_step(
            match outcome {
                RunOutcome::TargetReached => "target reached",
                RunOutcome::Breakpoint => "breakpoint",
                RunOutcome::Event => "event",
                RunOutcome::FrameComplete => "frame complete",
                RunOutcome::BudgetExhausted => "no event within execution budget",
                RunOutcome::Unsupported => "unsupported step target",
            },
            cx,
        );
    }

    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        self.target.reset();

        self.invalidate_video_keys();

        self.refresh_panes(CpuRefresh::Baseline, false, cx);
        self.set_state("reset");
        self.execution = ExecutionState::Paused;
        self.run_task = None;

        cx.notify();
    }
}
