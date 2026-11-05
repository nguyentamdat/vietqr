//! Byte-length TLV codec. Unknown tags are opaque; duplicate IDs are invalid per template.
use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field<'a> {
    pub id: u8,
    pub value: &'a str,
}

pub fn encode(id: u8, value: &str) -> Result<String, Error> {
    if id > 99 {
        return Err(Error::InvalidTag(id));
    }
    if value.is_empty() || value.len() > 99 {
        return Err(Error::InvalidLength { tag: id });
    }
    Ok(format!("{id:02}{:02}{value}", value.len()))
}

pub fn decode(input: &str) -> Result<Vec<Field<'_>>, Error> {
    let mut fields = Vec::new();
    let mut seen = [false; 100];
    let mut offset = 0;
    while offset < input.len() {
        let header = input
            .as_bytes()
            .get(offset..offset + 4)
            .ok_or(Error::MalformedTlv { offset })?;
        if !header.iter().all(u8::is_ascii_digit) {
            return Err(Error::MalformedTlv { offset });
        }
        let id = (header[0] - b'0') * 10 + header[1] - b'0';
        let len = ((header[2] - b'0') * 10 + header[3] - b'0') as usize;
        if len == 0 {
            return Err(Error::InvalidLength { tag: id });
        }
        let end = offset + 4 + len;
        let value = input
            .get(offset + 4..end)
            .ok_or(Error::MalformedTlv { offset })?;
        if seen[id as usize] {
            return Err(Error::DuplicateTag(id));
        }
        seen[id as usize] = true;
        fields.push(Field { id, value });
        offset = end;
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tlv_round_trips_all_ids_and_lengths() {
        for id in 0..=99 {
            for len in 1..=99 {
                let value = "x".repeat(len);
                let encoded = encode(id, &value).unwrap();
                assert_eq!(decode(&encoded).unwrap(), vec![Field { id, value: &value }]);
            }
        }
    }
}
