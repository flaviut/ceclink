//! Raspberry Pi's USB reset interface, used by fwupd's rp-pico plugin.
use usb_device::{
    bus::{InterfaceNumber, UsbBus, UsbBusAllocator},
    class_prelude::{ControlOut, DescriptorWriter, UsbClass},
    control::{Recipient, RequestType},
    Result,
};

pub struct UsbReset {
    interface: InterfaceNumber,
    bootsel_requested: bool,
}

impl UsbReset {
    pub fn new<B: UsbBus>(bus: &UsbBusAllocator<B>) -> Self {
        Self {
            interface: bus.interface(),
            bootsel_requested: false,
        }
    }

    pub fn take_bootsel_request(&mut self) -> bool {
        core::mem::take(&mut self.bootsel_requested)
    }
}

impl<B: UsbBus> UsbClass<B> for UsbReset {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        // The rp-pico plugin locates class ff, subclass 00, protocol 01.
        writer.interface(self.interface, 0xff, 0x00, 0x01)
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let request = xfer.request();
        // Raspberry Pi's class request 1 enters BOOTSEL.
        if request.request_type != RequestType::Class
            || request.recipient != Recipient::Interface
            || request.index != u8::from(self.interface) as u16
            || request.request != 0x01
        {
            return;
        }
        // Value 0 keeps both the mass-storage and picoboot interfaces enabled.
        if request.value != 0 || request.length != 0 {
            let _ = xfer.reject();
            return;
        }
        if xfer.accept().is_ok() {
            self.bootsel_requested = true;
        }
    }
}
