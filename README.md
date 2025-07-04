# BipBuffer

A safe, efficient bipbuffer implementation perfect for high-throughput network I/O, memcache servers, and streaming.

The bipbuffer maintains two regions in a single buffer:

```
┌─────────────────────────────────────────────┐
│  Region B  │     Free     │ Region A        │
│  [0..b_end]│              │[a_start..a_end] │
└─────────────────────────────────────────────┘
```

- ✅ **Memory safe** - 100% safe, no unsafe blocks
- ✅ **Simple API** - Reserve, commit, read, consume pattern


### Operation Flow

1. **Write Phase**: Reserve space → Write data → Commit
2. **Read Phase**: Read contiguous data → Consume processed bytes
3. **Wrap Around**: When Region A reaches the end, new data goes to Region B

### 🚀 Quick Start

```rust
use bipbuffer::{BipBuffer, BipBufferError};

fn main() -> Result<(), BipBufferError> {
    let mut buffer = BipBuffer::new(1024);
    
    // Write data
    buffer.write(b"Hello, World!")?;
    
    // Read data
    let data = buffer.read();
    println!("Read: {:?}", std::str::from_utf8(data).unwrap());
    
    // Consume processed data
    buffer.consume(data.len())?;
    
    Ok(())
}
```

The current implementation is **not thread-safe**. For concurrent access, wrap in a mutex:

```rust
use std::sync::{Arc, Mutex};

let shared_buffer = Arc::new(Mutex::new(BipBuffer::new(8192)));

// In producer thread
let producer_buffer = shared_buffer.clone();
std::thread::spawn(move || {
    let mut buffer = producer_buffer.lock().unwrap();
    buffer.write(data).unwrap();
});

// In consumer thread  
let consumer_buffer = shared_buffer.clone();
std::thread::spawn(move || {
    let mut buffer = consumer_buffer.lock().unwrap();
    let data = buffer.read();
    buffer.consume(data.len()).unwrap();
});
```

### 📚 Examples

#### `new(capacity: usize) -> BipBuffer`
Creates a new bipbuffer with the specified capacity.

```rust
let mut buffer = BipBuffer::new(8192);
```

#### `reserve(size: usize) -> Result<&mut [u8], BipBufferError>`
Reserves space for writing. Returns a mutable slice where you can write data.

```rust
let space = buffer.reserve(100)?;
space.copy_from_slice(my_data);
buffer.commit(my_data.len())?;
```

#### `commit(size: usize) -> Result<(), BipBufferError>`
Makes reserved data available for reading. Must be called after `reserve()`.

#### `write(data: &[u8]) -> Result<(), BipBufferError>`
Convenience method that combines `reserve()` and `commit()`.

```rust
buffer.write(b"Hello, World!")?;
```

#### `read() -> &[u8]`
Returns a slice of readable data. May need multiple calls due to wrapping.

```rust
while !buffer.is_empty() {
    let data = buffer.read();
    process_data(data);
    buffer.consume(data.len())?;
}
```

#### `consume(size: usize) -> Result<(), BipBufferError>`
Marks data as processed and removes it from the buffer.

### 🎭 Visual Example

Let's trace through a complete example:

```rust
let mut buffer = BipBuffer::new(8);

// 1. Write "ABCD"
buffer.write(b"ABCD")?;
// Buffer: [A B C D _ _ _ _]
//          ^     ^
//          a_start=0, a_end=4

// 2. Consume 2 bytes
buffer.consume(2)?;
// Buffer: [A B C D _ _ _ _]
//              ^   ^
//              a_start=2, a_end=4

// 3. Write "XYZ" - goes after Region A
buffer.write(b"XYZ")?;
// Buffer: [A B C D X Y Z _]
//              ^       ^
//              a_start=2, a_end=7

// 4. Write "12" - wraps to Region B
buffer.write(b"12")?;
// Buffer: [1 2 C D X Y Z _]
//          ^   ^       ^
//          b_end=2, a_start=2, a_end=7

// 5. Read data
let data1 = buffer.read(); // Returns "CDXYZ" (Region A)
buffer.consume(5)?;

let data2 = buffer.read(); // Returns "12" (Region B)
buffer.consume(2)?;
```

## Acknowledgments

- Inspired by the original [bipbuffer concept](https://www.codeproject.com/Articles/3479/The-Bip-Buffer-The-Circular-Buffer-with-a-Twist) by Simon Cooke
- Inspired by [bipbuffer crate](https://www.codeproject.com/Articles/3479/The-Bip-Buffer-The-Circular-Buffer-with-a-Twist)
- Inspired by [bipbuffer of memcache](https://github.com/memcached/memcached/blob/master/bipbuffer.c)
