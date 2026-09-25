#[path = "../src/ddc_protocol.rs"]
mod ddc_protocol;

use ddc_protocol::{Decoder, GAP, START, STOP};

fn byte(value: u8, ack: bool) -> u32 {
    // The PIO shifts the address/data bits and ACK into the top of its ISR.
    let nine_bits = (u16::from(value) << 1) | u16::from(!ack);
    u32::from(nine_bits).reverse_bits()
}

fn read_cta_address(decoder: &mut Decoder) {
    decoder.push(START);
    decoder.push(byte(0xa0, true)); // EDID write address
    decoder.push(byte(128, true)); // CTA extension offset
    decoder.push(START); // repeated START
    decoder.push(byte(0xa1, true)); // EDID read address
    for (index, value) in [
        0x02, 0x03, 0x0a, 0x00, // CTA header, data-block end at 138
        0x65, 0x03, 0x0c, 0x00, 0x12, 0x34, // HDMI VSDB with address 1.2.3.4
    ]
    .into_iter()
    .enumerate()
    {
        decoder.push(byte(value, index != 9)); // final byte is NACKed
    }
    decoder.push(STOP);
}

#[test]
fn repeated_start_and_read_nack_preserve_physical_address() {
    let mut decoder = Decoder::new();
    read_cta_address(&mut decoder);
    assert_eq!(decoder.physical_address(), 0x1234);
}

#[test]
fn a_gap_or_new_extension_read_discards_old_bytes() {
    let mut decoder = Decoder::new();
    read_cta_address(&mut decoder);
    assert_eq!(decoder.stats().edid_bytes, 10);
    decoder.push(GAP);
    assert_eq!(decoder.physical_address(), 0xffff);
    assert_eq!(decoder.stats().edid_bytes, 10);

    read_cta_address(&mut decoder);
    assert_eq!(decoder.physical_address(), 0x1234);
    decoder.push(START);
    decoder.push(byte(0xa0, true));
    decoder.push(byte(128, true));
    assert_eq!(decoder.physical_address(), 0xffff);
}

#[test]
fn unacknowledged_address_is_ignored() {
    let mut decoder = Decoder::new();
    decoder.push(START);
    decoder.push(byte(0xa1, false));
    for value in [0x02, 0x03, 0x0a, 0x00, 0x65, 0x03, 0x0c, 0x00, 0x12, 0x34] {
        decoder.push(byte(value, true));
    }
    assert_eq!(decoder.physical_address(), 0xffff);
}
