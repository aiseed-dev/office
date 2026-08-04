//! ECMA-376 の文書暗号化(Standard Encryption。MS-OFFCRYPTO 2.3.4)。
//!
//! 暗号化した docx は zip ではなく**複合ファイル(CFB)**になり、
//! `EncryptionInfo`(鍵の作り方と照合用の暗号文)と `EncryptedPackage`
//! (中身の zip を AES-128-ECB で包んだもの)の2つの流れを持つ。
//! 鍵はパスワードと塩から SHA-1 を 50000 回まわして作る。
//! Word 2007 以来の「標準」方式 — Word も LibreOffice も開ける。
//!
//! Word/Excel 2013+ の既定である Agile 方式(XML の EncryptionInfo)も
//! 読み書きする(このファイルの後半)。SHA-512 の鍵導出を spinCount 回
//! (既定 10 万)まわし、中身は AES-256-CBC で 4096 バイトずつ区分し、
//! 完全性は HMAC-SHA512 で照合する。**本物との相互検証済み**:
//! 自作の出力は msoffcrypto-tool(実物の Office 暗号を解く道具)が解け、
//! LibreOffice が書いた本物を自作が解いた結果は msoffcrypto と1バイト一致。

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
        .map(|c| c.exists("/EncryptionInfo"))
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
        // Agile 方式(Word/Excel 2013+ の既定)。記述子は XML
        let xml = String::from_utf8_lossy(&info[8..]).to_string();
        let mut package = Vec::new();
        comp.open_stream("/EncryptedPackage")
            .and_then(|mut s| s.read_to_end(&mut package))
            .map_err(|e| format!("EncryptedPackage が読めません: {e}"))?;
        return agile_decrypt(&xml, &package, password);
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

// ---- Agile 方式(MS-OFFCRYPTO 2.3.4.10〜15。Word/Excel 2013+ の既定) ----
//
// 記述子が XML になり、鍵導出は SHA-512 を spinCount(既定 10 万)回、
// 中身は AES-256-CBC で 4096 バイトずつ(区分ごとに IV が変わる)。
// 完全性は HMAC-SHA512。読み書きとも対応する。

use sha2::{Sha256, Sha512};

/// Agile の判(hashAlgorithm)。SHA512 が既定、古い道具の SHA1/256 も受ける
#[derive(Clone, Copy)]
enum Hasher {
    S1,
    S256,
    S512,
}

impl Hasher {
    fn from_name(n: &str) -> Result<Hasher, String> {
        match n {
            "SHA1" | "SHA-1" => Ok(Hasher::S1),
            "SHA256" | "SHA-256" => Ok(Hasher::S256),
            "SHA512" | "SHA-512" => Ok(Hasher::S512),
            other => Err(format!("知らないハッシュです: {other}")),
        }
    }
    fn digest(&self, parts: &[&[u8]]) -> Vec<u8> {
        match self {
            Hasher::S1 => sha1(parts).to_vec(),
            Hasher::S256 => {
                let mut d = Sha256::new();
                for p in parts {
                    d.update(p);
                }
                d.finalize().to_vec()
            }
            Hasher::S512 => {
                let mut d = Sha512::new();
                for p in parts {
                    d.update(p);
                }
                d.finalize().to_vec()
            }
        }
    }
    fn block_len(&self) -> usize {
        match self {
            Hasher::S1 | Hasher::S256 => 64,
            Hasher::S512 => 128,
        }
    }
}

/// HMAC(完全性の照合)。判は Agile の hashAlgorithm と同じもの
fn hmac(h: Hasher, key: &[u8], msg: &[u8]) -> Vec<u8> {
    let bl = h.block_len();
    let mut k = if key.len() > bl { h.digest(&[key]) } else { key.to_vec() };
    k.resize(bl, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5C).collect();
    let inner = h.digest(&[&ipad, msg]);
    h.digest(&[&opad, &inner])
}

