use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

/**
    A decoded packet ready for the decode threads.
    Contains raw packet data and timing information.
*/
pub struct Packet {
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub duration: i64,
    /// Flags from the original packet (e.g., keyframe)
    pub flags: i32,
}

impl Packet {
    pub fn new(data: Vec<u8>, pts: i64, dts: i64, duration: i64, flags: i32) -> Self {
        Self {
            data,
            pts,
            dts,
            duration,
            flags,
        }
    }
}

struct PacketQueueInner {
    packets: VecDeque<Packet>,
    capacity: usize,
    closed: bool,
}

/**
    Thread-safe bounded queue for packets.
    Used to route demuxed packets to decode threads.
*/
pub struct PacketQueue {
    inner: Mutex<PacketQueueInner>,
    not_full: Condvar,
    not_empty: Condvar,
}

impl PacketQueue {
    /**
        Create a new packet queue with the given capacity
    */
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

    /**
        Push a packet to the queue, blocking if full.
        Returns false if the queue was closed.
    */
    pub fn push(&self, packet: Packet) -> bool {
        let mut inner = self.inner.lock().unwrap();

        // Wait until there's space or queue is closed
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

    /**
        Pop a packet from the queue, blocking if empty.
        Returns None if the queue is closed and empty.
    */
    pub fn pop(&self) -> Option<Packet> {
        let mut inner = self.inner.lock().unwrap();

        // Wait until there's a packet or queue is closed
        while inner.packets.is_empty() && !inner.closed {
            inner = self.not_empty.wait(inner).unwrap();
        }

        let packet = inner.packets.pop_front();

        if packet.is_some() {
            self.not_full.notify_one();
        }

        packet
    }

    /**
        Close the queue, signaling EOF.
        Wakes all waiting threads.
    */
    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        self.not_full.notify_all();
        self.not_empty.notify_all();
    }

    /**
        Check if the queue is closed
    */
    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    /**
        Get the number of packets in the queue
    */
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().packets.len()
    }

    /**
        Clear all packets from the queue.
        Wakes any threads waiting to push.
    */
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.packets.clear();
        self.not_full.notify_all();
    }

    /**
        Reopen a closed queue for reuse (e.g., after seeking).
        Clears any remaining packets and resets the closed flag.
    */
    pub fn reopen(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.packets.clear();
        inner.closed = false;
        self.not_full.notify_all();
    }
}
