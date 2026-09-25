//! Passive HDMI DDC capture on XIAO D4/D5 (GPIO6/GPIO7).
//! PIO observes both input pins and pushes START, STOP, and nine-bit words.
//! The IRQ only moves FIFO words into SRAM; protocol parsing runs in `poll`.

use core::cell::RefCell;

use critical_section::Mutex;
use hal::{
    gpio::{
        bank0::{Gpio6, Gpio7},
        FunctionPio0, FunctionSioInput, Pin, PullNone,
    },
    pac::{self, PIO0},
    pio::{Buffers, PIOBuilder, PIOExt, PinDir, PioIRQ, Running, Rx, StateMachine, SM0},
};
use heapless::Deque;
use rp235x_hal as hal;

use crate::ddc_protocol::{Decoder, GAP, START, STOP};

type Sda = Pin<Gpio6, FunctionSioInput, PullNone>;
type Scl = Pin<Gpio7, FunctionSioInput, PullNone>;
type PioSda = Pin<Gpio6, FunctionPio0, PullNone>;
type PioScl = Pin<Gpio7, FunctionPio0, PullNone>;
type DdcRx = Rx<(PIO0, SM0)>;

const RING_WORDS: usize = 512;
pub const DIAGNOSTIC_LEN: usize = 57;

#[derive(Clone, Copy, Default)]
struct CaptureStats {
    starts: u32,
    stops: u32,
    words: u32,
    fifo_stalls: u32,
    ring_overflows: u32,
    ring_peak: u16,
    trace_len: u8,
    trace: [u32; 2],
}

struct Capture {
    rx: DdcRx,
    ring: Deque<u32, RING_WORDS>,
    stats: CaptureStats,
}

impl Capture {
    fn enqueue(&mut self, word: u32) {
        match word {
            START => {
                self.stats.starts = self.stats.starts.wrapping_add(1);
                self.stats.trace_len = 0;
            }
            STOP => self.stats.stops = self.stats.stops.wrapping_add(1),
            GAP => {}
            _ => {
                self.stats.words = self.stats.words.wrapping_add(1);
                if self.stats.trace_len < 2 {
                    self.stats.trace[self.stats.trace_len as usize] = word;
                    self.stats.trace_len += 1;
                }
            }
        }
        if self.ring.push_back(word).is_err() {
            self.stats.ring_overflows = self.stats.ring_overflows.wrapping_add(1);
            self.ring.clear();
            // The decoder must wait for a new START after any lost word.
            let _ = self.ring.push_back(GAP);
        }
        self.stats.ring_peak = self.stats.ring_peak.max(self.ring.len() as u16);
    }
}

static CAPTURE: Mutex<RefCell<Option<Capture>>> = Mutex::new(RefCell::new(None));

pub struct LedState {
    pub address_known: bool,
    pub saw_start: bool,
    pub capture_error: bool,
}

pub struct Ddc {
    decoder: Decoder,
    _sda: PioSda,
    _scl: PioScl,
    _sm: StateMachine<(PIO0, SM0), Running>,
    _pio: hal::pio::PIO<PIO0>,
}

impl Ddc {
    pub fn poll(&mut self) {
        // The IRQ keeps filling the ring while foreground work and USB run.
        for _ in 0..64 {
            let word = critical_section::with(|cs| {
                CAPTURE
                    .borrow_ref_mut(cs)
                    .as_mut()
                    .and_then(|capture| capture.ring.pop_front())
            });
            let Some(word) = word else { break };
            self.decoder.push(word);
        }
    }

    pub fn physical_address(&self) -> u16 {
        self.decoder.physical_address()
    }

    pub fn led_state(&self) -> LedState {
        let stats = critical_section::with(|cs| CAPTURE.borrow_ref(cs).as_ref().unwrap().stats);
        LedState {
            address_known: self.physical_address() != 0xffff,
            saw_start: stats.starts != 0,
            capture_error: stats.fifo_stalls != 0 || stats.ring_overflows != 0,
        }
    }

