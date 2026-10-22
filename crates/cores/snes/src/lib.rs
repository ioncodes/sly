mod alu;
pub mod apu;
pub mod callstack;
pub mod cartridge;
pub mod cpu;
#[cfg(feature = "debug")]
pub mod debug;
pub mod error;
pub mod interrupt;
pub mod joypad;
mod macros;
mod math;
pub mod mmio;
pub mod ppu;
pub mod sa1;
pub mod scheduler;
pub mod state;
pub mod superfx;

pub use cartridge::Cartridge;
pub use cpu::Cpu;
pub use error::SnesError;
pub use mmio::{CartridgeMap, MemoryMap, Mmio};

use crate::apu::Apu;
use crate::cpu::disasm::Disasm;
use crate::interrupt::Irq;
use crate::joypad::Joypads;
use crate::math::Math;
use crate::mmio::Address;
use crate::mmio::dma::{self, Dma};
use crate::ppu::Ppu;
use crate::sa1::Sa1;
use crate::scheduler::{EventKind, Scheduler};
use crate::superfx::SuperFx;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Deref, DerefMut};

pub struct Snes<M: MemoryMap = CartridgeMap> {
    pub cpu: Cpu,
    bus: Bus<M>,
}

pub struct Bus<M: MemoryMap = CartridgeMap> {
    pub mmu: Mmio<M>,
    pub irq: Irq,
    pub dma: Dma,
    pub apu: Apu,
    pub ppu: Ppu,
    pub math: Math,
    pub joypads: Joypads,
    pub scheduler: Scheduler,
    /// State for the cartridge's SA-1 coprocessor.
    pub sa1: Sa1,
    pub superfx: SuperFx,
}

#[derive(Deserialize)]
#[serde(bound = "")]
struct SerializedSnes<M: MemoryMap> {
    cpu: Cpu,
    mmu: Mmio<M>,
    irq: Irq,
    dma: Dma,
    apu: Apu,
    ppu: Ppu,
    math: Math,
    joypads: Joypads,
    scheduler: Scheduler,
    sa1: Sa1,
    superfx: SuperFx,
}

impl<M: MemoryMap> Serialize for Snes<M> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Snes", 11)?;
        state.serialize_field("cpu", &self.cpu)?;
        state.serialize_field("mmu", &self.mmu)?;
        state.serialize_field("irq", &self.irq)?;
        state.serialize_field("dma", &self.dma)?;
        state.serialize_field("apu", &self.apu)?;
        state.serialize_field("ppu", &self.ppu)?;
        state.serialize_field("math", &self.math)?;
        state.serialize_field("joypads", &self.joypads)?;
        state.serialize_field("scheduler", &self.scheduler)?;
        state.serialize_field("sa1", &self.sa1)?;
        state.serialize_field("superfx", &self.superfx)?;
        state.end()
    }
}

impl<'de, M: MemoryMap> Deserialize<'de> for Snes<M> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let state = SerializedSnes::deserialize(deserializer)?;
        Ok(Self {
            cpu: state.cpu,
            bus: Bus {
                mmu: state.mmu,
                irq: state.irq,
                dma: state.dma,
                apu: state.apu,
                ppu: state.ppu,
                math: state.math,
                joypads: state.joypads,
                scheduler: state.scheduler,
                sa1: state.sa1,
                superfx: state.superfx,
            },
        })
    }
}

impl<M: MemoryMap> Deref for Snes<M> {
    type Target = Bus<M>;

    fn deref(&self) -> &Self::Target {
        &self.bus
    }
}

impl<M: MemoryMap> DerefMut for Snes<M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bus
    }
}

impl Snes {
    pub fn new(cartridge: Cartridge) -> Self {
        Self::with_mmu(Mmio::new(cartridge))
    }
}

impl<M: MemoryMap> Snes<M> {
    pub fn with_mmu(mmu: Mmio<M>) -> Self {
        let tv_system = cartridge::tv_system(mmu.cartridge.region());
        let mut snes = Self {
            cpu: Cpu::new(0),
            bus: Bus {
                mmu,
                irq: Irq::default(),
                dma: Dma::default(),
                apu: Apu::new(),
                ppu: Ppu::new(),
                math: Math::new(),
                joypads: Joypads::default(),
                scheduler: Scheduler::new(tv_system),
                sa1: Sa1::default(),
                superfx: SuperFx::default(),
            },
        };
        snes.reset();
        snes
    }

