// Hexadecimal encoding shared by every hash in this application. Keeping one
// implementation means session tokens, proof-of-work digests, and audit chain
// hashes are all rendered the same way.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Encodes a byte slice as a lowercase hexadecimal string.
pub fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}
