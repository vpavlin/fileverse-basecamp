//! Base64 the way Fileverse writes it: js-base64's `toUint8Array` accepts the
//! standard and the URL-safe alphabet, padded or not, and Fileverse uses all four.

use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine;

const LENIENT: GeneralPurposeConfig = GeneralPurposeConfig::new()
    .with_decode_padding_mode(DecodePaddingMode::Indifferent)
    .with_decode_allow_trailing_bits(true);
const STD: GeneralPurpose = GeneralPurpose::new(&alphabet::STANDARD, LENIENT);

pub fn decode(s: &str) -> Result<Vec<u8>, String> {
    let t: String = s
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    STD.decode(t.as_bytes()).map_err(|e| format!("bad base64: {e}"))
}

pub fn encode_url(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

pub fn encode(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_spellings() {
        let want = vec![0xfb, 0xff, 0xfe, 0x01];
        for s in ["+//+AQ==", "+//+AQ", "-__-AQ==", "-__-AQ"] {
            assert_eq!(decode(s).unwrap(), want, "{s}");
        }
    }
}
