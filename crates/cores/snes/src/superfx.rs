pub(crate) mod bitmap;
pub(crate) mod decoder;
mod interpreter;
pub mod io;
pub mod map;
pub mod regs;

use crate::Bus;
use crate::mmio::{Address, CartRegion, MemoryMap, OPEN_BUS};
use crate::scheduler::EventKind;
use regs::{Cfgr, Clsr, Colr, Por, Scmr, Sfr};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

pub use bitmap::PixelCache;

pub const VERSION_CODE: u8 = 0x04;

#[derive(Serialize, Deserialize)]
pub struct SuperFx {
    pub r: [u16; 16],
    pub sfr: Sfr,
    pub pbr: u8,
    pub rombr: u8,
    pub rambr: u8,
    pub cbr: u16,
    pub scbr: u8,
    pub scmr: Scmr,
    pub cfgr: Cfgr,
    pub clsr: Clsr,
    pub bramr: u8,
    pub sreg: u8,
    pub dreg: u8,
    pub pipeline: u8,
    pub ram_addr: u16,
    pub colr: Colr,
    pub por: Por,
    pub cache: CodeCache,

    pub rom_buffer: RomBuffer,
    pub ram_buffer: RamWriteBuffer,
    pub wait_rom: bool,
    pub wait_ram: bool,
    pub pixel_primary: PixelCache,
    pub pixel_secondary: PixelCache,

    #[cfg(feature = "debug")]
    #[serde(skip)]
    pub pipeline_addr: u16,

    #[cfg(feature = "debug")]
    #[serde(skip)]
    pub stopped_screen: Option<(u8, u8, u8)>,

    #[cfg(feature = "debug")]
    #[serde(skip)]
    step_debt: u64,

    tick_scheduled: bool,
    latch: u8,
}

