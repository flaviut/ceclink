//! Foreground decoder for the event words produced by the DDC PIO program.
//!
//! START and STOP are 2 and 3. A data word contains nine bits in PIO's
//! right-shifted input format: eight data bits followed by the ACK bit.

pub const GAP: u32 = 1;
pub const START: u32 = 2;
pub const STOP: u32 = 3;

#[derive(Clone, Copy, Default)]
pub struct DecoderStats {
    pub edid_write_addresses: u32,
    pub edid_read_addresses: u32,
    pub edid_bytes: u32,
    pub invalid_words: u32,
    pub unacknowledged_addresses: u32,
}

pub struct Decoder {
    active: bool,
    byte_index: u16,
    target: u8,
    reading: bool,
    offset: u8,
    segment: u8,
    edid: [u8; 256],
    seen: [u8; 32],
    physical_address: u16,
    stats: DecoderStats,
}

impl Decoder {
    pub const fn new() -> Self {
        Self {
            active: false,
            byte_index: 0,
            target: 0,
            reading: false,
            offset: 0,
            segment: 0,
            edid: [0; 256],
            seen: [0; 32],
            physical_address: 0xffff,
            stats: DecoderStats {
                edid_write_addresses: 0,
                edid_read_addresses: 0,
                edid_bytes: 0,
                invalid_words: 0,
                unacknowledged_addresses: 0,
            },
        }
    }

    pub fn physical_address(&self) -> u16 {
        self.physical_address
    }

    pub fn stats(&self) -> DecoderStats {
        self.stats
    }

    pub fn push(&mut self, word: u32) {
        match word {
            GAP => {
                let stats = self.stats;
                *self = Self::new();
                self.stats = stats;
            }
            START => {
                self.active = true;
                self.byte_index = 0;
            }
            STOP => self.active = false,
            _ if self.active && word & 0x7f_ffff == 0 => {
                let nine_bits = (word.reverse_bits() & 0x1ff) as u16;
                self.byte((nine_bits >> 1) as u8, nine_bits & 1 == 0);
            }
            _ if self.active => {
                self.stats.invalid_words = self.stats.invalid_words.wrapping_add(1);
                self.active = false;
            }
            _ => {}
        }
    }

    fn byte(&mut self, value: u8, ack: bool) {
        if self.byte_index == 0 {
            self.target = value >> 1;
            self.reading = value & 1 != 0;
            self.active = ack;
            if !ack {
                self.stats.unacknowledged_addresses =
                    self.stats.unacknowledged_addresses.wrapping_add(1);
            } else if self.target == 0x50 {
                let counter = if self.reading {
                    &mut self.stats.edid_read_addresses
                } else {
                    &mut self.stats.edid_write_addresses
                };
                *counter = counter.wrapping_add(1);
            }
        } else if self.active && self.target == 0x50 {
            if self.reading {
                let index = u16::from(self.segment) * 256 + u16::from(self.offset);
                if index < 256 {
                    self.record(index as usize, value);
                }
                self.offset = self.offset.wrapping_add(1);
            } else if self.byte_index == 1 && ack {
                self.offset = value;
                if (self.segment == 0 && (value == 0 || value == 128))
                    || (self.segment == 1 && value == 0)
                {
                    self.clear_edid();
                }
            }
        } else if self.active && self.target == 0x30 && !self.reading && self.byte_index == 1 && ack
        {
            self.segment = value;
        }
        self.byte_index = self.byte_index.saturating_add(1);
    }

    fn clear_edid(&mut self) {
        self.seen = [0; 32];
        self.physical_address = 0xffff;
    }

    fn record(&mut self, index: usize, value: u8) {
        self.edid[index] = value;
        self.seen[index / 8] |= 1 << (index % 8);
        self.stats.edid_bytes = self.stats.edid_bytes.wrapping_add(1);
        self.find_physical_address();
    }

    fn captured(&self, index: usize) -> bool {
        self.seen[index / 8] & (1 << (index % 8)) != 0
    }

    fn find_physical_address(&mut self) {
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
                self.physical_address =
                    u16::from_be_bytes([self.edid[index + 4], self.edid[index + 5]]);
                return;
            }
            index = next;
        }
    }
}
