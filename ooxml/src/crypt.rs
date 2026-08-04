//! ECMA-376 の文書暗号化(Standard Encryption。MS-OFFCRYPTO 2.3.4)。
//!
//! 暗号化した docx は zip ではなく**複合ファイル(CFB)**になり、
//! `EncryptionInfo`(鍵の作り方と照合用の暗号文)と `EncryptedPackage`
//! (中身の zip を AES-128-ECB で包んだもの)の2つの流れを持つ。
//! 鍵はパスワードと塩から SHA-1 を 50000 回まわして作る。
//! Word 2007 以来の「標準」方式 — Word も LibreOffice も開ける。
//!
//! Word 2013+ の既定である Agile 方式(XML の EncryptionInfo)は
//! **まだ読めない。読めないと正直に言う**(黙って壊さない)。

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use sha1::{Digest, Sha1};
use std::io::{Read, Write};

/// 鍵まわし(spin)の回数。Standard Encryption の固定値
const SPIN: u32 = 50_000;

/// 複合ファイル(CFB)の魔法数。これで始まれば「暗号化されているかも」
pub fn is_cfb(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
}

fn sha1(parts: &[&[u8]]) -> [u8; 20] {
    let mut d = Sha1::new();
    for p in parts {
        d.update(p);
    }
    d.finalize().into()
}

/// パスワードと塩から AES の鍵を作る(MS-OFFCRYPTO 2.3.4.7)。
fn derive_key(salt: &[u8], password: &str, key_len: usize) -> Vec<u8> {
    let pw: Vec<u8> = password
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let mut h = sha1(&[salt, &pw]);
    for i in 0..SPIN {
        h = sha1(&[&i.to_le_bytes(), &h]);
    }
    let hfin = sha1(&[&h, &0u32.to_le_bytes()]);
    // 64 バイトの 0x36 / 0x5C と xor して伸ばす(HMAC の親戚の形)
    let mut buf1 = [0x36u8; 64];
    for (b, x) in buf1.iter_mut().zip(hfin.iter()) {
        *b ^= x;
    }
    let x1 = sha1(&[&buf1]);
    if key_len <= 20 {
        return x1[..key_len].to_vec();
    }
    let mut buf2 = [0x5Cu8; 64];
    for (b, x) in buf2.iter_mut().zip(hfin.iter()) {
        *b ^= x;
    }
    let x2 = sha1(&[&buf2]);
    let mut key = x1.to_vec();
    key.extend_from_slice(&x2);
    key.truncate(key_len);
    key
}

/// AES-ECB(鍵の長さで 128/192/256 を選ぶ)。Standard Encryption は
/// CBC ではなく ECB — 仕様がそうなっている
enum Ecb {
    A128(aes::Aes128),
    A192(aes::Aes192),
    A256(aes::Aes256),
}

impl Ecb {
    fn new(key: &[u8]) -> Result<Ecb, String> {
        match key.len() {
            16 => Ok(Ecb::A128(aes::Aes128::new(GenericArray::from_slice(key)))),
            24 => Ok(Ecb::A192(aes::Aes192::new(GenericArray::from_slice(key)))),
            32 => Ok(Ecb::A256(aes::Aes256::new(GenericArray::from_slice(key)))),
            n => Err(format!("鍵の長さが変です: {n} バイト")),
        }
    }
    fn enc(&self, data: &mut [u8]) {
        for chunk in data.chunks_exact_mut(16) {
            let b = GenericArray::from_mut_slice(chunk);
            match self {
                Ecb::A128(c) => c.encrypt_block(b),
                Ecb::A192(c) => c.encrypt_block(b),
                Ecb::A256(c) => c.encrypt_block(b),
            }
        }
    }
    fn dec(&self, data: &mut [u8]) {
        for chunk in data.chunks_exact_mut(16) {
            let b = GenericArray::from_mut_slice(chunk);
            match self {
                Ecb::A128(c) => c.decrypt_block(b),
                Ecb::A192(c) => c.decrypt_block(b),
                Ecb::A256(c) => c.decrypt_block(b),
            }
        }
    }
}

