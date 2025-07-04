use thiserror::Error;

#[derive(Error, Debug)]
pub enum BipBufferError {
    #[error("Buffer is full - cannot allocate {requested} bytes")]
    BufferFull { requested: usize },

    #[error("Invalid commit size: {size} (max: {max})")]
    InvalidCommitSize { size: usize, max: usize },

    #[error("No data available to read")]
    NoData,

    #[error("Buffer overflow - size {size} exceeds capacity {capacity}")]
    BufferOverflow { size: usize, capacity: usize },
}

/// Bipbuffer allows efficient circular buffering without data copying.
#[derive(Debug)]
pub struct BipBuffer {
    buffer: Vec<u8>,
    capacity: usize,

    // Region A (primary data region)
    a_start: usize,
    a_end: usize,

    // Region B (secondary data region, used when wrapping)
    b_end: usize,

    // Reserve region (for pending writes)
    reserve_start: usize,
    reserve_size: usize,
}

impl BipBuffer {
    /// Create a new bipbuffer with the specified capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0; capacity],
            capacity,
            a_start: 0,
            a_end: 0,
            b_end: 0,
            reserve_start: 0,
            reserve_size: 0,
        }
    }

    /// Get the total capacity of the buffer
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the amount of data currently stored
    pub fn len(&self) -> usize {
        let a_size = self.a_end.saturating_sub(self.a_start);
        let b_size = self.b_end;
        a_size + b_size
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the amount of free space available
    pub fn free_space(&self) -> usize {
        self.capacity
            .saturating_sub(self.len())
            .saturating_sub(self.reserve_size)
    }

    /// Reserve space for writing data
    ///
    /// Returns a mutable slice where you can write data.
    /// Must call `commit()` afterwards to make the data available for reading.
    pub fn reserve(&mut self, size: usize) -> Result<&mut [u8], BipBufferError> {
        if size == 0 {
            return Ok(&mut []);
        }

        if size > self.capacity {
            return Err(BipBufferError::BufferOverflow {
                size,
                capacity: self.capacity,
            });
        }

        // Clear any existing reservation
        self.reserve_size = 0;

        let space_after_a = self.capacity.saturating_sub(self.a_end);
        let space_before_a = self.a_start.saturating_sub(self.b_end);

        // Try to allocate after region A first
        if space_after_a >= size {
            self.reserve_start = self.a_end;
            self.reserve_size = size;
            let end = (self.reserve_start + size).min(self.capacity);
            return Ok(&mut self.buffer[self.reserve_start..end]);
        }

        // Try to allocate before region A (region B area)
        if space_before_a >= size {
            self.reserve_start = self.b_end;
            self.reserve_size = size;
            return Ok(&mut self.buffer[self.b_end..self.b_end + size]);
        }

        Err(BipBufferError::BufferFull { requested: size })
    }

    /// Commit previously reserved data
    ///
    /// `size` must be <= the size that was reserved.
    /// After committing, the data becomes available for reading.
    pub fn commit(&mut self, size: usize) -> Result<(), BipBufferError> {
        if size > self.reserve_size {
            return Err(BipBufferError::InvalidCommitSize {
                size,
                max: self.reserve_size,
            });
        }

        if size == 0 {
            self.reserve_size = 0;
            return Ok(());
        }

        // Committing to region A
        if self.reserve_start == self.a_end {
            self.a_end = (self.a_end + size).min(self.capacity);
        }
        // Committing to region B
        else if self.reserve_start == 0 {
            self.b_end = (self.b_end + size).min(self.a_start.min(self.capacity));
        }

        self.reserve_size = 0;
        Ok(())
    }

    /// Write data directly to the buffer
    ///
    /// This is a convenience method that combines reserve() and commit().
    pub fn write(&mut self, data: &[u8]) -> Result<(), BipBufferError> {
        if data.is_empty() {
            return Ok(());
        }

        let reserved = self.reserve(data.len())?;
        let copy_len = data.len().min(reserved.len());
        reserved[..copy_len].copy_from_slice(&data[..copy_len]);
        self.commit(copy_len)
    }

    /// Get a slice of readable data
    ///
    /// Returns the first contiguous block of data.
    /// May need to call multiple times to read all data due to wrapping.
    pub fn read(&self) -> &[u8] {
        if self.a_start < self.a_end {
            // Return data from region A
            &self.buffer[self.a_start..self.a_end]
        } else if self.b_end > 0 {
            // Return data from region B
            &self.buffer[0..self.b_end]
        } else {
            &[]
        }
    }

    /// Consume (remove) data from the buffer
    ///
    /// Marks `size` bytes as read and removes them from the buffer.
    /// Uses safe Rust operations only.
    pub fn consume(&mut self, size: usize) -> Result<(), BipBufferError> {
        if size == 0 {
            return Ok(());
        }

        let available = self.read().len();
        if size > available {
            return Err(BipBufferError::InvalidCommitSize {
                size,
                max: available,
            });
        }

        if self.a_start < self.a_end {
            // Consuming from region A
            self.a_start = (self.a_start + size).min(self.a_end);

            // If we've consumed all of region A, move region B to A
            if self.a_start == self.a_end && self.b_end > 0 {
                self.a_start = 0;
                self.a_end = self.b_end;
                self.b_end = 0;
            }
        } else if self.b_end > 0 {
            // Consuming from region B - use safe copy_within
            if size >= self.b_end {
                self.b_end = 0;
            } else {
                let remaining = self.b_end - size;
                // Safe alternative to unsafe ptr::copy
                self.buffer.copy_within(size..self.b_end, 0);
                self.b_end = remaining;
            }
        }

        Ok(())
    }

    /// Read and consume data in one operation
    pub fn read_and_consume(&mut self, size: usize) -> Result<Vec<u8>, BipBufferError> {
        let mut result = Vec::with_capacity(size);
        let mut remaining = size;

        while remaining > 0 && !self.is_empty() {
            let available = self.read();
            if available.is_empty() {
                break;
            }

            let to_read = remaining.min(available.len());
            result.extend_from_slice(&available[..to_read]);
            self.consume(to_read)?;
            remaining -= to_read;
        }

        if result.len() < size {
            return Err(BipBufferError::NoData);
        }

        Ok(result)
    }

    /// Copy all available data into a new Vec
    pub fn read_all(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(self.len());

        // Read from region A
        if self.a_start < self.a_end {
            result.extend_from_slice(&self.buffer[self.a_start..self.a_end]);
        }

        // Read from region B
        if self.b_end > 0 {
            result.extend_from_slice(&self.buffer[0..self.b_end]);
        }

        result
    }

    /// Peek at data without consuming it
    pub fn peek(&self, size: usize) -> Vec<u8> {
        let mut result = Vec::with_capacity(size.min(self.len()));
        let mut remaining = size;

        // Peek from region A
        if self.a_start < self.a_end && remaining > 0 {
            let a_data = &self.buffer[self.a_start..self.a_end];
            let to_take = remaining.min(a_data.len());
            result.extend_from_slice(&a_data[..to_take]);
            remaining -= to_take;
        }

        // Peek from region B if needed
        if self.b_end > 0 && remaining > 0 {
            let b_data = &self.buffer[0..self.b_end];
            let to_take = remaining.min(b_data.len());
            result.extend_from_slice(&b_data[..to_take]);
        }

        result
    }

    /// Clear all data from the buffer
    pub fn clear(&mut self) {
        self.a_start = 0;
        self.a_end = 0;
        self.b_end = 0;
        self.reserve_size = 0;
    }

    /// Compact the buffer to make more contiguous space available
    /// This is a safe operation that reorganizes data for better space utilization
    pub fn compact(&mut self) {
        if self.b_end == 0 {
            return; // Nothing to compact
        }

        // Move region B data to after region A
        let a_size = self.a_end - self.a_start;
        if a_size > 0 {
            // First, move region A to the beginning if it's not already there
            if self.a_start > 0 {
                self.buffer.copy_within(self.a_start..self.a_end, 0);
                self.a_start = 0;
                self.a_end = a_size;
            }

            // Then append region B after region A
            self.buffer.copy_within(0..self.b_end, self.a_end);
            self.a_end += self.b_end;
        } else {
            // If region A is empty, just move region B to the beginning
            self.buffer.copy_within(0..self.b_end, 0);
            self.a_start = 0;
            self.a_end = self.b_end;
        }

        self.b_end = 0;
    }

    /// Find the first occurrence of a byte pattern
    pub fn find(&self, pattern: &[u8]) -> Option<usize> {
        if pattern.is_empty() {
            return Some(0);
        }

        let all_data = self.read_all();
        all_data
            .windows(pattern.len())
            .position(|window| window == pattern)
    }

    /// Create an iterator over all bytes in the buffer
    pub fn iter(&self) -> impl Iterator<Item = u8> + '_ {
        let a_iter = self.buffer[self.a_start..self.a_end].iter().copied();
        let b_iter = self.buffer[0..self.b_end].iter().copied();
        a_iter.chain(b_iter)
    }

    /// Get debug information about the buffer state
    pub fn debug_info(&self) -> String {
        format!(
            "BipBuffer {{ capacity: {}, len: {}, free: {}, a: {}..{}, b: 0..{}, reserve: {}+{} }}",
            self.capacity,
            self.len(),
            self.free_space(),
            self.a_start,
            self.a_end,
            self.b_end,
            self.reserve_start,
            self.reserve_size
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let mut buf = BipBuffer::new(100);
        assert_eq!(buf.capacity(), 100);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());

        // Write some data
        buf.write(b"hello").unwrap();
        assert_eq!(buf.len(), 5);
        assert!(!buf.is_empty());

        // Read it back
        let data = buf.read();
        assert_eq!(data, b"hello");

        // Consume it
        buf.consume(5).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_reserve_commit() {
        let mut buf = BipBuffer::new(100);

        // Reserve space
        let space = buf.reserve(10).unwrap();
        assert_eq!(space.len(), 10);

        // Write to reserved space
        space.copy_from_slice(b"1234567890");

        // Data not visible until committed
        assert!(buf.read().is_empty());

        // Commit it
        buf.commit(10).unwrap();

        // Now it's visible
        assert_eq!(buf.read(), b"1234567890");
    }

    #[test]
    fn test_wrapping() {
        let mut buf = BipBuffer::new(10);

        // Fill most of the buffer
        buf.write(b"12345678").unwrap();

        // Read and consume part of it
        buf.consume(3).unwrap(); // Remove "123", leaving "45678"

        // Now write more data - this should wrap to region B
        // We have 3 bytes free at the start, so write 3 bytes
        buf.write(b"ABC").unwrap();

        assert_eq!(buf.read(), b"45678");
        buf.consume(5).unwrap();

        assert_eq!(buf.read(), b"ABC");
    }

    #[test]
    fn test_compact() {
        let mut buf = BipBuffer::new(10);

        // Create a fragmented state
        buf.write(b"12345").unwrap();
        buf.consume(2).unwrap(); // a_start=2, a_end=5
        buf.write(b"ABC").unwrap(); // This goes to region B

        println!("Before compact: {}", buf.debug_info());
        buf.compact();
        println!("After compact: {}", buf.debug_info());

        // After compacting, all data should be in region A
        assert_eq!(buf.b_end, 0);
        assert_eq!(buf.read_all(), b"345ABC");
    }

    #[test]
    fn test_peek() {
        let mut buf = BipBuffer::new(10);
        buf.write(b"hello wo").unwrap();

        // Peek should not modify the buffer
        let peeked = buf.peek(5);
        assert_eq!(peeked, b"hello");
        assert_eq!(buf.len(), 8); // Length unchanged

        // Reading should still work
        assert_eq!(buf.read(), b"hello wo");
    }

    #[test]
    fn test_space_limitations() {
        let mut buf = BipBuffer::new(10);

        buf.write(b"12345678").unwrap(); // 8 bytes
        buf.consume(3).unwrap(); // Free 3 bytes at start

        // Try to write 5 bytes - should fail (only 3+2=5 bytes free but not contiguous enough)
        assert!(matches!(
            buf.write(b"ABCDE"),
            Err(BipBufferError::BufferFull { requested: 5 })
        ));

        // But 3 bytes should work (fits in region B)
        buf.write(b"ABC").unwrap();
        assert_eq!(buf.len(), 8); // 5 bytes in region A + 3 bytes in region B

        // And 2 more bytes should work (extends region A)
        buf.write(b"XY").unwrap();
        assert_eq!(buf.len(), 10); // Buffer is now full

        // Verify we can read both regions correctly
        assert_eq!(buf.read(), b"45678XY"); // Region A first (now extended)
        buf.consume(7).unwrap();
        assert_eq!(buf.read(), b"ABC"); // Region B
    }

    #[test]
    fn test_iterator() {
        let mut buf = BipBuffer::new(10);
        buf.write(b"test").unwrap();

        let collected: Vec<u8> = buf.iter().collect();
        assert_eq!(collected, b"test");
    }

    #[test]
    fn test_find() {
        let mut buf = BipBuffer::new(20);
        buf.write(b"hello world test").unwrap();

        assert_eq!(buf.find(b"world"), Some(6));
        assert_eq!(buf.find(b"xyz"), None);
        assert_eq!(buf.find(b"hello"), Some(0));
    }
}
