//! Framing and the small Pulse-Eight command subset used by Linux pulse8-cec.
use crate::transport::{self, Event, Frame, MAX_FRAME};
use binary_serde::{BinarySerde, Endianness};
use heapless::Deque;

const START: u8 = 0xff;
const END: u8 = 0xfe;
const ESCAPE: u8 = 0xfd;
const EOM: u8 = 0x80;
#[derive(Clone, Copy, BinarySerde)]
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
}

#[derive(BinarySerde)]
struct Ack {
    code: Code,
    command: Code,
}

#[derive(BinarySerde)]
struct ValueReply {
    code: Code,
    value: u16,
}

#[derive(BinarySerde)]
struct ReceivedByte {
    code: u8,
    value: u8,
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

    fn send<T: BinarySerde>(&mut self, value: &T) {
        let bytes = value.binary_serialize_to_array(Endianness::Big);
        self.packet(bytes.as_ref());
    }

    fn accepted(&mut self, command: Code) {
        self.send(&Ack {
            code: Code::Accepted,
            command,
        });
    }

    fn rejected(&mut self) {
        self.send(&Code::Rejected);
    }

    fn word(args: &[u8]) -> Option<u16> {
        u16::binary_deserialize(args.get(..u16::SERIALIZED_SIZE)?, Endianness::Big).ok()
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
        let Ok(code) = Code::binary_deserialize(core::slice::from_ref(code), Endianness::Big)
        else {
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
            Code::FirmwareVersion => self.send(&ValueReply {
                code: Code::FirmwareVersion,
                value: 1,
            }),
            Code::GetPhysicalAddress => {
                let address = self.host_address.unwrap_or(self.sniffed_address);
                self.send(&ValueReply {
                    code: Code::GetPhysicalAddress,
                    value: address,
                });
            }
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
                    self.send(&ReceivedByte {
                        code: (code as u8) | eom,
                        value: frame.bytes[i],
                    });
                }
            }
            Event::Sent => self.send(&Code::Sent),
            Event::Nack => self.send(&Code::Nack),
            Event::LineError => self.send(&Code::LineError),
        }
    }
}
