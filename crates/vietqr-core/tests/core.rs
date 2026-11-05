use vietqr_core::{
    Error, MAX_AMOUNT, MAX_PAYLOAD_LEN, Profile, Service, Transfer, banks, crc16, generate,
    generate_with_profile, parse, strip_vietnamese_diacritics, tlv, validate,
};

fn transfer() -> Transfer<'static> {
    Transfer {
        bank_bin: "970436",
        account: "0000000000",
        service: Service::Account,
        amount: None,
        name: None,
        memo: None,
    }
}
fn seal(body: &str) -> String {
    let body = format!("{body}6304");
    format!("{body}{:04X}", crc16(body.as_bytes()))
}
fn change(payload: &str, id: u8, value: Option<&str>) -> String {
    let mut body = String::new();
    for field in tlv::decode(payload).unwrap() {
        if field.id == 63 {
            continue;
        }
        if field.id == id {
            if let Some(value) = value {
                body.push_str(&tlv::encode(id, value).unwrap());
            }
        } else {
            body.push_str(&tlv::encode(field.id, field.value).unwrap());
        }
    }
    seal(&body)
}

#[test]
fn crc16_known_check_vector() {
    assert_eq!(crc16(b"123456789"), 0x29b1);
    assert_eq!(crc16(b""), 0xffff);
    let payload = generate_with_profile(&transfer(), Profile::Strict).unwrap();
    assert_eq!(
        payload,
        "00020101021138540010A00000072701240006970436011000000000000208QRIBFTTA53037045802VN6304BF8F"
    );
}

#[test]
fn tlv_byte_lengths_and_errors() {
    assert_eq!(tlv::encode(8, "đ").unwrap(), "0802đ");
    assert_eq!(tlv::decode("0802đ").unwrap()[0].value, "đ");
    assert_eq!(tlv::decode("0801đ"), Err(Error::MalformedTlv { offset: 0 }));
    assert_eq!(tlv::encode(100, "a"), Err(Error::InvalidTag(100)));
    assert_eq!(tlv::encode(8, ""), Err(Error::InvalidLength { tag: 8 }));
    assert!(tlv::encode(8, &"a".repeat(99)).is_ok());
    assert_eq!(
        tlv::encode(8, &"a".repeat(100)),
        Err(Error::InvalidLength { tag: 8 })
    );
    assert_eq!(tlv::decode("0001a0001b"), Err(Error::DuplicateTag(0)));
    for bad in [
        "0", "000", "ab01x", "0000", "0099x", "-101a", "10001a", "0001aX",
    ] {
        assert!(tlv::decode(bad).is_err(), "{bad}");
    }
    assert!(tlv::decode("").unwrap().is_empty());
}

#[test]
fn npm_reference_vectors() {
    let fixture = include_str!("fixtures/npm-vectors.tsv");
    let mut count = 0;
    for line in fixture.lines().filter(|line| !line.starts_with('#')) {
        let cols: Vec<_> = line.split('\t').collect();
        assert_eq!(cols.len(), 8);
        let optional = |i: usize| {
            if cols[i].is_empty() {
                None
            } else {
                Some(cols[i])
            }
        };
        let input = Transfer {
            bank_bin: cols[0],
            account: cols[1],
            service: if cols[2] == "card" {
                Service::Card
            } else {
                Service::Account
            },
            amount: optional(3).map(|s| s.parse().unwrap()),
            name: optional(4),
            memo: optional(5),
        };
        let generated = generate(&input).unwrap();
        assert_eq!(generated, cols[6]); // Default profile matches raw npm bytes.
        assert_eq!(
            generate_with_profile(&input, Profile::Strict).unwrap(),
            cols[7]
        );
        let reference = parse(cols[6]).unwrap();
        let parsed = parse(&generated).unwrap();
        assert_eq!(reference.bank_bin, parsed.bank_bin);
        assert_eq!(reference.account, parsed.account);
        assert_eq!(reference.service, parsed.service);
        assert_eq!(reference.amount, parsed.amount);
        assert_eq!(reference.memo, parsed.memo);
        if input.name.is_some() {
            assert_eq!(reference.name, parsed.name);
        }
        count += 1;
    }
    assert_eq!(count, 6);
}

#[test]
fn round_trip_property_style() {
    for bank in banks::BANKS {
        for service in [Service::Account, Service::Card] {
            for amount in [None, Some(1), Some(50000), Some(MAX_AMOUNT)] {
                for length in [1, 10, 19] {
                    let account = "0".repeat(length);
                    let input = Transfer {
                        bank_bin: bank.bin,
                        account: &account,
                        service,
                        amount,
                        name: Some("TEST USER"),
                        memo: Some("Đóng tiền ăn"),
                    };
                    let payload = generate(&input).unwrap();
                    let parsed = parse(&payload).unwrap();
                    assert_eq!(parsed.bank_bin, input.bank_bin);
                    assert_eq!(parsed.account, account);
                    assert_eq!(parsed.service, service);
                    assert_eq!(parsed.amount, amount);
                    assert_eq!(parsed.name, input.name);
                    assert_eq!(parsed.memo, Some("Dong tien an"));
                    validate(&payload).unwrap();
                }
            }
        }
    }
}

