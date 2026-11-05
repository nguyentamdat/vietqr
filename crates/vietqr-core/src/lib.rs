#![doc = include_str!("../../../README.md")]
pub mod banks;
pub mod tlv;

use std::fmt;
use tlv::{Field, decode, encode};

pub const MAX_AMOUNT: u64 = 9_999_999_999_999;
pub const MAX_MEMO_LEN: usize = 25;
pub const MAX_NAME_LEN: usize = 25;
pub const MAX_ACCOUNT_LEN: usize = 50;
pub const MAX_PAYLOAD_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidTag(u8),
    InvalidLength { tag: u8 },
    MalformedTlv { offset: usize },
    DuplicateTag(u8),
    MissingTag(u8),
    InvalidValue { tag: u8 },
    InvalidBin,
    InvalidAccount,
    InvalidAmount,
    InvalidMemo,
    InvalidName,
    PayloadTooLong,
    InvalidCrc,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VietQR: {self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Account,
    Card,
}
impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Account => "QRIBFTTA",
            Self::Card => "QRIBFTTC",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Transfer<'a> {
    pub bank_bin: &'a str,
    pub account: &'a str,
    pub service: Service,
    pub amount: Option<u64>,
    pub name: Option<&'a str>,
    pub memo: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed<'a> {
    pub bank_bin: &'a str,
    pub account: &'a str,
    pub service: Service,
    pub amount: Option<u64>,
    pub name: Option<&'a str>,
    pub memo: Option<&'a str>,
}

/// CRC16-CCITT-FALSE: poly 0x1021, init 0xFFFF, no reflection or final XOR.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Removes Vietnamese accents in both precomposed and decomposed input.
/// Other characters are preserved so validation never silently drops unsupported text.
pub fn strip_vietnamese_diacritics(input: &str) -> String {
    input
        .chars()
        .filter_map(|c| {
            let mapped = match c {
                'à' | 'á' | 'ạ' | 'ả' | 'ã' | 'â' | 'ầ' | 'ấ' | 'ậ' | 'ẩ' | 'ẫ' | 'ă' | 'ằ'
                | 'ắ' | 'ặ' | 'ẳ' | 'ẵ' => 'a',
                'À' | 'Á' | 'Ạ' | 'Ả' | 'Ã' | 'Â' | 'Ầ' | 'Ấ' | 'Ậ' | 'Ẩ' | 'Ẫ' | 'Ă' | 'Ằ'
                | 'Ắ' | 'Ặ' | 'Ẳ' | 'Ẵ' => 'A',
                'è' | 'é' | 'ẹ' | 'ẻ' | 'ẽ' | 'ê' | 'ề' | 'ế' | 'ệ' | 'ể' | 'ễ' => {
                    'e'
                }
                'È' | 'É' | 'Ẹ' | 'Ẻ' | 'Ẽ' | 'Ê' | 'Ề' | 'Ế' | 'Ệ' | 'Ể' | 'Ễ' => {
                    'E'
                }
                'ì' | 'í' | 'ị' | 'ỉ' | 'ĩ' => 'i',
                'Ì' | 'Í' | 'Ị' | 'Ỉ' | 'Ĩ' => 'I',
                'ò' | 'ó' | 'ọ' | 'ỏ' | 'õ' | 'ô' | 'ồ' | 'ố' | 'ộ' | 'ổ' | 'ỗ' | 'ơ' | 'ờ'
                | 'ớ' | 'ợ' | 'ở' | 'ỡ' => 'o',
                'Ò' | 'Ó' | 'Ọ' | 'Ỏ' | 'Õ' | 'Ô' | 'Ồ' | 'Ố' | 'Ộ' | 'Ổ' | 'Ỗ' | 'Ơ' | 'Ờ'
                | 'Ớ' | 'Ợ' | 'Ở' | 'Ỡ' => 'O',
                'ù' | 'ú' | 'ụ' | 'ủ' | 'ũ' | 'ư' | 'ừ' | 'ứ' | 'ự' | 'ử' | 'ữ' => {
                    'u'
                }
                'Ù' | 'Ú' | 'Ụ' | 'Ủ' | 'Ũ' | 'Ư' | 'Ừ' | 'Ứ' | 'Ự' | 'Ử' | 'Ữ' => {
                    'U'
                }
                'ỳ' | 'ý' | 'ỵ' | 'ỷ' | 'ỹ' => 'y',
                'Ỳ' | 'Ý' | 'Ỵ' | 'Ỷ' | 'Ỹ' => 'Y',
                'đ' => 'd',
                'Đ' => 'D',
                '\u{0300}' | '\u{0301}' | '\u{0303}' | '\u{0309}' | '\u{0323}' | '\u{0302}'
                | '\u{0306}' | '\u{031b}' => return None,
                _ => c,
            };
            Some(mapped)
        })
        .collect()
}

