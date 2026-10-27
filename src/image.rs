//! Minimal PNG writer (uncompressed deflate), so frames can be saved without dependencies.

use std::io::{self, Write};

fn crc32(chunks: &[&[u8]]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for data in chunks {
        for &b in *data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { 0xedb8_8320 ^ (crc >> 1) } else { crc >> 1 };
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn write_chunk(out: &mut impl Write, kind: &[u8; 4], data: &[u8]) -> io::Result<()> {
    out.write_all(&(data.len() as u32).to_be_bytes())?;
    out.write_all(kind)?;
    out.write_all(data)?;
    out.write_all(&crc32(&[kind, data]).to_be_bytes())
}

/// Saves 0x00RRGGBB pixels (row-major, top row first) as an RGB PNG.
pub fn write_png(path: &std::path::Path, width: usize, height: usize, pixels: &[u32]) -> io::Result<()> {
    assert_eq!(pixels.len(), width * height);
    let mut raw = Vec::with_capacity(height * (1 + width * 3));
    for row in pixels.chunks_exact(width) {
        raw.push(0); // filter: none
        for &p in row {
            raw.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8); // BFINAL, BTYPE=00 (stored)
        let len = block.len() as u16;
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend(adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend((width as u32).to_be_bytes());
    ihdr.extend((height as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]); // 8-bit, truecolor RGB
    let mut out = io::BufWriter::new(std::fs::File::create(path)?);
    out.write_all(b"\x89PNG\r\n\x1a\n")?;
    write_chunk(&mut out, b"IHDR", &ihdr)?;
    write_chunk(&mut out, b"IDAT", &z)?;
    write_chunk(&mut out, b"IEND", &[])?;
    out.flush()
}