    pub fn diagnostic_bytes(&self) -> [u8; DIAGNOSTIC_LEN] {
        let (capture, ring_used) = critical_section::with(|cs| {
            let borrow = CAPTURE.borrow_ref(cs);
            let capture = borrow.as_ref().unwrap();
            (capture.stats, capture.ring.len() as u16)
        });
        // SIO GPIO_IN observes the pads even while PIO owns their function.
        let pins = unsafe { &*pac::SIO::ptr() }.gpio_in().read().bits();
        let levels = ((pins >> 6) & 3) as u8;
        let decoder = self.decoder.stats();
        let mut bytes = [0; DIAGNOSTIC_LEN];
        bytes[0] = 1; // diagnostic format version
        bytes[1] = levels; // bit 0: SDA, bit 1: SCL
        bytes[2..4].copy_from_slice(&ring_used.to_be_bytes());
        bytes[4..6].copy_from_slice(&capture.ring_peak.to_be_bytes());
        for (slot, value) in [
            capture.starts,
            capture.stops,
            capture.words,
            capture.fifo_stalls,
            capture.ring_overflows,
        ]
        .into_iter()
        .enumerate()
        {
            let offset = 6 + slot * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        bytes[26] = capture.trace_len;
        bytes[27..31].copy_from_slice(&capture.trace[0].to_be_bytes());
        bytes[31..35].copy_from_slice(&capture.trace[1].to_be_bytes());
        for (slot, value) in [
            decoder.edid_write_addresses,
            decoder.edid_read_addresses,
            decoder.edid_bytes,
            decoder.invalid_words,
            decoder.unacknowledged_addresses,
        ]
        .into_iter()
        .enumerate()
        {
            let offset = 35 + slot * 4;
            bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        let pio = unsafe { &*PIO0::ptr() };
        bytes[55] = pio.sm(0).sm_addr().read().bits() as u8;
        let fifo = pio.fstat().read();
        bytes[56] = u8::from(fifo.rxempty().bits() & 1 != 0)
            | (u8::from(fifo.rxfull().bits() & 1 != 0) << 1);
        bytes
    }
}

pub fn init(sda: Sda, scl: Scl, pio0: PIO0, resets: &mut pac::RESETS) -> Ddc {
    // MOV x/y, PINS reads GPIO6 (SDA) as bit 0 and GPIO7 (SCL) as bit 1.
    // JMP PIN tests GPIO7. The program waits for a real SDA fall while SCL
    // is high, then samples SDA once per SCL high period. A change in SDA
    // before SCL falls is a START or STOP instead of a data bit.
    let program = pio::pio_asm!(
        ".wrap_target",
        "idle:",
        "wait 1 gpio 7",
        "wait 1 gpio 6",
        "wait 0 gpio 6",
        "mov x, pins",
        "jmp pin start",
        "jmp idle",
        "start:",
        "mov isr, x",
        "push block",
        "bit:",
        "wait 0 gpio 7",
        "wait 1 gpio 7",
        "mov y, pins",
        "changed:",
        "mov x, pins",
        "jmp x!=y, handle",
        "jmp changed",
        "handle:",
        "jmp pin sda_event",
        "in y, 1",
        "jmp bit",
        "sda_event:",
        "mov isr, x",
        "push block",
        "set y, 2",
        "jmp x!=y, idle",
        "jmp bit",
        ".wrap"
    )
    .program;

    let (mut pio, sm0, _, _, _) = pio0.split(resets);
    let installed = pio.install(&program).unwrap();
    let (mut sm, rx, _tx) = PIOBuilder::from_installed_program(installed)
        .in_pin_base(6)
        .in_count(2)
        .jmp_pin(7)
        .autopush(true)
        .push_threshold(9)
        .buffers(Buffers::OnlyRx)
        .build(sm0);
    sm.set_pindirs([(6, PinDir::Input), (7, PinDir::Input)]);
    let sda = sda.into_function::<FunctionPio0>();
    let scl = scl.into_function::<FunctionPio0>();

    rx.enable_rx_not_empty_interrupt(PioIRQ::Irq0);
    critical_section::with(|cs| {
        CAPTURE.borrow_ref_mut(cs).replace(Capture {
            rx,
            ring: Deque::new(),
            stats: CaptureStats::default(),
        });
    });
    unsafe { hal::arch::interrupt_unmask(pac::Interrupt::PIO0_IRQ_0) };
    let sm = sm.start();

    Ddc {
        decoder: Decoder::new(),
        _sda: sda,
        _scl: scl,
        _sm: sm,
        _pio: pio,
    }
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
fn PIO0_IRQ_0() {
    critical_section::with(|cs| {
        let mut borrow = CAPTURE.borrow_ref_mut(cs);
        let Some(capture) = borrow.as_mut() else {
            return;
        };

        while let Some(word) = capture.rx.read() {
            capture.enqueue(word);
        }
        // RXSTALL is sticky. A stalled state machine can miss bus transitions
        // even if all subsequently queued FIFO words are read successfully.
        let pio = unsafe { &*PIO0::ptr() };
        if pio.fdebug().read().rxstall().bits() & 1 != 0 {
            pio.fdebug().write(|w| unsafe { w.rxstall().bits(1) });
            capture.stats.fifo_stalls = capture.stats.fifo_stalls.wrapping_add(1);
            capture.ring.clear();
            capture.enqueue(GAP);
        }
    });
}
