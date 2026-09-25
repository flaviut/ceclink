//! CEC timing engine. Core 1 owns the bidirectional open-drain D9 pin.
use crate::transport::{self, Event, Frame, MAX_FRAME};
use embedded_hal::digital::{InputPin, OutputPin};
use hal::gpio::{bank0::Gpio4, FunctionSioInput, InOutPin, Pin, PullNone};
use hal::timer::{CopyableTimer0, Timer};
use rp235x_hal as hal;

type CecPin = InOutPin<Pin<Gpio4, FunctionSioInput, PullNone>>;

struct Bus {
    pin: CecPin,
    timer: Timer<CopyableTimer0>,
    last_activity: u32,
}

impl Bus {
    fn now(&self) -> u32 {
        self.timer.get_counter_low()
    }
    fn high(&mut self) -> bool {
        self.pin.is_high().unwrap_or(false)
    }
    fn release(&mut self) {
        let _ = self.pin.set_high();
    }
    fn low(&mut self) {
        let _ = self.pin.set_low();
    }

    fn delay(&self, us: u32) {
        let start = self.now();
        while self.now().wrapping_sub(start) < us {
            cortex_m::asm::nop();
        }
    }

    fn wait_level(&mut self, high: bool, timeout: u32) -> bool {
        let start = self.now();
        while self.high() != high {
            if self.now().wrapping_sub(start) >= timeout {
                return false;
            }
        }
        true
    }

    fn receive_bit(&mut self) -> Option<bool> {
        if !self.wait_level(false, 3500) {
            return None;
        }
        let start = self.now();
        while self.now().wrapping_sub(start) < 1050 {}
        let bit = self.high();
        if !self.wait_level(true, 1500) {
            return None;
        }
        Some(bit)
    }

    fn receive(&mut self) {
        let start = self.now();
        if !self.wait_level(true, 4300) {
            return;
        }
        let low_time = self.now().wrapping_sub(start);
        if !(3500..=3900).contains(&low_time) {
            return;
        }
        let mut frame = Frame::default();
        for _ in 0..MAX_FRAME {
            let mut value = 0u8;
            for _ in 0..8 {
                let Some(bit) = self.receive_bit() else {
                    return;
                };
                value = (value << 1) | u8::from(bit);
            }
            let Some(eom) = self.receive_bit() else {
                return;
            };
            frame.bytes[frame.len as usize] = value;
            frame.len += 1;

            if !self.wait_level(false, 3500) {
                return;
            }
            let ack_start = self.now();
            let address = frame.bytes[0] & 0x0f;
            let ack = address != 15 && transport::should_ack(address);
            if ack {
                // Extend the initiator's ACK low pulse immediately. Its own
                // low pulse may end at 400 us; waiting creates another edge.
                self.low();
            }
            while self.now().wrapping_sub(ack_start) < 1500 {}
            if ack {
                self.release();
            }
            if !self.wait_level(true, 1000) {
                return;
            }
            if eom {
                transport::push_event(Event::Received(frame));
                return;
            }
        }
    }

    fn send_bit(&mut self, bit: bool) -> Result<bool, ()> {
        self.low();
        self.delay(if bit { 600 } else { 1500 });
        self.release();
        let observed_low = if bit {
            self.delay(450);
            let low = !self.high();
            self.delay(1350);
            low
        } else {
            self.delay(900);
            false
        };
        if self.high() {
            Ok(observed_low)
        } else {
            Err(())
        }
    }

    fn send(&mut self, frame: Frame) -> Event {
        self.low();
        self.delay(3700);
        self.release();
        self.delay(800);
        for index in 0..frame.len as usize {
            for bit_index in (0..8).rev() {
                let bit = frame.bytes[index] & (1 << bit_index) != 0;
                match self.send_bit(bit) {
                    Ok(true) if bit => return Event::LineError, // lost arbitration
                    Ok(_) => {}
                    Err(_) => return Event::LineError,
                }
            }
            if self.send_bit(index + 1 == frame.len as usize).is_err() {
                return Event::LineError;
            }
            let Ok(ack_low) = self.send_bit(true) else {
                return Event::LineError;
            };
            let broadcast = frame.bytes[0] & 0x0f == 15;
            if broadcast == ack_low {
                return Event::Nack;
            }
        }
        Event::Sent
    }
}

pub fn run(pin: Pin<Gpio4, FunctionSioInput, PullNone>, timer: Timer<CopyableTimer0>) -> ! {
    let mut bus = Bus {
        pin: InOutPin::new(pin),
        timer,
        last_activity: 0,
    };
    bus.release();
    bus.last_activity = bus.now();
    let mut pending = None;
    let mut was_high = bus.high();
    loop {
        let mut high = bus.high();
        if was_high && !high {
            bus.receive();
            high = bus.high();
            bus.last_activity = bus.now();
        }
        was_high = high;
        if pending.is_none() {
            pending = transport::take_pending();
        }
        if high && bus.now().wrapping_sub(bus.last_activity) >= 12000 {
            if let Some(frame) = pending.take() {
                let event = bus.send(frame);
                transport::push_event(event);
                bus.last_activity = bus.now();
                was_high = bus.high();
            }
        }
    }
}
