//! Framing and the small Pulse-Eight command subset used by Linux pulse8-cec.
use crate::transport::{self, Event, Frame, MAX_FRAME};
use heapless::Deque;

const START: u8 = 0xff;
const END: u8 = 0xfe;
const ESCAPE: u8 = 0xfd;
const ACCEPTED: u8 = 0x08;
const REJECTED: u8 = 0x09;

pub struct Protocol {
    input: [u8; 32],
    input_len: usize,
    started: bool,
    escaped: bool,
    output: Deque<u8, 256>,
    transmit: Frame,
    sniffed_address: u16,
    host_address: Option<u16>,
}

impl Protocol {
    pub fn new() -> Self {
        Self {
            input: [0; 32],
            input_len: 0,
            started: false,
            escaped: false,
            output: Deque::new(),
            transmit: Frame::default(),
            sniffed_address: 0xffff,
            host_address: None,
        }
    }

    pub fn set_physical_address(&mut self, address: u16) {
        self.sniffed_address = address;
    }

    pub fn next_output(&mut self) -> Option<u8> {
        self.output.pop_front()
    }

    fn packet(&mut self, payload: &[u8]) {
        let encoded_len = 2 + payload
            .iter()
            .map(|b| if *b >= ESCAPE { 2 } else { 1 })
            .sum::<usize>();
        if self.output.capacity() - self.output.len() < encoded_len {
            return;
        }
        let _ = self.output.push_back(START);
        for &byte in payload {
            if byte >= ESCAPE {
                let _ = self.output.push_back(ESCAPE);
                let _ = self.output.push_back(byte - 3);
            } else {
                let _ = self.output.push_back(byte);
            }
        }
        let _ = self.output.push_back(END);
    }

    fn accepted(&mut self, code: u8) {
        self.packet(&[ACCEPTED, code]);
    }

    pub fn input_byte(&mut self, byte: u8) {
        if byte == START && !self.escaped {
            self.started = true;
            self.input_len = 0;
            return;
        }
        if !self.started {
            return;
        }
        if byte == ESCAPE && !self.escaped {
            self.escaped = true;
            return;
        }
        if byte == END && !self.escaped {
            let mut command = [0; 32];
            let length = self.input_len;
            command[..length].copy_from_slice(&self.input[..length]);
            self.started = false;
            self.input_len = 0;
            self.command(&command[..length]);
            return;
        }
        let value = if self.escaped {
            byte.wrapping_add(3)
        } else {
            byte
        };
        self.escaped = false;
        if self.input_len == self.input.len() {
            self.started = false;
            return;
        }
        self.input[self.input_len] = value;
        self.input_len += 1;
    }

    fn command(&mut self, command: &[u8]) {
        let Some((&code, args)) = command.split_first() else {
            return;
        };
        match code {
            0x01 | 0x18 | 0x1a | 0x1c | 0x1e | 0x20 | 0x22 | 0x24 | 0x26 => {
                if code == 0x20 && args.len() >= 2 {
                    self.host_address = Some(u16::from_be_bytes([args[0], args[1]]));
                }
                self.accepted(code);
            }
            0x0a if args.len() >= 2 => {
                transport::set_ack_mask(u16::from_be_bytes([args[0], args[1]]));
                self.accepted(code);
            }
            0x0d | 0x0e if !args.is_empty() => self.accepted(code),
            0x0b | 0x0c if !args.is_empty() => {
                if self.transmit.len as usize >= MAX_FRAME {
                    self.transmit = Frame::default();
                    self.packet(&[REJECTED]);
                    return;
                }
                self.transmit.bytes[self.transmit.len as usize] = args[0];
                self.transmit.len += 1;
                if code == 0x0c {
                    let frame = core::mem::take(&mut self.transmit);
                    if !transport::submit(frame) {
                        self.packet(&[REJECTED]);
                        return;
                    }
                }
                self.accepted(code);
            }
            // Report version 1: Linux skips persistent EEPROM configuration.
            0x15 => self.packet(&[0x15, 0x00, 0x01]),
            0x1f => {
                let address = self.host_address.unwrap_or(self.sniffed_address);
                self.packet(&[0x1f, (address >> 8) as u8, address as u8]);
            }
            _ => self.packet(&[REJECTED]),
        }
    }

    pub fn event(&mut self, event: Event) {
        match event {
            Event::Received(frame) => {
                for i in 0..frame.len as usize {
                    let code = if i == 0 { 0x05 } else { 0x06 };
                    let eom = if i + 1 == frame.len as usize { 0x80 } else { 0 };
                    self.packet(&[code | eom, frame.bytes[i]]);
                }
            }
            Event::Sent => self.packet(&[0x10]),
            Event::Nack => self.packet(&[0x12]),
            Event::LineError => self.packet(&[0x11]),
        }
    }
}