    pub fn reset(&mut self) {
        const RESET_VECTOR_ADDR: u16 = 0xFFFC;
        const BANK_ZERO: u8 = 0x00;

        let tv_system = cartridge::tv_system(self.mmu.cartridge.region());

        self.cpu = Cpu::new(0);
        self.irq = Irq::default();
        self.dma = Dma::default();
        self.apu = Apu::new();
        self.ppu = Ppu::new();
        self.joypads.reset();
        self.sa1 = Sa1::default();
        self.superfx = SuperFx::default();

        self.scheduler = Scheduler::new(tv_system);
        let spc_delay = self.scheduler.spc_master_clocks(1);
        let frame_cycles = self.scheduler.cycles_per_frame();
        self.scheduler
            .schedule_event(spc_delay, EventKind::Spc700Tick);
        self.scheduler.schedule_periodic_at(
            scheduler::VBLANK_START_SCANLINE * scheduler::MASTER_CLOCKS_PER_SCANLINE,
            frame_cycles,
            EventKind::VblankStart,
        );
        self.scheduler
            .schedule_periodic(frame_cycles, EventKind::VblankEnd);
        self.scheduler.schedule_periodic(
            scheduler::MASTER_CLOCKS_PER_SCANLINE,
            EventKind::DrawScanline,
        );
        self.scheduler.schedule_periodic_at(
            dma::HDMA_INIT_CLOCK,
            frame_cycles,
            EventKind::InitHdma,
        );
        self.scheduler.schedule_periodic_at(
            dma::HDMA_RUN_CLOCK,
            scheduler::MASTER_CLOCKS_PER_SCANLINE,
            EventKind::RunHdma,
        );
        self.scheduler.schedule_periodic(
            scheduler::MASTER_CLOCKS_PER_SCANLINE,
            EventKind::HvTimerLine,
        );
        self.scheduler.schedule_periodic_at(
            scheduler::REFRESH_START_CLOCK,
            scheduler::MASTER_CLOCKS_PER_SCANLINE,
            EventKind::DramRefresh,
        );

        self.scheduler
            .schedule_periodic_spc(apu::SPC_TIMER01_DIVIDER, EventKind::Timer01Tick);
        self.scheduler
            .schedule_periodic_spc(apu::SPC_TIMER2_DIVIDER, EventKind::Timer2Tick);
        self.scheduler
            .schedule_periodic_spc(apu::DSP_SAMPLE_PERIOD_SPC, EventKind::DspTick);

        self.mmu.reset();
        self.cpu.pc = self.read16(Address::new(BANK_ZERO, RESET_VECTOR_ADDR));
    }

    pub fn disassemble_at(&self, pc: u16) -> Disasm {
        cpu::disassemble_at(self, pc)
    }

    pub fn disassemble_next(&self) -> Disasm {
        self.disassemble_at(self.cpu.pc)
    }

    pub fn step(&mut self) {
        self.step_until_sa1(|_| false);
    }

    pub(crate) fn step_until_sa1(&mut self, stop: impl FnMut(&Cpu) -> bool) {
        cpu::step(self);
        self.bus.sync();
        let clocks = self.mmu.clocks;
        sa1::run_until_if(&mut self.bus, clocks, stop);
    }

    pub fn run_frame(&mut self) {
        let end = self.scheduler.frame_deadline();
        while self.scheduler.cycles() < end {
            self.step();
        }
    }
}

impl<M: MemoryMap> Bus<M> {
    #[inline]
    pub(crate) fn sync(&mut self) {
        let clocks = self.mmu.clocks;
        self.scheduler.advance_to(clocks);
        if self.scheduler.event_due(clocks) {
            self.drain_events();
        }
    }

    #[inline(never)]
    fn drain_events(&mut self) {
        while let Some((deadline, kind)) = self.scheduler.pop_current_event() {
            kind.dispatch(self, deadline);
        }
    }
}

impl<M: MemoryMap> core_model::Core for Snes<M> {
    fn reset(&mut self) {
        Snes::reset(self);
    }

    fn step(&mut self) {
        Snes::step(self);
    }

    fn run_frame(&mut self) {
        Snes::run_frame(self);
    }

    fn audio_spec(&self) -> Option<core_model::AudioSpec> {
        Some(core_model::AudioSpec {
            sample_rate: (apu::SPC700_CLOCK / apu::DSP_SAMPLE_PERIOD_SPC) as u32,
            channels: 2,
        })
    }

    fn drain_audio(&mut self, out: &mut Vec<i16>) {
        self.apu.drain_audio(out);
    }

    fn set_button(&mut self, player: usize, button: core_model::Button, pressed: bool) {
        self.joypads.set_button(player, button, pressed);
    }

    fn controllers(&self) -> Vec<core_model::ControllerDescriptor> {
        (0..2)
            .map(|port| core_model::ControllerDescriptor {
                port,
                name: format!("Controller {}", port + 1).into(),
                buttons: core_model::Button::CLASSIC.to_vec(),
            })
            .collect()
    }

    fn session_info(&self) -> core_model::SessionInfo {
        core_model::SessionInfo {
            system: "SNES".into(),
            session: self.mmu.cartridge.title().into(),
            segments: vec![
                self.mmu.cartridge.region().to_string().into(),
                format!("{} KB", self.mmu.cartridge.rom.len() / 1024).into(),
            ],
        }
    }

    fn cartridge_info(&self) -> Option<core_model::CartridgeInfo> {
        Some(self.mmu.cartridge.info())
    }

    fn software_frame(&self) -> Option<core_model::SoftwareFrame<'_>> {
        Some(core_model::SoftwareFrame {
            width: ppu::SCREEN_WIDTH,
            height: ppu::SCREEN_HEIGHT,
            pixels: self.ppu.frame_buffer(),
        })
    }

    fn save_state(&self) -> Result<Vec<u8>, core_model::StateError> {
        Snes::save_state(self)
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), core_model::StateError> {
        Snes::load_state(self, bytes)
    }

    fn save_ram(&self) -> Option<&[u8]> {
        Snes::save_ram(self)
    }

    fn load_save_ram(&mut self, bytes: &[u8]) -> bool {
        Snes::load_save_ram(self, bytes)
    }

    fn frame_interval(&self) -> Option<std::time::Duration> {
        let nanos = self.scheduler.cycles_per_frame() as u128 * 1_000_000_000
            / self.scheduler.master_clock_hz() as u128;
        Some(std::time::Duration::from_nanos(nanos as u64))
    }
}
