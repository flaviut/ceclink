mod transport {
    use std::sync::atomic::{AtomicU16, Ordering};

    pub const MAX_FRAME: usize = 16;

    #[derive(Clone, Copy, Default)]
    pub struct Frame {
        pub len: u8,
        pub bytes: [u8; MAX_FRAME],
    }

    #[allow(dead_code)]
    pub enum Event {
        Received(Frame),
        Sent,
        Nack,
        LineError,
    }

    static ACK_MASK: AtomicU16 = AtomicU16::new(0);

    pub fn set_ack_mask(mask: u16) {
        ACK_MASK.store(mask, Ordering::Relaxed);
    }

    pub fn ack_mask() -> u16 {
        ACK_MASK.load(Ordering::Relaxed)
    }

    pub fn submit(_frame: Frame) -> bool {
        true
    }
}

#[path = "../src/pulse8.rs"]
mod pulse8;

fn output(protocol: &mut pulse8::Protocol) -> Vec<u8> {
    std::iter::from_fn(|| protocol.next_output()).collect()
}

fn input(protocol: &mut pulse8::Protocol, bytes: &[u8]) {
    for &byte in bytes {
        protocol.input_byte(byte);
    }
}

#[test]
fn commands_preserve_pulse_eight_wire_format() {
    let mut protocol = pulse8::Protocol::new();
    protocol.set_physical_address(0x1234);

    input(&mut protocol, &[0xff, 0x15, 0xfe]);
    assert_eq!(output(&mut protocol), [0xff, 0x15, 0x00, 0x01, 0xfe]);

    input(&mut protocol, &[0xff, 0x1f, 0xfe]);
    assert_eq!(output(&mut protocol), [0xff, 0x1f, 0x12, 0x34, 0xfe]);

    input(&mut protocol, &[0xff, 0x20, 0xfd, 0xfc, 0xfd, 0xfb, 0xfe]);
    assert_eq!(output(&mut protocol), [0xff, 0x08, 0x20, 0xfe]);
    input(&mut protocol, &[0xff, 0x1f, 0xfe]);
    assert_eq!(
        output(&mut protocol),
        [0xff, 0x1f, 0xfd, 0xfc, 0xfd, 0xfb, 0xfe]
    );

    input(&mut protocol, &[0xff, 0x0a, 0x12, 0x34, 0xfe]);
    assert_eq!(transport::ack_mask(), 0x1234);
    assert_eq!(output(&mut protocol), [0xff, 0x08, 0x0a, 0xfe]);

    input(&mut protocol, &[0xff, 0x0a, 0x12, 0xfe]);
    assert_eq!(output(&mut protocol), [0xff, 0x09, 0xfe]);
    assert_eq!(transport::ack_mask(), 0x1234);
}

#[test]
fn received_frame_keeps_eom_and_escape_bytes() {
    let mut protocol = pulse8::Protocol::new();
    let mut frame = transport::Frame::default();
    frame.len = 2;
    frame.bytes[..2].copy_from_slice(&[0xff, 0xfe]);
    protocol.event(transport::Event::Received(frame));
    assert_eq!(
        output(&mut protocol),
        [0xff, 0x05, 0xfd, 0xfc, 0xfe, 0xff, 0x86, 0xfd, 0xfb, 0xfe]
    );
}
