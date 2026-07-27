// NIP-19: bech32-encoded entities
// Encodes and decodes npub, nsec, note, nprofile, nevent, naddr
#![allow(dead_code)]

const CHARSET: [char; 32] = [
    'q', 'p', 'z', 'r', 'y', '9', 'x', '8', 'g', 'f', '2', 't', 'v', 'd', 'w', '0', 's', '3', 'j',
    'n', '5', '4', 'k', 'h', 'c', 'e', '6', 'm', 'u', 'a', '7', 'l',
];

fn charset_index(c: char) -> Option<u8> {
    CHARSET.iter().position(|&x| x == c).map(|i| i as u8)
}

fn polymod(values: &[u8]) -> u32 {
    let gen: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for &v in values {
        let b = chk >> 25;
        chk = ((chk & 0x01ffffff) << 5) ^ (v as u32);
        for (i, g) in gen.iter().enumerate() {
            if (b >> i) & 1 != 0 {
                chk ^= g;
            }
        }
    }
    chk
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for c in hrp.chars() {
        v.push((c as u8) >> 5);
    }
    v.push(0);
    for c in hrp.chars() {
        v.push((c as u8) & 0x1f);
    }
    v
}

fn create_checksum(hrp: &str, data: &[u8]) -> Vec<u8> {
    let mut values = hrp_expand(hrp);
    values.extend_from_slice(data);
    values.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let polymod = polymod(&values) ^ 1;
    let mut checksum = Vec::new();
    for i in 0..6 {
        checksum.push(((polymod >> (5 * (5 - i))) & 0x1f) as u8);
    }
    checksum
}

fn verify_checksum(hrp: &str, data: &[u8]) -> bool {
    let mut values = hrp_expand(hrp);
    values.extend_from_slice(data);
    polymod(&values) == 1
}

fn encode_bech32(hrp: &str, data: &[u8]) -> String {
    let checksum = create_checksum(hrp, data);
    let mut result = String::from(hrp);
    result.push('1');
    for &b in data.iter().chain(checksum.iter()) {
        result.push(CHARSET[b as usize]);
    }
    result
}

fn decode_bech32(encoded: &str) -> Option<(String, Vec<u8>)> {
    let (hrp, data_str) = encoded.split_once('1')?;
    if hrp.is_empty() || hrp.len() > 83 || !hrp.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let mut data = Vec::new();
    for c in data_str.chars() {
        data.push(charset_index(c)?);
    }
    if data.len() < 6 {
        return None;
    }
    let payload = &data[..data.len() - 6];
    let full = &data;
    if !verify_checksum(hrp, full) {
        return None;
    }
    Some((hrp.to_string(), payload.to_vec()))
}

fn u8_to_u5(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut acc: u16 = 0;
    let mut bits: u8 = 0;
    for &byte in data {
        acc = (acc << 8) | byte as u16;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            result.push(((acc >> bits) & 0x1f) as u8);
        }
    }
    if bits > 0 {
        result.push(((acc << (5 - bits)) & 0x1f) as u8);
    }
    result
}

