use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use super::frame::VideoFrame;

/**
    Thread-safe bounded frame queue for producer-consumer pattern
*/
pub struct FrameQueue {
    inner: Mutex<QueueInner>,
    not_full: Condvar,
    not_empty: Condvar,
}

struct QueueInner {
    frames: VecDeque<VideoFrame>,
    capacity: usize,
    closed: bool,
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

    /**
        Push a frame to the queue, blocking if full.
        Returns false if the queue was closed.
    */
    pub fn push(&self, frame: VideoFrame) -> bool {
        let mut inner = self.inner.lock().unwrap();

        // Wait until there's space or queue is closed
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

    /**
        Try to push without blocking. Returns true if successful.
    */
    pub fn try_push(&self, frame: VideoFrame) -> bool {
        let mut inner = self.inner.lock().unwrap();

        if inner.closed || inner.frames.len() >= inner.capacity {
            return false;
        }

        inner.frames.push_back(frame);
        self.not_empty.notify_one();
        true
    }

    /**
        Pop a frame from the queue, blocking if empty.
        Returns None if the queue is closed and empty.
    */
    pub fn pop(&self) -> Option<VideoFrame> {
        let mut inner = self.inner.lock().unwrap();

        // Wait until there's a frame or queue is closed
        while inner.frames.is_empty() && !inner.closed {
            inner = self.not_empty.wait(inner).unwrap();
        }

        let frame = inner.frames.pop_front();
        if frame.is_some() {
            self.not_full.notify_one();
        }
        frame
    }

    /**
        Try to pop without blocking.
    */
    pub fn try_pop(&self) -> Option<VideoFrame> {
        let mut inner = self.inner.lock().unwrap();
        let frame = inner.frames.pop_front();
        if frame.is_some() {
            self.not_full.notify_one();
        }
        frame
    }

    /**
        Pop a frame with timeout. Returns None if timeout or closed.
    */
    pub fn pop_timeout(&self, timeout: Duration) -> Option<VideoFrame> {
        let mut inner = self.inner.lock().unwrap();

        if inner.frames.is_empty() && !inner.closed {
            let result = self.not_empty.wait_timeout(inner, timeout).unwrap();
            inner = result.0;
        }

        let frame = inner.frames.pop_front();
        if frame.is_some() {
            self.not_full.notify_one();
        }
        frame
    }

    /**
        Peek at the front frame without removing it.
    */
    pub fn peek(&self) -> Option<VideoFrame> {
        let inner = self.inner.lock().unwrap();
        inner.frames.front().cloned()
    }

    /**
        Get the number of frames currently in the queue.
    */
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().frames.len()
    }

    /**
        Check if the queue is empty.
    */
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().frames.is_empty()
    }

    /**
        Close the queue, waking all waiters.
    */
    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        self.not_full.notify_all();
        self.not_empty.notify_all();
    }

    /**
        Check if the queue is closed.
    */
    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    /**
        Clear all frames from the queue.
    */
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.frames.clear();
        self.not_full.notify_all();
    }

    /**
        Reopen a closed queue for reuse (e.g., after seeking).
        Clears any remaining frames and resets the closed flag.
    */
    pub fn reopen(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.frames.clear();
        inner.closed = false;
        self.not_full.notify_all();
    }
}