#[test]
fn input_boundaries_and_accents() {
    let mut input = transfer();
    for bin in ["", "97043", "9704367", "97043A", "９７０４３６"] {
        input.bank_bin = bin;
        assert_eq!(generate(&input), Err(Error::InvalidBin));
    }
    input = transfer();
    for account in ["", "a b", "123-456", "đ", "\n"] {
        input.account = account;
        assert_eq!(generate(&input), Err(Error::InvalidAccount));
    }
    let long = "A".repeat(50);
    input.account = &long;
    assert!(generate(&input).is_ok());
    let longer = "A".repeat(51);
    input.account = &longer;
    assert_eq!(generate(&input), Err(Error::InvalidAccount));
    input = transfer();
    input.service = Service::Card;
    for account in ["TEST0001", "00000000000000000000"] {
        input.account = account;
        assert_eq!(generate(&input), Err(Error::InvalidAccount));
    }
    input = transfer();
    for amount in [0, MAX_AMOUNT + 1, u64::MAX] {
        input.amount = Some(amount);
        assert_eq!(generate(&input), Err(Error::InvalidAmount));
    }
    input = transfer();
    for memo in ["", "😸", "a\nb", "pay!"] {
        input.memo = Some(memo);
        assert_eq!(generate(&input), Err(Error::InvalidMemo));
    }
    input.memo = Some("AZaz09 -.,_@/():");
    input.name = Some("ĐẶNG THỊ ÁNH");
    let payload = generate(&input).unwrap();
    assert_eq!(parse(&payload).unwrap().name, Some("DANG THI ANH"));
    for name in ["", "é\n"] {
        input.name = Some(name);
        assert_eq!(generate(&input), Err(Error::InvalidName));
    }
    assert_eq!(
        strip_vietnamese_diacritics("Đặng đóng Ă Â Ê Ô Ơ Ư Ý"),
        "Dang dong A A E O O U Y"
    );
    assert_eq!(
        strip_vietnamese_diacritics("Đo\u{0301}ng tie\u{0302}\u{0300}n a\u{0306}n"),
        "Dong tien an"
    );
}
#[test]
fn generation_profiles_text_boundaries_and_defaults() {
    assert_eq!(Profile::default(), Profile::Npm);
    let npm = generate(&transfer()).unwrap();
    let strict = generate_with_profile(&transfer(), Profile::Strict).unwrap();
    assert_eq!(parse(&npm).unwrap().name, Some("NA"));
    assert_eq!(parse(&strict).unwrap().name, None);
    assert!(
        tlv::decode(&npm)
            .unwrap()
            .iter()
            .any(|f| f.id == 60 && f.value == "VN")
    );
    assert!(
        !tlv::decode(&strict)
            .unwrap()
            .iter()
            .any(|f| f.id == 59 || f.id == 60)
    );
    for length in [25, 26, 50, 51] {
        // Multi-byte input is measured only after removing accents.
        let text = "đ".repeat(length);
        let plain = "d".repeat(length);
        for memo in [false, true] {
            let input = Transfer {
                name: (!memo).then_some(text.as_str()),
                memo: memo.then_some(text.as_str()),
                ..transfer()
            };
            let error = if memo {
                Error::InvalidMemo
            } else {
                Error::InvalidName
            };
            for (profile, limit) in [(Profile::Npm, 50), (Profile::Strict, 25)] {
                let result = generate_with_profile(&input, profile);
                if length > limit {
                    assert_eq!(result, Err(error.clone()));
                } else {
                    let payload = result.unwrap();
                    let parsed = parse(&payload).unwrap();
                    assert_eq!(
                        if memo { parsed.memo } else { parsed.name },
                        Some(plain.as_str())
                    );
                }
            }
        }
    }
}