fn u5_to_u8(data: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::new();
    let mut acc: u16 = 0;
    let mut bits: u8 = 0;
    for &b in data {
        if b > 31 {
            return None;
        }
        acc = (acc << 5) | b as u16;
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            result.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits >= 5 || acc & ((1 << bits) - 1) != 0 {
        return None;
    }
    Some(result)
}

// TLV (Type-Length-Value) encoding for complex NIP-19 types
const TLV_SPECIAL: u8 = 0;
const TLV_RELAY: u8 = 1;
const TLV_AUTHOR: u8 = 2;
const TLV_KIND: u8 = 3;

type TlvResult = (Option<[u8; 32]>, Vec<String>, Option<[u8; 32]>, Option<u64>);

fn parse_tlv(data: &[u8]) -> Option<TlvResult> {
    let mut special: Option<[u8; 32]> = None;
    let mut relays: Vec<String> = Vec::new();
    let mut author: Option<[u8; 32]> = None;
    let mut kind: Option<u64> = None;
    let mut i = 0;
    while i + 2 <= data.len() {
        let t = data[i];
        let len = data[i + 1] as usize;
        i += 2;
        if i + len > data.len() {
            return None;
        }
        let val = &data[i..i + len];
        match t {
            TLV_SPECIAL => {
                if len == 32 {
                    special = Some(val.try_into().ok()?);
                }
            }
            TLV_RELAY => {
                if let Ok(s) = std::str::from_utf8(val) {
                    relays.push(s.to_string());
                }
            }
            TLV_AUTHOR => {
                if len == 32 {
                    author = Some(val.try_into().ok()?);
                }
            }
            TLV_KIND if len <= 8 => {
                let mut bytes = [0u8; 8];
                bytes[8 - len..].copy_from_slice(val);
                kind = Some(u64::from_be_bytes(bytes));
            }
            _ => {}
        }
        i += len;
    }
    Some((special, relays, author, kind))
}

fn encode_tlv(
    special: Option<&[u8; 32]>,
    relays: &[String],
    author: Option<&[u8; 32]>,
    kind: Option<u32>,
) -> Vec<u8> {
    let mut data = Vec::new();
    if let Some(s) = special {
        data.push(TLV_SPECIAL);
        data.push(32);
        data.extend_from_slice(s);
    }
    for r in relays {
        let r = r.as_bytes();
        data.push(TLV_RELAY);
        data.push(r.len() as u8);
        data.extend_from_slice(r);
    }
    if let Some(a) = author {
        data.push(TLV_AUTHOR);
        data.push(32);
        data.extend_from_slice(a);
    }
    if let Some(k) = kind {
        let kb = k.to_be_bytes();
        data.push(TLV_KIND);
        data.push(4);
        data.extend_from_slice(&kb);
    }
    data
}

/// Encode a hex public key to npub
pub fn npub_encode(pubkey_hex: &str) -> Option<String> {
    let bytes = hex::decode(pubkey_hex).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    Some(encode_bech32("npub", &u8_to_u5(&bytes)))
}

/// Decode an npub to hex public key
pub fn npub_decode(npub: &str) -> Option<String> {
    let (hrp, data) = decode_bech32(npub)?;
    if hrp != "npub" {
        return None;
    }
    let bytes = u5_to_u8(&data)?;
    if bytes.len() != 32 {
        return None;
    }
    Some(hex::encode(&bytes))
}

/// Encode a hex private key to nsec
pub fn nsec_encode(privkey_hex: &str) -> Option<String> {
    let bytes = hex::decode(privkey_hex).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    Some(encode_bech32("nsec", &u8_to_u5(&bytes)))
}

/// Encode a hex event ID to note
pub fn note_encode(event_id_hex: &str) -> Option<String> {
    let bytes = hex::decode(event_id_hex).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    Some(encode_bech32("note", &u8_to_u5(&bytes)))
}

/// Decode a note to hex event ID
pub fn note_decode(note: &str) -> Option<String> {
    let (hrp, data) = decode_bech32(note)?;
    if hrp != "note" {
        return None;
    }
    let bytes = u5_to_u8(&data)?;
    if bytes.len() != 32 {
        return None;
    }
    Some(hex::encode(&bytes))
}

/// Encode an nprofile (pubkey + relay hints)
pub fn nprofile_encode(pubkey_hex: &str, relays: &[String]) -> Option<String> {
    let pubkey_bytes: [u8; 32] = hex::decode(pubkey_hex).ok()?.try_into().ok()?;
    let tlv = encode_tlv(Some(&pubkey_bytes), relays, None, None);
    Some(encode_bech32("nprofile", &u8_to_u5(&tlv)))
}

/// Decode an nprofile to (pubkey_hex, relays)
pub fn nprofile_decode(nprofile: &str) -> Option<(String, Vec<String>)> {
    let (hrp, data) = decode_bech32(nprofile)?;
    if hrp != "nprofile" {
        return None;
    }
    let tlv = u5_to_u8(&data)?;
    let (special, relays, ..) = parse_tlv(&tlv)?;
    Some((hex::encode(special?), relays))
}

/// Encode an nevent (event_id + relay hints + optional author + kind)
pub fn nevent_encode(
    event_id_hex: &str,
    relays: &[String],
    author: Option<&str>,
    kind: Option<u32>,
) -> Option<String> {
    let id_bytes: [u8; 32] = hex::decode(event_id_hex).ok()?.try_into().ok()?;
    let author_bytes: Option<[u8; 32]> = author
        .and_then(|a| hex::decode(a).ok())
        .and_then(|a| a.try_into().ok());
    let tlv = encode_tlv(Some(&id_bytes), relays, author_bytes.as_ref(), kind);
    Some(encode_bech32("nevent", &u8_to_u5(&tlv)))
}

/// Decode an nevent to (event_id_hex, relays, author_hex, kind)
type NeventResult = (String, Vec<String>, Option<String>, Option<u64>);

pub fn nevent_decode(nevent: &str) -> Option<NeventResult> {
    let (hrp, data) = decode_bech32(nevent)?;
    if hrp != "nevent" {
        return None;
    }
    let tlv = u5_to_u8(&data)?;
    let (special, relays, author, kind) = parse_tlv(&tlv)?;
    Some((hex::encode(special?), relays, author.map(hex::encode), kind))
}

/// Encode an naddr (kind + pubkey + d_tag + relay hints)
pub fn naddr_encode(kind: u32, pubkey_hex: &str, d_tag: &str, relays: &[String]) -> Option<String> {
    let identifier = d_tag.as_bytes();
    let pubkey_bytes: [u8; 32] = hex::decode(pubkey_hex).ok()?.try_into().ok()?;
    let mut tlv = Vec::new();
    // type 0 = d-tag
    tlv.push(0);
    tlv.push(identifier.len() as u8);
    tlv.extend_from_slice(identifier);
    // type 1 = relays
    for r in relays {
        let r = r.as_bytes();
        tlv.push(TLV_RELAY);
        tlv.push(r.len() as u8);
        tlv.extend_from_slice(r);
    }
    // type 2 = pubkey
    tlv.push(TLV_AUTHOR);
    tlv.push(32);
    tlv.extend_from_slice(&pubkey_bytes);
    // type 3 = kind
    tlv.push(TLV_KIND);
    tlv.push(4);
    tlv.extend_from_slice(&kind.to_be_bytes());
    Some(encode_bech32("naddr", &u8_to_u5(&tlv)))
}

/// Decode an naddr to (kind, pubkey_hex, d_tag, relays)
pub fn naddr_decode(naddr: &str) -> Option<(u64, String, String, Vec<String>)> {
    let (hrp, data) = decode_bech32(naddr)?;
    if hrp != "naddr" {
        return None;
    }
    let tlv = u5_to_u8(&data)?;
    let mut pubkey: Option<[u8; 32]> = None;
    let mut kind: Option<u64> = None;
    let mut identifier: Option<String> = None;
    let mut relays: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 2 <= tlv.len() {
        let t = tlv[i];
        let len = tlv[i + 1] as usize;
        i += 2;
        if i + len > tlv.len() {
            return None;
        }
        let val = &tlv[i..i + len];
        match t {
            0 => {
                identifier = Some(String::from_utf8_lossy(val).to_string());
            }
            TLV_RELAY => {
                if let Ok(s) = std::str::from_utf8(val) {
                    relays.push(s.to_string());
                }
            }
            TLV_AUTHOR => {
                if len == 32 {
                    pubkey = Some(val.try_into().ok()?);
                }
            }
            TLV_KIND if len <= 8 => {
                let mut bytes = [0u8; 8];
                bytes[8 - len..].copy_from_slice(val);
                kind = Some(u64::from_be_bytes(bytes));
            }
            _ => {}
        }
        i += len;
    }
    Some((
        kind?,
        hex::encode(pubkey?),
        identifier.unwrap_or_default(),
        relays,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_npub_encode_decode() {
        // From NIP-19 spec
        let pubkey = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
        let npub = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";
        assert_eq!(npub_encode(pubkey).unwrap(), npub);
        assert_eq!(npub_decode(npub).unwrap(), pubkey);
    }

    #[test]
    fn test_note_encode_decode() {
        let event_id = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
        let encoded = note_encode(event_id).unwrap();
        let decoded = note_decode(&encoded).unwrap();
        assert_eq!(decoded, event_id);
        assert!(encoded.starts_with("note1"));
    }

    #[test]
    fn test_nprofile_encode_decode() {
        let pubkey = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
        let relays = vec!["wss://r.example.com".to_string()];
        let encoded = nprofile_encode(pubkey, &relays).unwrap();
        let (pk, r) = nprofile_decode(&encoded).unwrap();
        assert_eq!(pk, pubkey);
        assert_eq!(r, relays);
    }

    #[test]
    fn test_naddr_encode_decode() {
        let pubkey = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
        let encoded = naddr_encode(30023, pubkey, "my-article", &[]).unwrap();
        let (kind, pk, d_tag, _) = naddr_decode(&encoded).unwrap();
        assert_eq!(kind, 30023);
        assert_eq!(pk, pubkey);
        assert_eq!(d_tag, "my-article");
    }
}
