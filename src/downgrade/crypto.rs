//! Steam's symmetric crypto (AES-256: the IV itself ECB-encrypted in the
//! first block, then CBC with PKCS#7) and its chunk containers: "VZ" (raw
//! LZMA), "VSZa" (zstd) or a plain zip.

use aes::Aes256;
use aes::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
use std::io::Read;

pub fn decrypt(data: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    if key.len() != 32 {
        return Err("bad key length".into());
    }
    if data.len() < 32 || !data.len().is_multiple_of(16) {
        return Err("bad ciphertext length".into());
    }
    let aes = Aes256::new(GenericArray::from_slice(key));
    let mut iv = GenericArray::clone_from_slice(&data[..16]);
    aes.decrypt_block(&mut iv);
    let mut out = Vec::with_capacity(data.len() - 16);
    let mut prev: [u8; 16] = iv.into();
    for block in data[16..].as_chunks::<16>().0 {
        let mut b = GenericArray::clone_from_slice(block);
        aes.decrypt_block(&mut b);
        for (o, p) in b.iter_mut().zip(prev) {
            *o ^= p;
        }
        out.extend_from_slice(&b);
        prev.copy_from_slice(block);
    }
    let pad = *out.last().ok_or("empty")? as usize;
    if pad == 0
        || pad > 16
        || pad > out.len()
        || out[out.len() - pad..].iter().any(|&b| b as usize != pad)
    {
        return Err("bad padding (wrong key?)".into());
    }
    out.truncate(out.len() - pad);
    Ok(out)
}

/// A downloaded chunk to its plain bytes.
pub fn decode_chunk(data: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    let d = decrypt(data, key)?;
    if d.starts_with(b"VZ") {
        // "VZ" 'a' crc(4) | props(5) | lzma … | crc(4) size(4) "zv"
        if d.len() < 12 + 10 || !d.ends_with(b"zv") || d[2] != b'a' {
            return Err("VZ: bad frame".into());
        }
        let n = d.len();
        let crc = u32::from_le_bytes(d[n - 10..n - 6].try_into().unwrap());
        let size = u32::from_le_bytes(d[n - 6..n - 2].try_into().unwrap()) as u64;
        // Make it a .lzma stream: props + unpacked size + data.
        let mut stream = Vec::with_capacity(n);
        stream.extend_from_slice(&d[7..12]);
        stream.extend_from_slice(&size.to_le_bytes());
        stream.extend_from_slice(&d[12..n - 10]);
        let mut out = Vec::with_capacity(size as usize);
        lzma_rs::lzma_decompress_with_options(
            &mut std::io::Cursor::new(stream),
            &mut out,
            &lzma_rs::decompress::Options {
                unpacked_size: lzma_rs::decompress::UnpackedSize::ReadFromHeader,
                allow_incomplete: true,
                ..Default::default()
            },
        )
        .map_err(|e| format!("VZ: {e}"))?;
        out.truncate(size as usize);
        if crc32fast::hash(&out) != crc {
            return Err("VZ: checksum".into());
        }
        Ok(out)
    } else if d.starts_with(b"VSZa") {
        // "VSZa" crc(4) | zstd … | crc(4) size(4) "zsv"  (15-byte footer)
        let n = d.len();
        if n < 8 + 15 || !d.ends_with(b"zsv") {
            return Err("VSZ: bad frame".into());
        }
        let crc = u32::from_le_bytes(d[4..8].try_into().unwrap());
        let size = u32::from_le_bytes(d[n - 11..n - 7].try_into().unwrap()) as usize;
        let mut dec = ruzstd::decoding::StreamingDecoder::new(std::io::Cursor::new(&d[8..n - 15]))
            .map_err(|e| format!("VSZ: {e}"))?;
        let mut out = Vec::with_capacity(size);
        dec.read_to_end(&mut out).map_err(|e| format!("VSZ: {e}"))?;
        out.truncate(size);
        if crc32fast::hash(&out) != crc {
            return Err("VSZ: checksum".into());
        }
        Ok(out)
    } else {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(d)).map_err(|e| e.to_string())?;
        let mut f = z.by_index(0).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        f.read_to_end(&mut out).map_err(|e| e.to_string())?;
        Ok(out)
    }
}

pub fn sha1(b: &[u8]) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    Sha1::digest(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncrypt;

    fn encrypt(plain: &[u8], key: &[u8], iv: [u8; 16]) -> Vec<u8> {
        let aes = Aes256::new(GenericArray::from_slice(key));
        let mut out = Vec::new();
        let mut eiv = GenericArray::clone_from_slice(&iv);
        aes.encrypt_block(&mut eiv);
        out.extend_from_slice(&eiv);
        let pad = 16 - plain.len() % 16;
        let mut p = plain.to_vec();
        p.extend(std::iter::repeat_n(pad as u8, pad));
        let mut prev = iv;
        for block in p.chunks(16) {
            let mut b = GenericArray::clone_from_slice(block);
            for (x, y) in b.iter_mut().zip(prev) {
                *x ^= y;
            }
            aes.encrypt_block(&mut b);
            prev.copy_from_slice(&b);
            out.extend_from_slice(&b);
        }
        out
    }

    #[test]
    fn aes_round_trip() {
        let key = [7u8; 32];
        for len in [0, 5, 16, 33] {
            let plain: Vec<u8> = (0..len as u8).collect();
            let c = encrypt(&plain, &key, [3; 16]);
            assert_eq!(decrypt(&c, &key).unwrap(), plain);
        }
    }

    #[test]
    fn vz_chunk() {
        // An LZMA stream made by lzma-rs, put in a VZ frame.
        let plain = b"hello hello hello steam depot chunk".repeat(20);
        let mut lz = Vec::new();
        lzma_rs::lzma_compress(&mut std::io::Cursor::new(&plain), &mut lz).unwrap();
        let mut vz = b"VZa\0\0\0\0".to_vec();
        vz.extend_from_slice(&lz[..5]); // props
        vz.extend_from_slice(&lz[13..]); // after the 8-byte size
        vz.extend_from_slice(&crc32fast::hash(&plain).to_le_bytes());
        vz.extend_from_slice(&(plain.len() as u32).to_le_bytes());
        vz.extend_from_slice(b"zv");
        let key = [9u8; 32];
        let c = encrypt(&vz, &key, [1; 16]);
        assert_eq!(decode_chunk(&c, &key).unwrap(), plain);
    }
}
