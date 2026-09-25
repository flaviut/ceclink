use core::cell::RefCell;
use critical_section::Mutex;
use heapless::Deque;

pub const MAX_FRAME: usize = 16;

#[derive(Clone, Copy, Default)]
pub struct Frame {
    pub len: u8,
    pub bytes: [u8; MAX_FRAME],
}

#[derive(Clone, Copy)]
pub enum Event {
    Received(Frame),
    Sent,
    Nack,
    LineError,
}

struct Shared {
    pending_tx: Option<Frame>,
    events: Deque<Event, 8>,
    ack_mask: u16,
}

static SHARED: Mutex<RefCell<Shared>> = Mutex::new(RefCell::new(Shared {
    pending_tx: None,
    events: Deque::new(),
    ack_mask: 0,
}));

pub fn submit(frame: Frame) -> bool {
    critical_section::with(|cs| {
        let mut shared = SHARED.borrow_ref_mut(cs);
        if shared.pending_tx.is_some() || frame.len == 0 || frame.len as usize > MAX_FRAME {
            return false;
        }
        shared.pending_tx = Some(frame);
        true
    })
}

pub fn take_pending() -> Option<Frame> {
    critical_section::with(|cs| SHARED.borrow_ref_mut(cs).pending_tx.take())
}

pub fn push_event(event: Event) {
    critical_section::with(|cs| {
        let _ = SHARED.borrow_ref_mut(cs).events.push_back(event);
    });
}

pub fn pop_event() -> Option<Event> {
    critical_section::with(|cs| SHARED.borrow_ref_mut(cs).events.pop_front())
}

pub fn set_ack_mask(mask: u16) {
    critical_section::with(|cs| SHARED.borrow_ref_mut(cs).ack_mask = mask);
}

pub fn should_ack(address: u8) -> bool {
    critical_section::with(|cs| SHARED.borrow_ref(cs).ack_mask & (1 << address) != 0)
}