fn urandom(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_err()
    {
        // 予備(乱数装置の無い環境)。塩の質は落ちるが動きはする
        let seed = std::process::id() as u64 ^ &buf as *const _ as u64;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (seed.wrapping_mul(6364136223846793005).wrapping_add(i as u64) >> 33) as u8;
        }
    }
    buf
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// 平文の docx(zip の丸ごと)をパスワードで包む。
pub fn encrypt(plain: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let salt = urandom(16);
    let verifier = urandom(16);
    let key = derive_key(&salt, password, 16);
    let ecb = Ecb::new(&key)?;

    let mut enc_verifier = verifier.clone();
    ecb.enc(&mut enc_verifier);
    let vh = sha1(&[&verifier]);
    let mut enc_vh = [0u8; 32];
    enc_vh[..20].copy_from_slice(&vh);
    ecb.enc(&mut enc_vh);

    // EncryptionInfo(version 3.2 = Standard)
    let csp: Vec<u8> = "Microsoft Enhanced RSA and AES Cryptographic Provider\0"
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let flags = 0x24u32; // fCryptoAPI | fAES
    let mut header = Vec::new();
    header.extend_from_slice(&flags.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes()); // sizeExtra
    header.extend_from_slice(&0x0000_660Eu32.to_le_bytes()); // AES-128
    header.extend_from_slice(&0x0000_8004u32.to_le_bytes()); // SHA-1
    header.extend_from_slice(&128u32.to_le_bytes()); // 鍵の長さ(bit)
    header.extend_from_slice(&0x18u32.to_le_bytes()); // providerType AES
    header.extend_from_slice(&0u32.to_le_bytes()); // reserved1
    header.extend_from_slice(&0u32.to_le_bytes()); // reserved2
    header.extend_from_slice(&csp);

    let mut info = Vec::new();
    info.extend_from_slice(&3u16.to_le_bytes()); // major
    info.extend_from_slice(&2u16.to_le_bytes()); // minor
    info.extend_from_slice(&flags.to_le_bytes());
    info.extend_from_slice(&(header.len() as u32).to_le_bytes());
    info.extend_from_slice(&header);
    info.extend_from_slice(&16u32.to_le_bytes()); // saltSize
    info.extend_from_slice(&salt);
    info.extend_from_slice(&enc_verifier);
    info.extend_from_slice(&20u32.to_le_bytes()); // verifierHashSize
    info.extend_from_slice(&enc_vh);

    // EncryptedPackage: 先頭 8 バイトが元の大きさ、続いて 16 の倍数に
    // 詰め物をした暗号文
    let mut data = plain.to_vec();
    let pad = (16 - data.len() % 16) % 16;
    data.extend(std::iter::repeat(0u8).take(pad));
    ecb.enc(&mut data);
    let mut package = Vec::new();
    package.extend_from_slice(&(plain.len() as u64).to_le_bytes());
    package.extend_from_slice(&data);

    // 複合ファイル(CFB)に納める
    let cur = std::io::Cursor::new(Vec::new());
    let mut comp = cfb::CompoundFile::create(cur).map_err(|e| e.to_string())?;
    comp.create_stream("/EncryptionInfo")
        .and_then(|mut s| s.write_all(&info))
        .map_err(|e| e.to_string())?;
    comp.create_stream("/EncryptedPackage")
        .and_then(|mut s| s.write_all(&package))
        .map_err(|e| e.to_string())?;
    comp.flush().map_err(|e| e.to_string())?;
    Ok(comp.into_inner().into_inner())
}

/// 暗号化されている docx(CFB)か。zip のままなら false
pub fn is_encrypted(bytes: &[u8]) -> bool {
    if !is_cfb(bytes) {
        return false;
    }
    cfb::CompoundFile::open(std::io::Cursor::new(bytes))
        .map(|mut c| c.exists("/EncryptionInfo"))
        .unwrap_or(false)
}