impl Default for SuperFx {
    fn default() -> Self {
        Self {
            r: [0; 16],
            sfr: Sfr::default(),
            pbr: 0,
            rombr: 0,
            rambr: 0,
            cbr: 0,
            scbr: 0,
            scmr: Scmr::default(),
            cfgr: Cfgr::default(),
            clsr: Clsr::default(),
            bramr: 0,
            sreg: 0,
            dreg: 0,
            pipeline: 0x01,
            ram_addr: 0,
            colr: Colr::default(),
            por: Por::default(),
            cache: CodeCache::default(),

            rom_buffer: RomBuffer::default(),
            ram_buffer: RamWriteBuffer::default(),
            wait_rom: false,
            wait_ram: false,
            pixel_primary: PixelCache::default(),
            pixel_secondary: PixelCache::default(),

            #[cfg(feature = "debug")]
            pipeline_addr: 0,

            #[cfg(feature = "debug")]
            stopped_screen: None,

            #[cfg(feature = "debug")]
            step_debt: 0,

            tick_scheduled: false,
            latch: 0,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct RomBuffer {
    pub value: u8,
    pub delay: u8,
}

#[derive(Default, Serialize, Deserialize)]
pub struct RamWriteBuffer {
    pub addr: u16,
    pub value: u8,
    pub delay: u8,
}

impl RomBuffer {
    pub(crate) fn step(&mut self, cycles: u64) -> bool {
        count_down(&mut self.delay, cycles)
    }
}

impl RamWriteBuffer {
    pub(crate) fn queue(&mut self, addr: u16, value: u8, delay: u8) {
        self.addr = addr;
        self.value = value;
        self.delay = delay;
    }

    pub(crate) fn step(&mut self, cycles: u64) -> bool {
        count_down(&mut self.delay, cycles)
    }
}

fn count_down(delay: &mut u8, cycles: u64) -> bool {
    if *delay == 0 {
        return false;
    }
    *delay = delay.saturating_sub(cycles.min(u8::MAX as u64) as u8);
    *delay == 0
}

#[derive(Serialize, Deserialize)]
pub struct CodeCache {
    #[serde(with = "BigArray")]
    pub data: [u8; 0x200],
    pub valid: u32,
}

impl Default for CodeCache {
    fn default() -> Self {
        Self {
            data: [0; 0x200],
            valid: 0,
        }
    }
}

impl SuperFx {
    pub(crate) fn owns_rom(&self) -> bool {
        self.sfr.go() && self.scmr.rom_to_gsu()
    }

    pub(crate) fn owns_ram(&self) -> bool {
        self.sfr.go() && self.scmr.ram_to_gsu()
    }

    pub(crate) fn bus_cost(&self) -> u64 {
        if self.clsr.fast_clock() { 5 } else { 6 }
    }

    pub(crate) fn cache_cost(&self) -> u64 {
        if self.clsr.fast_clock() { 1 } else { 2 }
    }

    pub(crate) fn trigger_rom_prefetch(&mut self) {
        self.sfr.set_rom_busy(true);
        self.rom_buffer.delay = self.bus_cost() as u8;
    }

    pub(crate) fn force_stop(&mut self) {
        self.sfr.set_go(false);
        self.cbr = 0;
        self.cache.invalidate();
    }
}

pub(crate) fn snes_cart_read<M: MemoryMap>(
    snes: &Bus<M>,
    region: CartRegion,
    index: usize,
    addr: Address,
) -> u8 {
    match region {
        CartRegion::Rom if snes.superfx.owns_rom() => vector_byte(addr.address()),
        CartRegion::Ram if snes.superfx.owns_ram() => OPEN_BUS,
        _ => snes.mmu.cart_read(region, index),
    }
}

pub(crate) fn snes_cart_write<M: MemoryMap>(
    snes: &mut Bus<M>,
    region: CartRegion,
    index: usize,
    value: u8,
) {
    if region == CartRegion::Ram && snes.superfx.owns_ram() {
        return;
    }
    snes.mmu.cart_write(region, index, value);
}

fn vector_byte(addr: u16) -> u8 {
    match addr & 0x0F {
        n if n & 1 == 1 => 0x01,
        0x4 => 0x04,
        0xA => 0x08,
        0xE => 0x0C,
        _ => 0x00,
    }
}

impl CodeCache {
    fn invalidate(&mut self) {
        self.valid = 0;
    }

    fn mmio_index(cbr: u16, index: usize) -> usize {
        (cbr as usize + index) & 0x1FF
    }

    fn read(&self, cbr: u16, index: usize) -> u8 {
        self.data[Self::mmio_index(cbr, index)]
    }

    fn line_valid(&self, offset: u16) -> bool {
        self.valid & (1 << (offset >> 4)) != 0
    }

    fn mark_line(&mut self, offset: u16) {
        self.valid |= 1 << (offset >> 4);
    }

    fn write(&mut self, cbr: u16, index: usize, value: u8) {
        let index = Self::mmio_index(cbr, index);
        self.data[index] = value;
        if index & 0xF == 0xF {
            self.mark_line(index as u16);
        }
    }
}

#[cfg(feature = "debug")]
pub(crate) fn debug_step<M: MemoryMap>(snes: &mut Bus<M>) -> bool {
    if stalled(&snes.superfx) {
        return false;
    }
    snes.superfx.step_debt += interpreter::step(snes);
    true
}

pub(crate) fn start<M: MemoryMap>(snes: &mut Bus<M>) {
    snes.superfx.sfr.set_go(true);
    resume(snes);
}

fn resume<M: MemoryMap>(snes: &mut Bus<M>) {
    let now = snes.scheduler.cycles();
    schedule_tick(snes, now + 1);
}

fn schedule_tick<M: MemoryMap>(snes: &mut Bus<M>, deadline: u64) {
    if !snes.superfx.tick_scheduled {
        snes.superfx.tick_scheduled = true;
        snes.scheduler.schedule_event(deadline, EventKind::GsuTick);
    }
}

const TICK_BUDGET: u64 = 512;

fn stalled(fx: &SuperFx) -> bool {
    !fx.sfr.go() || fx.wait_rom || fx.wait_ram
}

pub(crate) fn gsu_tick<M: MemoryMap>(snes: &mut Bus<M>, deadline: u64) {
    snes.superfx.tick_scheduled = false;

    #[cfg(not(feature = "debug"))]
    let mut spent = 0;
    #[cfg(feature = "debug")]
    let mut spent = std::mem::take(&mut snes.superfx.step_debt);

    while spent < TICK_BUDGET && !stalled(&snes.superfx) {
        spent += interpreter::step(snes);
    }

    if stalled(&snes.superfx) {
        interpreter::drain_buffers(snes);
    } else {
        schedule_tick(snes, deadline + spent);
    }
}

pub(crate) fn bus_grants_updated<M: MemoryMap>(snes: &mut Bus<M>) {
    let fx = &mut snes.superfx;
    if fx.scmr.rom_to_gsu() {
        fx.wait_rom = false;
    }
    if fx.scmr.ram_to_gsu() {
        fx.wait_ram = false;
    }

    if !stalled(&snes.superfx) {
        resume(snes);
    }
}
