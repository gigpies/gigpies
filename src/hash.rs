//! Version-independent lowercase hexadecimal digest representation.
//! Hash inputs and algorithms remain owned by their callers.

/// Encode every byte as two lowercase digits, retaining leading zeroes.
pub fn hex(bytes: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    result
}

/// Preserve streaming `io::copy` hashing after digest removed its `Write` impl.
#[cfg(feature = "hardware-host")]
pub(crate) struct DigestWriter<'a, D>(pub &'a mut D);
#[cfg(feature = "hardware-host")]
impl<D: sha2::Digest> std::io::Write for DigestWriter<'_, D> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn standard_vectors_preserve_stored_and_wire_sha256() {
        for (input, expected) in [
            (
                b"".as_slice(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc".as_slice(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".as_slice(),
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ] {
            assert_eq!(hex(Sha256::digest(input)), expected);
            let mut streaming = Sha256::new();
            for byte in input {
                streaming.update([*byte]);
            }
            assert_eq!(hex(streaming.finalize()), expected);
        }
    }

    #[test]
    fn leading_zeroes_and_device_lock_name_are_stable() {
        assert_eq!(hex([0, 1, 15, 16, 127, 128, 254, 255]), "00010f107f80feff");
        assert_eq!(
            format!("device-{}.lock", hex(Sha256::digest(b"abc"))),
            "device-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.lock"
        );
    }

    #[cfg(feature = "hardware-host")]
    #[test]
    fn streamed_library_hash_keeps_bytes_and_read_errors() {
        use std::io::Read;
        let mut digest = Sha256::new();
        let copied = std::io::copy(
            &mut std::io::repeat(b'a').take(1_000_000),
            &mut DigestWriter(&mut digest),
        )
        .unwrap();
        assert_eq!(copied, 1_000_000);
        assert_eq!(
            hex(digest.finalize()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        struct Failed;
        impl Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("read failed"))
            }
        }
        let mut digest = Sha256::new();
        let mut reader = b"abc".as_slice().chain(Failed);
        assert!(std::io::copy(&mut reader, &mut DigestWriter(&mut digest)).is_err());
        assert_eq!(hex(digest.finalize()), hex(Sha256::digest(b"abc")));
    }
}
