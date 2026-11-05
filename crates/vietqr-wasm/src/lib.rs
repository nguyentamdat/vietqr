//! String-only JSON boundary for browsers; core types remain independent of serde/WASM.

use serde::Deserialize;
use serde_json::json;
use vietqr_core::{Profile, Service, Transfer};
use wasm_bindgen::prelude::*;

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum JsonService {
    Account,
    Card,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum JsonProfile {
    #[default]
    Npm,
    Strict,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    bank_bin: String,
    account: String,
    service: JsonService,
    amount: Option<u64>,
    name: Option<String>,
    memo: Option<String>,
    #[serde(default)]
    profile: JsonProfile,
}

/// Takes JSON options and returns a payload; invalid input throws a string in JS.
#[wasm_bindgen]
pub fn generate(opts: &str) -> Result<String, String> {
    let opts: Options = serde_json::from_str(opts).map_err(|error| error.to_string())?;
    let input = Transfer {
        bank_bin: &opts.bank_bin,
        account: &opts.account,
        service: match opts.service {
            JsonService::Account => Service::Account,
            JsonService::Card => Service::Card,
        },
        amount: opts.amount,
        name: opts.name.as_deref(),
        memo: opts.memo.as_deref(),
    };
    vietqr_core::generate_with_profile(
        &input,
        match opts.profile {
            JsonProfile::Npm => Profile::Npm,
            JsonProfile::Strict => Profile::Strict,
        },
    )
    .map_err(|error| error.to_string())
}

/// Validates a payload and returns JSON fields; absent optional fields are null.
#[wasm_bindgen]
pub fn parse(payload: &str) -> Result<String, String> {
    let parsed = vietqr_core::parse(payload).map_err(|error| error.to_string())?;
    Ok(json!({
        "bank_bin": parsed.bank_bin,
        "account": parsed.account,
        "service": match parsed.service {
            Service::Account => "account",
            Service::Card => "card",
        },
        "amount": parsed.amount,
        "name": parsed.name,
        "memo": parsed.memo,
    })
    .to_string())
}

/// Renders any string as QR SVG (M, quiet zone 4), without VietQR validation.
#[wasm_bindgen]
pub fn render_svg(payload: &str) -> Result<String, String> {
    vietqr_render::render_svg(payload).map_err(|error| error.to_string())
}

/// Returns the offline bank snapshot as a JSON array of {bin, code, name}.
#[wasm_bindgen]
pub fn banks() -> String {
    let banks: Vec<_> = vietqr_core::banks::BANKS
        .iter()
        .map(|bank| json!({"bin": bank.bin, "code": bank.code, "name": bank.name}))
        .collect();
    json!(banks).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn opts() -> Value {
        json!({"bank_bin": "970436", "account": "0000000000", "service": "account"})
    }

    #[test]
    fn generate_parse_json_round_trip_and_profiles() {
        for profile in ["npm", "strict"] {
            let mut input = opts();
            input["profile"] = json!(profile);
            input["memo"] = json!("Đóng tiền ăn");
            input["amount"] = json!(50_000);
            let payload = generate(&input.to_string()).unwrap();
            let parsed: Value = serde_json::from_str(&parse(&payload).unwrap()).unwrap();
            assert_eq!(parsed["account"], "0000000000");
            assert_eq!(parsed["bank_bin"], "970436");
            assert_eq!(parsed["service"], "account");
            assert_eq!(parsed["amount"], 50_000);
            assert_eq!(parsed["memo"], "Dong tien an");
            assert_eq!(
                parsed["name"],
                if profile == "npm" {
                    json!("NA")
                } else {
                    Value::Null
                }
            );
            assert!(render_svg(&payload).unwrap().starts_with("<svg"));
        }
        assert_eq!(
            generate(&opts().to_string()).unwrap(),
            vietqr_core::generate(&Transfer {
                bank_bin: "970436",
                account: "0000000000",
                service: Service::Account,
                amount: None,
                name: None,
                memo: None,
            })
            .unwrap()
        );
        let mut card = opts();
        card["service"] = json!("card");
        let payload = generate(&card.to_string()).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&parse(&payload).unwrap()).unwrap()["service"],
            "card"
        );
    }

    #[test]
    fn json_boundary_rejects_invalid_input() {
        for input in ["{", "null", "{}"] {
            assert!(generate(input).is_err());
        }
        for (key, bad) in [
            ("account", json!(123)),
            ("service", json!("other")),
            ("amount", json!(-1)),
            ("amount", json!(1.5)),
            ("amount", json!(10_000_000_000_000u64)),
            ("amount", json!(0)),
            ("profile", json!("other")),
            ("unknown", json!(true)),
            ("memo", json!("x".repeat(51))),
        ] {
            let mut input = opts();
            input[key] = bad;
            assert!(generate(&input.to_string()).is_err(), "{key}");
        }
        let mut input = opts();
        input["memo"] = json!("x".repeat(50));
        assert!(generate(&input.to_string()).is_ok());
        input["profile"] = json!("strict");
        assert!(generate(&input.to_string()).is_err());
        assert!(parse("bad CRC").is_err());
        assert!(render_svg(&"x".repeat(10_000)).is_err());
    }

    #[test]
    fn banks_json_matches_core_snapshot() {
        let banks: Vec<Value> = serde_json::from_str(&banks()).unwrap();
        assert_eq!(banks.len(), vietqr_core::banks::BANKS.len());
        for (bank, reference) in banks.iter().zip(vietqr_core::banks::BANKS) {
            assert_eq!(bank["bin"], reference.bin);
            assert_eq!(bank["code"], reference.code);
            assert_eq!(bank["name"], reference.name);
        }
    }
}
