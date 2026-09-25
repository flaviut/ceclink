#![no_std]
#![no_main]

use hal::{
    gpio::Pins,
    multicore::{Multicore, Stack},
    sio::Sio,
};
use panic_halt as _;
use rp235x_hal as hal;
use usb_device::{class_prelude::UsbBusAllocator, prelude::*};
use usbd_serial::SerialPort;

mod cec;
mod ddc;
mod pulse8;
mod transport;

#[unsafe(link_section = ".start_block")]
#[used]
static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();
static CORE1_STACK: Stack<4096> = Stack::new();

#[hal::entry]
fn main() -> ! {
    let mut pac = hal::pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        12_000_000,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .unwrap();
    let timer = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);

    let mut sio = Sio::new(pac.SIO);
    let pins = Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );
    let cec_pin = pins.gpio4.into_floating_input(); // XIAO D9
    let sda = pins.gpio6.into_floating_input(); // XIAO D4
    let scl = pins.gpio7.into_floating_input(); // XIAO D5
    ddc::init(sda, scl);
    unsafe {
        hal::arch::interrupt_enable();
    }

    let mut multicore = Multicore::new(&mut pac.PSM, &mut pac.PPB, &mut sio.fifo);
    multicore.cores()[1]
        .spawn(CORE1_STACK.take().unwrap(), move || {
            cec::run(cec_pin, timer)
        })
        .unwrap();

    let usb_bus = UsbBusAllocator::new(hal::usb::UsbBus::new(
        pac.USB,
        pac.USB_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));
    let mut serial = SerialPort::new(&usb_bus);
    let mut device = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x2548, 0x1002))
        .strings(&[StringDescriptors::default()
            .manufacturer("Pulse-Eight")
            .product("CEC Adapter")
            .serial_number("CEC4K-RP2350")])
        .unwrap()
        .device_class(2)
        .build();

    let mut protocol = pulse8::Protocol::new();
    let mut pending_output = None;
    loop {
        let _ = device.poll(&mut [&mut serial]);
        let mut bytes = [0u8; 64];
        if let Ok(count) = serial.read(&mut bytes) {
            for &byte in &bytes[..count] {
                protocol.input_byte(byte);
            }
        }
        while let Some(event) = transport::pop_event() {
            protocol.event(event);
        }
        protocol.set_physical_address(ddc::physical_address());
        if pending_output.is_none() {
            pending_output = protocol.next_output();
        }
        if let Some(byte) = pending_output {
            if serial.write(&[byte]).is_ok() {
                pending_output = None;
            }
        }
    }
}