#[test]
fn parser_rejects_invalid_profile_with_valid_crc() {
    let payload = generate(&transfer()).unwrap();
    for (id, value, error) in [
        (0, "02", Error::InvalidValue { tag: 0 }),
        (1, "12", Error::InvalidValue { tag: 1 }),
        (53, "840", Error::InvalidValue { tag: 53 }),
        (58, "US", Error::InvalidValue { tag: 58 }),
    ] {
        assert_eq!(parse(&change(&payload, id, Some(value))), Err(error));
    }
    for id in [0, 1, 38, 53, 58] {
        assert!(parse(&change(&payload, id, None)).is_err());
    }
    let dynamic = generate(&Transfer {
        amount: Some(1),
        ..transfer()
    })
    .unwrap();
    for amount in ["0", "-1", "1.1", "+1", "10000000000000", "A"] {
        assert_eq!(
            parse(&change(&dynamic, 54, Some(amount))),
            Err(Error::InvalidAmount)
        );
    }
    let merchant = tlv::decode(&payload)
        .unwrap()
        .into_iter()
        .find(|f| f.id == 38)
        .unwrap()
        .value;
    let invalid_service = merchant.replace("QRIBFTTA", "QRIBFTXX");
    assert_eq!(
        parse(&change(&payload, 38, Some(&invalid_service))),
        Err(Error::InvalidValue { tag: 2 })
    );
    let invalid_bin = merchant.replace("970436", "97043A");
    assert_eq!(
        parse(&change(&payload, 38, Some(&invalid_bin))),
        Err(Error::InvalidBin)
    );
    let duplicate = format!("{merchant}0010A000000727");
    assert_eq!(
        parse(&change(&payload, 38, Some(&duplicate))),
        Err(Error::DuplicateTag(0))
    );
    // Explicitly build a duplicate beneficiary subfield inside tag 38.01.
    let nested = tlv::encode(0, "970436").unwrap()
        + &tlv::encode(1, "0000000000").unwrap()
        + &tlv::encode(1, "a").unwrap();
    let bad = tlv::encode(0, "A000000727").unwrap()
        + &tlv::encode(1, &nested).unwrap()
        + &tlv::encode(2, "QRIBFTTA").unwrap();
    assert_eq!(
        parse(&change(&payload, 38, Some(&bad))),
        Err(Error::DuplicateTag(1))
    );
    let additional = tlv::encode(8, "test").unwrap() + &tlv::encode(8, "test").unwrap();
    let body = &payload[..payload.len() - 8];
    assert_eq!(
        parse(&seal(&format!(
            "{body}{}",
            tlv::encode(62, &additional).unwrap()
        ))),
        Err(Error::DuplicateTag(8))
    );
}

#[test]
fn parser_crc_unknown_tags_and_length_limit() {
    let payload = generate_with_profile(&transfer(), Profile::Strict).unwrap();
    let mut bad = payload.clone().into_bytes();
    *bad.last_mut().unwrap() = b'0';
    assert_eq!(
        parse(std::str::from_utf8(&bad).unwrap()),
        Err(Error::InvalidCrc)
    );
    let lowercase = format!(
        "{}{}",
        &payload[..payload.len() - 4],
        payload[payload.len() - 4..].to_lowercase()
    );
    assert_eq!(parse(&lowercase), Err(Error::InvalidCrc));
    assert!(parse(&(payload.clone() + "9901x")).is_err());
    let body = &payload[..payload.len() - 8];
    assert_eq!(
        parse(&seal(&format!("{body}5303704"))),
        Err(Error::DuplicateTag(53))
    );
    assert_eq!(
        parse(&seal(&format!("{body}9901x"))).unwrap().account,
        "0000000000"
    );
    assert!(parse(&seal(&format!("{body}9902x"))).is_err());
    let mut body = body.to_owned();
    for id in 80..84 {
        body += &tlv::encode(id, &"X".repeat(99)).unwrap();
    }
    let padding = MAX_PAYLOAD_LEN - seal(&body).len() - 4;
    let exact = seal(&format!(
        "{body}{}",
        tlv::encode(84, &"X".repeat(padding)).unwrap()
    ));
    assert_eq!(exact.len(), MAX_PAYLOAD_LEN);
    validate(&exact).unwrap();
    let over = seal(&format!(
        "{body}{}",
        tlv::encode(84, &"X".repeat(padding + 1)).unwrap()
    ));
    assert_eq!(over.len(), MAX_PAYLOAD_LEN + 1);
    assert_eq!(parse(&over), Err(Error::PayloadTooLong));
    assert!(parse("").is_err());
}

#[test]
fn parser_checks_text_and_nested_structure() {
    let input = Transfer {
        name: Some("TEST USER"),
        memo: Some("test"),
        ..transfer()
    };
    let payload = generate(&input).unwrap();
    for name in ["bad\nname", &"x".repeat(51)] {
        assert_eq!(
            parse(&change(&payload, 59, Some(name))),
            Err(Error::InvalidName)
        );
    }
    for memo in ["bad!", &"x".repeat(51)] {
        let additional = tlv::encode(8, memo).unwrap();
        assert_eq!(
            parse(&change(&payload, 62, Some(&additional))),
            Err(Error::InvalidMemo)
        );
    }
    for id in [38, 62] {
        assert!(parse(&change(&payload, id, Some("0005x"))).is_err());
    }
    let additional = tlv::encode(8, "test").unwrap() + &tlv::encode(99, "opaque").unwrap();
    assert_eq!(
        parse(&change(&payload, 62, Some(&additional)))
            .unwrap()
            .memo,
        Some("test")
    );
}

#[test]
fn bank_lookup_is_offline_and_unique() {
    assert_eq!(banks::by_code("vcb").unwrap().bin, "970436");
    assert_eq!(banks::by_bin("970422").unwrap().code, "MB");
    assert!(banks::by_code("unknown").is_none());
    assert!(banks::by_bin("000000").is_none());
    for (i, bank) in banks::BANKS.iter().enumerate() {
        for other in &banks::BANKS[i + 1..] {
            assert_ne!(bank.bin, other.bin);
            assert_ne!(bank.code, other.code);
        }
    }
}
