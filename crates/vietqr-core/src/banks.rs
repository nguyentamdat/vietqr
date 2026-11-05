//! Offline subset of @viet-qr/core 1.4.0 (MIT, Zeforc), banks export.
//! Source: https://www.npmjs.com/package/@viet-qr/core/v/1.4.0
//! Retrieved 2026-11-05 via npm registry; names/codes are source snapshots, not live status.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bank {
    pub bin: &'static str,
    pub code: &'static str,
    pub name: &'static str,
}

pub const BANKS: &[Bank] = &[
    Bank {
        bin: "970415",
        code: "ICB",
        name: "VietinBank",
    },
    Bank {
        bin: "970436",
        code: "VCB",
        name: "Vietcombank",
    },
    Bank {
        bin: "970418",
        code: "BIDV",
        name: "BIDV",
    },
    Bank {
        bin: "970405",
        code: "VBA",
        name: "Agribank",
    },
    Bank {
        bin: "970448",
        code: "OCB",
        name: "OCB",
    },
    Bank {
        bin: "970422",
        code: "MB",
        name: "MBBank",
    },
    Bank {
        bin: "970407",
        code: "TCB",
        name: "Techcombank",
    },
    Bank {
        bin: "970416",
        code: "ACB",
        name: "ACB",
    },
    Bank {
        bin: "970432",
        code: "VPB",
        name: "VPBank",
    },
    Bank {
        bin: "970423",
        code: "TPB",
        name: "TPBank",
    },
    Bank {
        bin: "970403",
        code: "STB",
        name: "Sacombank",
    },
    Bank {
        bin: "970437",
        code: "HDB",
        name: "HDBank",
    },
    Bank {
        bin: "970454",
        code: "VCCB",
        name: "VietCapitalBank",
    },
    Bank {
        bin: "970429",
        code: "SCB",
        name: "SCB",
    },
    Bank {
        bin: "970441",
        code: "VIB",
        name: "VIB",
    },
    Bank {
        bin: "970443",
        code: "SHB",
        name: "SHB",
    },
    Bank {
        bin: "970431",
        code: "EIB",
        name: "Eximbank",
    },
    Bank {
        bin: "970426",
        code: "MSB",
        name: "MSB",
    },
];

pub fn by_bin(bin: &str) -> Option<&'static Bank> {
    BANKS.iter().find(|bank| bank.bin == bin)
}

pub fn by_code(code: &str) -> Option<&'static Bank> {
    BANKS
        .iter()
        .find(|bank| bank.code.eq_ignore_ascii_case(code))
}
