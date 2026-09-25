//! XIAO RP2350 onboard WS2812 on GPIO22, with power enabled by GPIO23.

use hal::{
    gpio::{
        bank0::{Gpio22, Gpio23},
        FunctionPio1, FunctionSioInput, FunctionSioOutput, Pin, PinState, PullNone,
    },
    pac::{self, PIO1},
    pio::{Buffers, PIOBuilder, PIOExt, PinDir, Running, ShiftDirection, StateMachine, Tx, SM0},
};
use rp235x_hal as hal;

use crate::ddc::LedState;

type DataPin = Pin<Gpio22, FunctionSioInput, PullNone>;
type PowerPin = Pin<Gpio23, FunctionSioInput, PullNone>;

pub struct StatusLed {
    _data: Pin<Gpio22, FunctionPio1, PullNone>,
    _power: Pin<Gpio23, FunctionSioOutput, PullNone>,
    _sm: StateMachine<(PIO1, SM0), Running>,
    _pio: hal::pio::PIO<PIO1>,
    tx: Tx<(PIO1, SM0)>,
    color: Option<u32>,
}

impl StatusLed {
    pub fn new(
        data: DataPin,
        power: PowerPin,
        pio1: PIO1,
        resets: &mut pac::RESETS,
        system_hz: u32,
    ) -> Self {
        // The board's RGB LED requires GPIO23 high before it can receive data.
        let power = power.into_push_pull_output_in_state(PinState::High);
        let program = pio::pio_asm!(
            ".side_set 1",
            ".wrap_target",
            "bitloop:",
            "out x, 1 side 0 [2]",
            "jmp !x do_zero side 1 [1]",
            "jmp bitloop side 1 [4]",
            "do_zero:",
            "nop side 0 [4]",
            ".wrap"
        )
        .program;
        let (mut pio, sm0, _, _, _) = pio1.split(resets);
        let installed = pio.install(&program).unwrap();
        let divisor = ((u64::from(system_hz) * 256) / 8_000_000) as u32;
        let (mut sm, _rx, tx) = PIOBuilder::from_installed_program(installed)
            .side_set_pin_base(22)
            .out_shift_direction(ShiftDirection::Left)
            .autopull(true)
            .pull_threshold(24)
            .clock_divisor_fixed_point((divisor >> 8) as u16, divisor as u8)
            .buffers(Buffers::OnlyTx)
            .build(sm0);
        sm.set_pindirs([(22, PinDir::Output)]);
        let data = data.into_function::<FunctionPio1>();
        let sm = sm.start();
        Self {
            _data: data,
            _power: power,
            _sm: sm,
            _pio: pio,
            tx,
            color: None,
        }
    }

    pub fn update(&mut self, state: LedState) {
        // WS2812 bytes are green, red, blue. Keep brightness low on the board.
        let color = if state.capture_error {
            0x000800 // red
        } else if state.address_known {
            0x080000 // green
        } else if state.saw_start {
            0x030800 // amber
        } else {
            0x000008 // blue
        };
        if self.color != Some(color) && self.tx.write(color << 8) {
            self.color = Some(color);
        }
    }
}
