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

use crate::ddc_protocol::{Decoder, GAP};

type Sda = Pin<Gpio6, FunctionSioInput, PullNone>;
type Scl = Pin<Gpio7, FunctionSioInput, PullNone>;
type PioSda = Pin<Gpio6, FunctionPio0, PullNone>;
type PioScl = Pin<Gpio7, FunctionPio0, PullNone>;
type DdcRx = Rx<(PIO0, SM0)>;

const RING_WORDS: usize = 512;

struct Capture {
    rx: DdcRx,
    ring: Deque<u32, RING_WORDS>,
}

impl Capture {
    fn enqueue(&mut self, word: u32) {
        if self.ring.push_back(word).is_err() {
            self.ring.clear();
            // The decoder must wait for a new START after any lost word.
            let _ = self.ring.push_back(GAP);
        }
    }
}

static CAPTURE: Mutex<RefCell<Option<Capture>>> = Mutex::new(RefCell::new(None));

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
            capture.ring.clear();
            capture.enqueue(GAP);
        }
    });
}