fn check_beneficiary(bin: &str, account: &str, service: Service) -> Result<(), Error> {
    if bin.len() != 6 || !bin.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::InvalidBin);
    }
    if account.is_empty()
        || account.len() > MAX_ACCOUNT_LEN
        || !account.bytes().all(|b| b.is_ascii_alphanumeric())
        || (service == Service::Card
            && (account.len() > 19 || !account.bytes().all(|b| b.is_ascii_digit())))
    {
        return Err(Error::InvalidAccount);
    }
    Ok(())
}
fn check_amount(amount: u64) -> Result<(), Error> {
    if amount == 0 || amount > MAX_AMOUNT {
        Err(Error::InvalidAmount)
    } else {
        Ok(())
    }
}
fn check_text(text: &str, memo: bool) -> Result<(), Error> {
    let max = if memo { MAX_MEMO_LEN } else { MAX_NAME_LEN };
    let valid = !text.is_empty()
        && text.len() <= max
        && text.bytes().all(|b| {
            if memo {
                b.is_ascii_alphanumeric() || b" -.,_@/():".contains(&b)
            } else {
                (b' '..=b'~').contains(&b)
            }
        });
    if valid {
        Ok(())
    } else if memo {
        Err(Error::InvalidMemo)
    } else {
        Err(Error::InvalidName)
    }
}

pub fn generate(input: &Transfer<'_>) -> Result<String, Error> {
    check_beneficiary(input.bank_bin, input.account, input.service)?;
    if let Some(amount) = input.amount {
        check_amount(amount)?;
    }
    let beneficiary = encode(0, input.bank_bin)? + &encode(1, input.account)?;
    let merchant =
        encode(0, "A000000727")? + &encode(1, &beneficiary)? + &encode(2, input.service.as_str())?;
    let mut payload = encode(0, "01")?
        + &encode(1, if input.amount.is_some() { "12" } else { "11" })?
        + &encode(38, &merchant)?
        + &encode(53, "704")?;
    if let Some(amount) = input.amount {
        payload.push_str(&encode(54, &amount.to_string())?);
    }
    payload.push_str(&encode(58, "VN")?);
    if let Some(name) = input.name {
        let name = strip_vietnamese_diacritics(name);
        check_text(&name, false)?;
        payload.push_str(&encode(59, &name)?);
    }
    if let Some(memo) = input.memo {
        let memo = strip_vietnamese_diacritics(memo);
        check_text(&memo, true)?;
        payload.push_str(&encode(62, &encode(8, &memo)?)?);
    }
    payload.push_str("6304");
    let checksum = crc16(payload.as_bytes());
    payload.push_str(&format!("{checksum:04X}"));
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(Error::PayloadTooLong);
    }
    Ok(payload)
}

fn get<'a>(fields: &[Field<'a>], id: u8) -> Option<&'a str> {
    fields
        .iter()
        .find(|field| field.id == id)
        .map(|field| field.value)
}
fn required<'a>(fields: &[Field<'a>], id: u8) -> Result<&'a str, Error> {
    get(fields, id).ok_or(Error::MissingTag(id))
}
fn expect(fields: &[Field<'_>], id: u8, value: &str) -> Result<(), Error> {
    if required(fields, id)? == value {
        Ok(())
    } else {
        Err(Error::InvalidValue { tag: id })
    }
}

/// Validates the supported transfer profile. Unknown tags remain opaque and are ignored.
/// The CRC must be uppercase and the final field; successful parsing is not proof of payment.
pub fn parse(payload: &str) -> Result<Parsed<'_>, Error> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(Error::PayloadTooLong);
    }
    if !payload.is_ascii() {
        return Err(Error::InvalidValue { tag: 0 });
    }
    let fields = decode(payload)?;
    let crc = required(&fields, 63)?;
    if fields.first().map(|field| field.id) != Some(0) {
        return Err(Error::InvalidValue { tag: 0 });
    }
    if fields.last().map(|field| field.id) != Some(63)
        || crc.len() != 4
        || !crc
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
    {
        return Err(Error::InvalidCrc);
    }
    let checksum = format!("{:04X}", crc16(&payload.as_bytes()[..payload.len() - 4]));
    if checksum != crc {
        return Err(Error::InvalidCrc);
    }
    expect(&fields, 0, "01")?;
    expect(&fields, 53, "704")?;
    expect(&fields, 58, "VN")?;
    let merchant = decode(required(&fields, 38)?)?;
    expect(&merchant, 0, "A000000727")?;
    let service = match required(&merchant, 2)? {
        "QRIBFTTA" => Service::Account,
        "QRIBFTTC" => Service::Card,
        _ => return Err(Error::InvalidValue { tag: 2 }),
    };
    let beneficiary = decode(required(&merchant, 1)?)?;
    let bank_bin = required(&beneficiary, 0)?;
    let account = required(&beneficiary, 1)?;
    check_beneficiary(bank_bin, account, service)?;
    let amount = get(&fields, 54)
        .map(|text| {
            if text.len() > 13 || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::InvalidAmount);
            }
            let value = text.parse().map_err(|_| Error::InvalidAmount)?;
            check_amount(value)?;
            Ok(value)
        })
        .transpose()?;
    expect(&fields, 1, if amount.is_some() { "12" } else { "11" })?;
    let name = get(&fields, 59);
    if let Some(name) = name {
        check_text(name, false)?;
    }
    let additional = get(&fields, 62).map(decode).transpose()?;
    let memo = additional.as_ref().and_then(|fields| get(fields, 8));
    if let Some(memo) = memo {
        check_text(memo, true)?;
    }
    Ok(Parsed {
        bank_bin,
        account,
        service,
        amount,
        name,
        memo,
    })
}

pub fn validate(payload: &str) -> Result<(), Error> {
    parse(payload).map(|_| ())
}
