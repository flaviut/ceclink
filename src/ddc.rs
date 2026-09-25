//! Receive-only HDMI DDC observer on XIAO D4/D5 (GPIO6/GPIO7).
//! It never enables an output driver on either pin.
use core::{
    cell::RefCell,
    sync::atomic::{AtomicU16, Ordering},
};
use critical_section::Mutex;
use embedded_hal::digital::InputPin;
use hal::gpio::{
    self,
    bank0::{Gpio6, Gpio7},
    FunctionSioInput, Pin, PullNone,
};
use rp235x_hal as hal;

type Sda = Pin<Gpio6, FunctionSioInput, PullNone>;
type Scl = Pin<Gpio7, FunctionSioInput, PullNone>;

static STATE: Mutex<RefCell<Option<Sniffer>>> = Mutex::new(RefCell::new(None));
static PHYSICAL_ADDRESS: AtomicU16 = AtomicU16::new(0xffff);

pub fn physical_address() -> u16 {
    PHYSICAL_ADDRESS.load(Ordering::Relaxed)
}

pub fn init(sda: Sda, scl: Scl) {
    sda.set_interrupt_enabled(gpio::Interrupt::EdgeLow, true);
    sda.set_interrupt_enabled(gpio::Interrupt::EdgeHigh, true);
    scl.set_interrupt_enabled(gpio::Interrupt::EdgeHigh, true);
    critical_section::with(|cs| STATE.borrow_ref_mut(cs).replace(Sniffer::new(sda, scl)));
    unsafe {
        hal::arch::interrupt_unmask(hal::pac::Interrupt::IO_IRQ_BANK0);
    }
}

struct Sniffer {
    sda: Sda,
    scl: Scl,
    active: bool,
    bits: u8,
    byte: u8,
    byte_index: u16,
    target: u8,
    reading: bool,
    offset: u8,
    segment: u8,
    edid: [u8; 256],
    seen: [u8; 32],
}

impl Sniffer {
    fn new(sda: Sda, scl: Scl) -> Self {
        Self {
            sda,
            scl,
            active: false,
            bits: 0,
            byte: 0,
            byte_index: 0,
            target: 0,
            reading: false,
            offset: 0,
            segment: 0,
            edid: [0; 256],
            seen: [0; 32],
        }
    }

    fn start(&mut self) {
        self.active = true;
        self.bits = 0;
        self.byte = 0;
        self.byte_index = 0;
    }

    fn rising_clock(&mut self) {
        if !self.active {
            return;
        }
        if self.bits == 8 {
            self.bits = 0; // ninth clock is ACK/NACK
            self.byte = 0;
            return;
        }
        self.byte = (self.byte << 1) | u8::from(self.sda.is_high().unwrap_or(false));
        self.bits += 1;
        if self.bits == 8 {
            let value = self.byte;
            if self.byte_index == 0 {
                self.target = value >> 1;
                self.reading = value & 1 != 0;
            } else if self.target == 0x50 {
                if self.reading {
                    let index = (u16::from(self.segment) * 256 + u16::from(self.offset)) as usize;
                    if index < self.edid.len() {
                        self.edid[index] = value;
                        self.seen[index / 8] |= 1 << (index % 8);
                        self.find_physical_address();
                    }
                    self.offset = self.offset.wrapping_add(1);
                } else if self.byte_index == 1 {
                    self.offset = value;
                }
            } else if self.target == 0x30 && !self.reading && self.byte_index == 1 {
                self.segment = value;
            }
            self.byte_index += 1;
        }
    }

    fn captured(&self, index: usize) -> bool {
        self.seen[index / 8] & (1 << (index % 8)) != 0
    }

    fn find_physical_address(&self) {
        if !self.captured(128) || self.edid[128] != 0x02 || !self.captured(130) {
            return;
        }
        let end = self.edid[130].min(127) as usize + 128;
        let mut index = 132;
        while index < end {
            if !self.captured(index) {
                return;
            }
            let header = self.edid[index];
            let next = index + 1 + (header & 31) as usize;
            if next > end {
                return;
            }
            if header >> 5 == 3
                && next >= index + 6
                && (index + 1..index + 6).all(|i| self.captured(i))
                && self.edid[index + 1..index + 4] == [0x03, 0x0c, 0x00]
            {
                let address = u16::from_be_bytes([self.edid[index + 4], self.edid[index + 5]]);
                PHYSICAL_ADDRESS.store(address, Ordering::Relaxed);
                return;
            }
            index = next;
        }
    }
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
fn IO_IRQ_BANK0() {
    critical_section::with(|cs| {
        let mut borrow = STATE.borrow_ref_mut(cs);
        let Some(state) = borrow.as_mut() else {
            return;
        };
        let sda_low = state.sda.interrupt_status(gpio::Interrupt::EdgeLow);
        let sda_high = state.sda.interrupt_status(gpio::Interrupt::EdgeHigh);
        let scl_high = state.scl.interrupt_status(gpio::Interrupt::EdgeHigh);
        if sda_low {
            state.sda.clear_interrupt(gpio::Interrupt::EdgeLow);
        }
        if sda_high {
            state.sda.clear_interrupt(gpio::Interrupt::EdgeHigh);
        }
        if scl_high {
            state.scl.clear_interrupt(gpio::Interrupt::EdgeHigh);
        }
        if state.scl.is_high().unwrap_or(false) {
            if sda_low {
                state.start();
            }
            if sda_high {
                state.active = false;
            }
        }
        if scl_high {
            state.rising_clock();
        }
    });
}
