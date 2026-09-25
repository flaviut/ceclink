//! Framing and the small Pulse-Eight command subset used by Linux pulse8-cec.
use crate::transport::{self, Event, Frame, MAX_FRAME};
use heapless::Deque;
use num_enum::{IntoPrimitive, TryFromPrimitive};

const START: u8 = 0xff;
const END: u8 = 0xfe;
const ESCAPE: u8 = 0xfd;
const EOM: u8 = 0x80;
#[derive(Clone, Copy, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
enum Code {
    Ping = 0x01,
    FrameStart = 0x05,
    FrameData = 0x06,
    Accepted = 0x08,
    Rejected = 0x09,
    SetAckMask = 0x0a,
    Transmit = 0x0b,
    TransmitEom = 0x0c,
    SetTransmitIdleTime = 0x0d,
    SetTransmitAckPolarity = 0x0e,
    Sent = 0x10,
    LineError = 0x11,
    Nack = 0x12,
    FirmwareVersion = 0x15,
    Config18 = 0x18,
    Config1a = 0x1a,
    Config1c = 0x1c,
    Config1e = 0x1e,
    GetPhysicalAddress = 0x1f,
    SetPhysicalAddress = 0x20,
    Config22 = 0x22,
    Config24 = 0x24,
    Config26 = 0x26,
    GetDdcDiagnostics = 0x40,
    EnterBootsel = 0x41,
}

pub struct Protocol {
    input: [u8; 32],
    input_len: usize,
    started: bool,
    escaped: bool,
    output: Deque<u8, 256>,
    transmit: Frame,
    sniffed_address: u16,
    host_address: Option<u16>,
    diagnostic_requested: bool,
    bootsel_requested: bool,
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
            diagnostic_requested: false,
            bootsel_requested: false,
        }
    }

    pub fn set_physical_address(&mut self, address: u16) {
        self.sniffed_address = address;
    }

    pub fn next_output(&mut self) -> Option<u8> {
        self.output.pop_front()
    }

    pub fn take_diagnostic_request(&mut self) -> bool {
        core::mem::take(&mut self.diagnostic_requested)
    }

    pub fn reply_diagnostics(&mut self, data: &[u8]) {
        if data.len() > 63 {
            return;
        }
        let mut payload = [0; 64];
        payload[0] = Code::GetDdcDiagnostics.into();
        payload[1..data.len() + 1].copy_from_slice(data);
        self.packet(&payload[..data.len() + 1]);
    }

    pub fn take_bootsel_request(&mut self) -> bool {
        core::mem::take(&mut self.bootsel_requested)
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

    fn accepted(&mut self, command: Code) {
        self.packet(&[Code::Accepted.into(), command.into()]);
    }

    fn rejected(&mut self) {
        self.packet(&[Code::Rejected.into()]);
    }

    fn word_reply(&mut self, code: Code, value: u16) {
        let [high, low] = value.to_be_bytes();
        self.packet(&[code.into(), high, low]);
    }

    fn word(args: &[u8]) -> Option<u16> {
        let bytes: [u8; 2] = args.get(..2)?.try_into().ok()?;
        Some(u16::from_be_bytes(bytes))
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
        let Some((code, args)) = command.split_first() else {
            return;
        };
        let Ok(code) = Code::try_from(*code) else {
            self.rejected();
            return;
        };
        match code {
            Code::Ping
            | Code::Config18
            | Code::Config1a
            | Code::Config1c
            | Code::Config1e
            | Code::SetPhysicalAddress
            | Code::Config22
            | Code::Config24
            | Code::Config26 => {
                if matches!(code, Code::SetPhysicalAddress) {
                    if let Some(address) = Self::word(args) {
                        self.host_address = Some(address);
                    }
                }
                self.accepted(code);
            }
            Code::SetAckMask => {
                if let Some(mask) = Self::word(args) {
                    transport::set_ack_mask(mask);
                    self.accepted(code);
                    return;
                }
                self.rejected();
            }
            Code::SetTransmitIdleTime | Code::SetTransmitAckPolarity if !args.is_empty() => {
                self.accepted(code)
            }
            Code::Transmit | Code::TransmitEom if !args.is_empty() => {
                if self.transmit.len as usize >= MAX_FRAME {
                    self.transmit = Frame::default();
                    self.rejected();
                    return;
                }
                self.transmit.bytes[self.transmit.len as usize] = args[0];
                self.transmit.len += 1;
                if matches!(code, Code::TransmitEom) {
                    let frame = core::mem::take(&mut self.transmit);
                    if !transport::submit(frame) {
                        self.rejected();
                        return;
                    }
                }
                self.accepted(code);
            }
            // Report version 1: Linux skips persistent EEPROM configuration.
            Code::FirmwareVersion => self.word_reply(Code::FirmwareVersion, 1),
            Code::GetPhysicalAddress => {
                let address = self.host_address.unwrap_or(self.sniffed_address);
                self.word_reply(Code::GetPhysicalAddress, address);
            }
            Code::GetDdcDiagnostics => self.diagnostic_requested = true,
            Code::EnterBootsel if args == b"RP25" => self.bootsel_requested = true,
            _ => self.rejected(),
        }
    }

    pub fn event(&mut self, event: Event) {
        match event {
            Event::Received(frame) => {
                for i in 0..frame.len as usize {
                    let code = if i == 0 {
                        Code::FrameStart
                    } else {
                        Code::FrameData
                    };
                    let eom = if i + 1 == frame.len as usize { EOM } else { 0 };
                    self.packet(&[u8::from(code) | eom, frame.bytes[i]]);
                }
            }
            Event::Sent => self.packet(&[Code::Sent.into()]),
            Event::Nack => self.packet(&[Code::Nack.into()]),
            Event::LineError => self.packet(&[Code::LineError.into()]),
        }
    }
}
