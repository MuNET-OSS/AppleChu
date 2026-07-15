pub struct IoBuf<'a> {
    bytes: &'a mut [u8],
    pub nbytes: usize,
    pub pos: usize,
}

impl<'a> IoBuf<'a> {
    pub fn new(bytes: &'a mut [u8]) -> Self {
        Self {
            bytes,
            nbytes: 0,
            pos: 0,
        }
    }

    pub fn with_data(bytes: &'a mut [u8], nbytes: usize) -> Self {
        let nbytes = nbytes.min(bytes.len());
        Self {
            bytes,
            nbytes,
            pos: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.bytes.len()
    }

    pub fn available(&self) -> usize {
        self.nbytes.saturating_sub(self.pos)
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.nbytes]
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes[..self.nbytes]
    }
}

pub fn iobuf_read(dest: &mut IoBuf<'_>, src: &mut IoBuf<'_>) -> usize {
    copy_available(dest, src)
}

pub fn iobuf_write(dest: &mut IoBuf<'_>, src: &mut IoBuf<'_>) -> usize {
    copy_available(dest, src)
}

fn copy_available(dest: &mut IoBuf<'_>, src: &mut IoBuf<'_>) -> usize {
    let len = dest
        .capacity()
        .saturating_sub(dest.nbytes)
        .min(src.available());
    if len == 0 {
        return 0;
    }

    let dest_start = dest.nbytes;
    let src_start = src.pos;
    dest.bytes[dest_start..dest_start + len]
        .copy_from_slice(&src.bytes[src_start..src_start + len]);
    dest.nbytes += len;
    src.pos += len;
    len
}
