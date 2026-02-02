use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

use ffmpeg_types::Packet;

struct PacketQueueInner {
    packets: VecDeque<Packet>,
    capacity: usize,
    closed: bool,
}

pub struct PacketQueue {
    inner: Mutex<PacketQueueInner>,
    not_full: Condvar,
    not_empty: Condvar,
}

impl PacketQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(PacketQueueInner {
                packets: VecDeque::with_capacity(capacity),
                capacity,
                closed: false,
            }),
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
        }
    }

    pub fn push(&self, packet: Packet) -> bool {
        let mut inner = self.inner.lock().unwrap();

        while inner.packets.len() >= inner.capacity && !inner.closed {
            inner = self.not_full.wait(inner).unwrap();
        }

        if inner.closed {
            return false;
        }

        inner.packets.push_back(packet);
        self.not_empty.notify_one();
        true
    }

    pub fn pop(&self) -> Option<Packet> {
        let mut inner = self.inner.lock().unwrap();

        while inner.packets.is_empty() && !inner.closed {
            inner = self.not_empty.wait(inner).unwrap();
        }

        let packet = inner.packets.pop_front();

        if packet.is_some() {
            self.not_full.notify_one();
        }

        packet
    }

    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        self.not_full.notify_all();
        self.not_empty.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().packets.len()
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.packets.clear();
        self.not_full.notify_all();
    }

    pub fn reopen(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.packets.clear();
        inner.closed = false;
        self.not_full.notify_all();
    }
}