/// AES-CBC。連鎖は ECB の器(Ecb)を使って手で回す
fn cbc_dec(ecb: &Ecb, iv: &[u8], data: &mut [u8]) {
    let mut prev = iv[..16].to_vec();
    for chunk in data.chunks_exact_mut(16) {
        let cipher = chunk.to_vec();
        ecb.dec(chunk);
        for (b, p) in chunk.iter_mut().zip(prev.iter()) {
            *b ^= p;
        }
        prev = cipher;
    }
}

fn cbc_enc(ecb: &Ecb, iv: &[u8], data: &mut [u8]) {
    let mut prev = iv[..16].to_vec();
    for chunk in data.chunks_exact_mut(16) {
        for (b, p) in chunk.iter_mut().zip(prev.iter()) {
            *b ^= p;
        }
        ecb.enc(chunk);
        prev = chunk.to_vec();
    }
}

/// 目的別の鍵(H(spun済み ∥ blockKey) を鍵長に切り、足りなければ 0x36 で埋める)
fn agile_block_key(h: Hasher, spun: &[u8], block: &[u8], key_len: usize) -> Vec<u8> {
    let mut k = h.digest(&[spun, block]);
    k.resize(key_len, 0x36);
    k
}

/// IV(salt をそのまま、または H(salt ∥ blockKey) を blockSize に)
fn agile_iv(h: Hasher, salt: &[u8], block: Option<&[u8]>, block_size: usize) -> Vec<u8> {
    let mut iv = match block {
        Some(b) => h.digest(&[salt, b]),
        None => salt.to_vec(),
    };
    iv.resize(block_size, 0x36);
    iv
}

const BK_VER_INPUT: [u8; 8] = [0xFE, 0xA7, 0xD2, 0x76, 0x3B, 0x4B, 0x9E, 0x79];
const BK_VER_VALUE: [u8; 8] = [0xD7, 0xAA, 0x0F, 0x6D, 0x30, 0x61, 0x34, 0x4E];
const BK_KEY_VALUE: [u8; 8] = [0x14, 0x6E, 0x0B, 0xE7, 0xAB, 0xAC, 0xD0, 0xD6];
const BK_HMAC_KEY: [u8; 8] = [0x5F, 0xB2, 0xAD, 0x01, 0x0C, 0xB9, 0xE1, 0xF6];
const BK_HMAC_VALUE: [u8; 8] = [0xA0, 0x67, 0x7F, 0x02, 0xB2, 0x2C, 0x84, 0x33];