/// パスワードで解いて、平文の docx(zip の丸ごと)を返す。
pub fn decrypt(bytes: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let mut comp =
        cfb::CompoundFile::open(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut info = Vec::new();
    comp.open_stream("/EncryptionInfo")
        .and_then(|mut s| s.read_to_end(&mut info))
        .map_err(|_| "EncryptionInfo がありません(暗号化文書ではない?)".to_string())?;
    if info.len() < 12 {
        return Err("EncryptionInfo が短すぎます".into());
    }
    let major = u16::from_le_bytes([info[0], info[1]]);
    let minor = u16::from_le_bytes([info[2], info[3]]);
    if minor == 4 {
        return Err("新しい暗号方式(Agile。Word 2013+ の既定)はまだ解けません。\
                    Word の「名前を付けて保存 > ツール > 全般オプション」で\
                    保存し直したものなら解けることがあります"
            .into());
    }
    if !(2..=4).contains(&major) || minor != 2 {
        return Err(format!("知らない暗号の版です({major}.{minor})"));
    }
    let flags = le32(&info, 4);
    if flags & 0x20 == 0 {
        return Err("古い暗号方式(RC4)はまだ解けません".into());
    }
    let hsize = le32(&info, 8) as usize;
    let h0 = 12; // ヘッダーの頭
    if info.len() < h0 + hsize + 4 {
        return Err("EncryptionInfo が壊れています".into());
    }
    let key_bits = le32(&info, h0 + 16) as usize;
    let key_len = if key_bits == 0 { 16 } else { key_bits / 8 };
    let v0 = h0 + hsize; // 照合部の頭
    let salt_size = le32(&info, v0) as usize;
    let salt = &info[v0 + 4..v0 + 4 + salt_size];
    let ev0 = v0 + 4 + salt_size;
    let enc_verifier = &info[ev0..ev0 + 16];
    let vh_size = le32(&info, ev0 + 16) as usize;
    let enc_vh = &info[ev0 + 20..(ev0 + 20 + 32).min(info.len())];

    let key = derive_key(salt, password, key_len);
    let ecb = Ecb::new(&key)?;
    let mut v = enc_verifier.to_vec();
    ecb.dec(&mut v);
    let mut vh = enc_vh.to_vec();
    ecb.dec(&mut vh);
    if sha1(&[&v])[..vh_size.min(20)] != vh[..vh_size.min(20)] {
        return Err("パスワードが違います".into());
    }

    let mut package = Vec::new();
    comp.open_stream("/EncryptedPackage")
        .and_then(|mut s| s.read_to_end(&mut package))
        .map_err(|e| format!("EncryptedPackage が読めません: {e}"))?;
    if package.len() < 8 {
        return Err("EncryptedPackage が短すぎます".into());
    }
    let total = u64::from_le_bytes(package[..8].try_into().unwrap()) as usize;
    let mut data = package[8..].to_vec();
    let whole = data.len() - data.len() % 16;
    ecb.dec(&mut data[..whole]);
    if total > data.len() {
        return Err("大きさが合いません(壊れているようです)".into());
    }
    data.truncate(total);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 暗号化して戻ると同じで合言葉が違うと開かない() {
        let plain = b"PK\x03\x04 zip owo pretend bytes 123456789".to_vec();
        let enc = encrypt(&plain, "秘密の合言葉").expect("包めない");
        assert!(is_cfb(&enc), "CFB になっていない");
        assert!(is_encrypted(&enc), "暗号化と分からない");
        assert!(!is_encrypted(&plain), "平文を暗号化と誤認");
        let back = decrypt(&enc, "秘密の合言葉").expect("解けない");
        assert_eq!(back, plain, "解いた中身が違う");
        let e = decrypt(&enc, "まちがい").unwrap_err();
        assert!(e.contains("パスワード"), "違う言い方: {e}");
    }

    #[test]
    fn 文書ごと往復する() {
        let d = kumihan::Document::plain("暗号化の検査", 10.5);
        let mut plain = Vec::new();
        crate::write(&d, std::io::Cursor::new(&mut plain)).unwrap();
        let enc = encrypt(&plain, "かぎ").unwrap();
        let back = decrypt(&enc, "かぎ").unwrap();
        let (d2, _) = crate::read(std::io::Cursor::new(&back)).unwrap();
        assert_eq!(d2.body_text(), d.body_text());
    }
}
