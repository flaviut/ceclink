//! Periodic, machine-readable diagnostics on a second USB CDC ACM interface.

use core::fmt::Write as _;

use heapless::String;
use usb_device::bus::UsbBus;
use usbd_serial::SerialPort;

use crate::ddc::{Ddc, DIAGNOSTIC_LEN};

const PERIOD_US: u64 = 1_000_000;
const LINE_CAPACITY: usize = 768;

pub struct DebugOutput {
    line: String<LINE_CAPACITY>,
    sent: usize,
    last_at: u64,
    connected: bool,
}

impl DebugOutput {
    pub fn new() -> Self {
        Self {
            line: String::new(),
            sent: 0,
            last_at: 0,
            connected: false,
        }
    }

    pub fn poll<B: UsbBus>(&mut self, port: &mut SerialPort<'_, B>, now: u64, ddc: &Ddc) {
        if !port.dtr() {
            self.connected = false;
            self.line.clear();
            self.sent = 0;
            return;
        }

        let just_connected = !self.connected;
        if just_connected {
            self.connected = true;
        }

        if self.line.is_empty() && (just_connected || now - self.last_at >= PERIOD_US) {
            self.last_at = now;
            self.line =
                format_snapshot(now / 1_000, ddc.physical_address(), &ddc.diagnostic_bytes());
            self.sent = 0;
        }

        if self.sent < self.line.len() {
            // One USB packet per loop keeps the CEC and DDC work responsive.
            let end = (self.sent + 64).min(self.line.len());
            if let Ok(written) = port.write(&self.line.as_bytes()[self.sent..end]) {
                self.sent += written;
            }
            if self.sent == self.line.len() {
                self.line.clear();
                self.sent = 0;
            }
        }
    }
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn format_snapshot(
    uptime_ms: u64,
    physical_address: u16,
    data: &[u8; DIAGNOSTIC_LEN],
) -> String<LINE_CAPACITY> {
    let mut line = String::new();
    let ring_used = u16::from_be_bytes([data[2], data[3]]);
    let ring_peak = u16::from_be_bytes([data[4], data[5]]);
    write!(
        line,
        "version={}\tuptime_ms={}\tsda={}\tscl={}\tring_used={}\tring_peak={}\tstarts={}\tstops={}\twords={}\tfifo_stalls={}\tring_overflows={}\ttrace_len={}\ttrace_0={}\ttrace_1={}\tedid_write_addresses={}\tedid_read_addresses={}\tedid_bytes={}\tinvalid_words={}\tunacknowledged_addresses={}\tpio_pc={}\trx_empty={}\trx_full={}\tphysical_address={}\n",
        data[0],
        uptime_ms,
        data[1] & 1,
        (data[1] >> 1) & 1,
        ring_used,
        ring_peak,
        word(data, 6),
        word(data, 10),
        word(data, 14),
        word(data, 18),
        word(data, 22),
        data[26],
        word(data, 27),
        word(data, 31),
        word(data, 35),
        word(data, 39),
        word(data, 43),
        word(data, 47),
        word(data, 51),
        data[55],
        data[56] & 1,
        (data[56] >> 1) & 1,
        physical_address,
    )
    .expect("debug line fits in its buffer");
    line
}
