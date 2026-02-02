use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

use super::frame::VideoFrame;

struct QueueInner {
    frames: VecDeque<VideoFrame>,
    capacity: usize,
    closed: bool,
}

pub struct FrameQueue {
    inner: Mutex<QueueInner>,
    not_full: Condvar,
    not_empty: Condvar,
}

impl FrameQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(QueueInner {
                frames: VecDeque::with_capacity(capacity),
                capacity,
                closed: false,
            }),
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
        }
    }

    pub fn push(&self, frame: VideoFrame) -> bool {
        let mut inner = self.inner.lock().unwrap();

        while inner.frames.len() >= inner.capacity && !inner.closed {
            inner = self.not_full.wait(inner).unwrap();
        }

        if inner.closed {
            return false;
        }

        inner.frames.push_back(frame);
        self.not_empty.notify_one();
        true
    }

    pub fn try_pop(&self) -> Option<VideoFrame> {
        let mut inner = self.inner.lock().unwrap();
        let frame = inner.frames.pop_front();
        if frame.is_some() {
            self.not_full.notify_one();
        }
        frame
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().frames.is_empty()
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

    pub fn reopen(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.frames.clear();
        inner.closed = false;
        self.not_full.notify_all();
    }
}