/// base64(記述子の中の塩・暗号文の書き方)。依存を増やさず手で書く
fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let val = |c: u8| -> Result<u32, String> {
        Ok(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return Err("base64 が壊れています".into()),
        })
    };
    let cs: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::new();
    for q in cs.chunks(4) {
        if q.len() < 2 {
            return Err("base64 が壊れています".into());
        }
        let pad = q.iter().filter(|c| **c == b'=').count();
        let mut n = 0u32;
        for (i, c) in q.iter().enumerate() {
            n |= if *c == b'=' { 0 } else { val(*c)? } << (18 - i * 6);
        }
        out.push((n >> 16) as u8);
        if q.len() > 2 && pad < 2 {
            out.push((n >> 8) as u8);
        }
        if q.len() > 3 && pad < 1 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

/// 記述子 XML の中の、ある要素の属性を引く(quick-xml を出すまでもない大きさ)
fn xattr(xml: &str, elem: &str, attr: &str) -> Option<String> {
    let start = xml.find(&format!("<{elem}")).or_else(|| xml.find(&format!(":{elem}")))?;
    let rest = &xml[start..];
    let end = rest.find('>')?;
    let tag = &rest[..end];
    let key = format!("{attr}=\"");
    let a = tag.find(&key)? + key.len();
    let b = tag[a..].find('"')? + a;
    Some(tag[a..b].to_string())
}

/// パスワードから spin 済みハッシュを作る(Agile。塩 ∥ UTF-16LE の合言葉)
fn agile_spin(h: Hasher, salt: &[u8], password: &str, spin: u32) -> Vec<u8> {
    let pw: Vec<u8> = password.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut cur = h.digest(&[salt, &pw]);
    for i in 0..spin {
        cur = h.digest(&[&i.to_le_bytes(), &cur]);
    }
    cur
}

/// Agile の復号。記述子(XML)と EncryptedPackage の流れから平文の zip を返す
fn agile_decrypt(xml: &str, package: &[u8], password: &str) -> Result<Vec<u8>, String> {
    // keyData(中身の暗号の諸元)と、パスワードの keyEncryptor の諸元
    let kd_salt = b64_decode(&xattr(xml, "keyData", "saltValue").ok_or("keyData がありません")?)?;
    let kd_hash = Hasher::from_name(
        &xattr(xml, "keyData", "hashAlgorithm").unwrap_or_else(|| "SHA512".into()),
    )?;
    let kd_block = xattr(xml, "keyData", "blockSize")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(16);
    let kd_bits = xattr(xml, "keyData", "keyBits")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(256);
    if xattr(xml, "keyData", "cipherChaining").as_deref() == Some("ChainingModeCFB") {
        return Err("CFB 連鎖の暗号はまだ解けません".into());
    }
    // encryptedKey(パスワード側)
    let ek = xml
        .find(":encryptedKey")
        .or_else(|| xml.find("<encryptedKey"))
        .map(|i| &xml[i..])
        .ok_or("パスワードの keyEncryptor がありません")?;
    let ek_salt = b64_decode(&xattr(ek, "encryptedKey", "saltValue").ok_or("塩がありません")?)?;
    let ek_hash = Hasher::from_name(
        &xattr(ek, "encryptedKey", "hashAlgorithm").unwrap_or_else(|| "SHA512".into()),
    )?;
    let ek_block = xattr(ek, "encryptedKey", "blockSize")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(16);
    let ek_bits = xattr(ek, "encryptedKey", "keyBits")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(256);
    let spin = xattr(ek, "encryptedKey", "spinCount")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(100_000);
    let enc_ver_input =
        b64_decode(&xattr(ek, "encryptedKey", "encryptedVerifierHashInput").ok_or("照合がありません")?)?;
    let enc_ver_value =
        b64_decode(&xattr(ek, "encryptedKey", "encryptedVerifierHashValue").ok_or("照合がありません")?)?;
    let enc_key_value =
        b64_decode(&xattr(ek, "encryptedKey", "encryptedKeyValue").ok_or("鍵がありません")?)?;

    // 合言葉の照合
    let spun = agile_spin(ek_hash, &ek_salt, password, spin);
    let iv = agile_iv(ek_hash, &ek_salt, None, ek_block);
    let mut vin = enc_ver_input.clone();
    cbc_dec(&Ecb::new(&agile_block_key(ek_hash, &spun, &BK_VER_INPUT, ek_bits / 8))?, &iv, &mut vin);
    vin.truncate(ek_salt.len());
    let mut vval = enc_ver_value.clone();
    cbc_dec(&Ecb::new(&agile_block_key(ek_hash, &spun, &BK_VER_VALUE, ek_bits / 8))?, &iv, &mut vval);
    let want = ek_hash.digest(&[&vin]);
    if vval[..want.len().min(vval.len())] != want[..want.len().min(vval.len())] {
        return Err("パスワードが違います".into());
    }
    // 中身の鍵(intermediate key)
    let mut ikey = enc_key_value.clone();
    cbc_dec(&Ecb::new(&agile_block_key(ek_hash, &spun, &BK_KEY_VALUE, ek_bits / 8))?, &iv, &mut ikey);
    ikey.truncate(kd_bits / 8);
    let ecb = Ecb::new(&ikey)?;

    // 完全性(HMAC)。記述子にあれば照合し、合わなければ壊れていると言う
    if let (Some(ehk), Some(ehv)) = (
        xattr(xml, "dataIntegrity", "encryptedHmacKey"),
        xattr(xml, "dataIntegrity", "encryptedHmacValue"),
    ) {
        let mut hk = b64_decode(&ehk)?;
        cbc_dec(&ecb, &agile_iv(kd_hash, &kd_salt, Some(&BK_HMAC_KEY), kd_block), &mut hk);
        let hash_len = kd_hash.digest(&[b""]).len();
        hk.truncate(hash_len);
        let mut hv = b64_decode(&ehv)?;
        cbc_dec(&ecb, &agile_iv(kd_hash, &kd_salt, Some(&BK_HMAC_VALUE), kd_block), &mut hv);
        hv.truncate(hash_len);
        if hmac(kd_hash, &hk, package) != hv {
            return Err("完全性の照合が合いません(壊れているか、改ざんされています)".into());
        }
    }

    // 中身。先頭 8 バイトが元の大きさ、続きが 4096 バイトずつの区分
    if package.len() < 8 {
        return Err("EncryptedPackage が短すぎます".into());
    }
    let total = u64::from_le_bytes(package[..8].try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(total);
    for (i, seg) in package[8..].chunks(4096).enumerate() {
        let iv = agile_iv(kd_hash, &kd_salt, Some(&(i as u32).to_le_bytes()), kd_block);
        let mut d = seg.to_vec();
        let whole = d.len() - d.len() % 16;
        cbc_dec(&ecb, &iv, &mut d[..whole]);
        out.extend_from_slice(&d);
    }
    if total > out.len() {
        return Err("大きさが合いません(壊れているようです)".into());
    }
    out.truncate(total);
    Ok(out)
}

/// Agile 方式で包む(SHA-512・AES-256-CBC・spin 10万・HMAC つき)。
/// Excel/Word 2013+ が既定で書くのと同じ形。
pub fn encrypt_agile(plain: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let h = Hasher::S512;
    let (kd_salt, ek_salt) = (urandom(16), urandom(16));
    let ikey = urandom(32); // 中身の鍵(乱数)
    let ecb = Ecb::new(&ikey)?;
    let spin = 100_000u32;
    let spun = agile_spin(h, &ek_salt, password, spin);
    let iv = agile_iv(h, &ek_salt, None, 16);

    // 照合と鍵の包み
    let verifier = urandom(16);
    let mut vin = verifier.clone();
    cbc_enc(&Ecb::new(&agile_block_key(h, &spun, &BK_VER_INPUT, 32))?, &iv, &mut vin);
    let mut vval = h.digest(&[&verifier]);
    cbc_enc(&Ecb::new(&agile_block_key(h, &spun, &BK_VER_VALUE, 32))?, &iv, &mut vval);
    let mut ekv = ikey.clone();
    cbc_enc(&Ecb::new(&agile_block_key(h, &spun, &BK_KEY_VALUE, 32))?, &iv, &mut ekv);

    // 中身(4096 バイトずつ CBC。詰め物は 16 の倍数まで)
    let mut package = Vec::new();
    package.extend_from_slice(&(plain.len() as u64).to_le_bytes());
    for (i, seg) in plain.chunks(4096).enumerate() {
        let mut d = seg.to_vec();
        let pad = (16 - d.len() % 16) % 16;
        d.extend(std::iter::repeat(0u8).take(pad));
        cbc_enc(&ecb, &agile_iv(h, &kd_salt, Some(&(i as u32).to_le_bytes()), 16), &mut d);
        package.extend_from_slice(&d);
    }

    // 完全性(HMAC-SHA512 を中身の流れ全体に)
    let hmac_key = urandom(64);
    let hv = hmac(h, &hmac_key, &package);
    let mut ehk = hmac_key.clone();
    cbc_enc(&ecb, &agile_iv(h, &kd_salt, Some(&BK_HMAC_KEY), 16), &mut ehk);
    let mut ehv = hv.clone();
    cbc_enc(&ecb, &agile_iv(h, &kd_salt, Some(&BK_HMAC_VALUE), 16), &mut ehv);

    let xml = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<encryption xmlns="http://schemas.microsoft.com/office/2006/encryption" xmlns:p="http://schemas.microsoft.com/office/2006/keyEncryptor/password" xmlns:c="http://schemas.microsoft.com/office/2006/keyEncryptor/certificate">"#,
            r#"<keyData saltSize="16" blockSize="16" keyBits="256" hashSize="64" cipherAlgorithm="AES" cipherChaining="ChainingModeCBC" hashAlgorithm="SHA512" saltValue="{kds}"/>"#,
            r#"<dataIntegrity encryptedHmacKey="{ehk}" encryptedHmacValue="{ehv}"/>"#,
            r#"<keyEncryptors><keyEncryptor uri="http://schemas.microsoft.com/office/2006/keyEncryptor/password">"#,
            r#"<p:encryptedKey spinCount="{spin}" saltSize="16" blockSize="16" keyBits="256" hashSize="64" cipherAlgorithm="AES" cipherChaining="ChainingModeCBC" hashAlgorithm="SHA512" saltValue="{eks}" encryptedVerifierHashInput="{vin}" encryptedVerifierHashValue="{vval}" encryptedKeyValue="{ekv}"/>"#,
            r#"</keyEncryptor></keyEncryptors></encryption>"#
        ),
        kds = b64_encode(&kd_salt),
        ehk = b64_encode(&ehk),
        ehv = b64_encode(&ehv),
        spin = spin,
        eks = b64_encode(&ek_salt),
        vin = b64_encode(&vin),
        vval = b64_encode(&vval),
        ekv = b64_encode(&ekv),
    );
    let mut info = Vec::new();
    info.extend_from_slice(&4u16.to_le_bytes()); // major
    info.extend_from_slice(&4u16.to_le_bytes()); // minor
    info.extend_from_slice(&0x40u32.to_le_bytes()); // fAgile
    info.extend_from_slice(xml.as_bytes());

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
    fn agileで包んで解ける() {
        // 4096 の区分をまたぐ大きさで(区分ごとの IV の検査になる)
        let mut plain = b"PK\x03\x04 agile ".to_vec();
        while plain.len() < 10_000 {
            plain.extend_from_slice(b"0123456789abcdef");
        }
        let enc = encrypt_agile(&plain, "農口の合言葉").expect("包めない");
        assert!(is_encrypted(&enc), "暗号化と分からない");
        let back = decrypt(&enc, "農口の合言葉").expect("解けない");
        assert_eq!(back, plain, "解いた中身が違う");
        let e = decrypt(&enc, "まちがい").unwrap_err();
        assert!(e.contains("パスワード"), "違う言い方: {e}");
        // 改ざん検知(HMAC): 中身の暗号文を崩すと完全性で止まる。
        // CFB の中の EncryptedPackage ストリームを開いて崩す(スラックに
        // 当てないため — ファイル末尾を崩すとセクタの余白に逃げる)
        let mut bad = enc.clone();
        {
            let mut c = cfb::CompoundFile::open(std::io::Cursor::new(&mut bad)).unwrap();
            let mut pkg = Vec::new();
            c.open_stream("/EncryptedPackage")
                .unwrap()
                .read_to_end(&mut pkg)
                .unwrap();
            pkg[100] ^= 0xFF; // 8バイトの大きさより後 = 暗号文の中
            c.open_stream("/EncryptedPackage")
                .unwrap()
                .write_all(&pkg)
                .unwrap();
            c.flush().unwrap();
        }
        let e = decrypt(&bad, "農口の合言葉").unwrap_err();
        assert!(e.contains("完全性"), "改ざんが素通り: {e}");
    }

    #[test]
    fn base64が往復する() {
        for n in 0..40usize {
            let data: Vec<u8> = (0..n as u8).map(|i| i.wrapping_mul(37)).collect();
            let s = b64_encode(&data);
            assert_eq!(b64_decode(&s).unwrap(), data, "{n} バイトで割れた");
        }
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
